// SPDX-License-Identifier: AGPL-3.0-or-later
//! Migration as a gated operation: importing a source artifact through a binding is a
//! change to a repository, so it obeys the same four rules as any other migration.
//!
//! 1. The source artifact is retained byte-for-byte and content-addressed BEFORE import is
//!    even attempted, so a migration can never destroy the thing it migrated.
//! 2. No mapping is applied silently: a blocking loss (Lossy or Unmappable) refuses the
//!    import unless the request names that exact loss as accepted, and the acceptance is
//!    recorded in the audit trail.
//! 3. The one half of fidelity the engine can see is measured, not trusted: the binding's
//!    OWN OKF->XMI->OKF round trip is diffed by the engine, which must agree nothing was lost
//!    on that journey. The native XMI->OKF read - the actual migration - is NOT independently
//!    measured; it rests on the binding's self-reported loss report.
//! 4. Provenance is durable and atomic: the commit and its link to the source artifact are
//!    written in one transaction, so a committed import can never be unlinked from its source.

use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use agent::losses::entry_identity;

use crate::api::{
    commit_core, commit_json, load_model, map_store_error, record_refusal, resolve_author,
    validate_name, verify_actor, ApiState, CommitCore, CommitFailure,
};
use crate::audit::{IMPORT_ACCEPT, IMPORT_REFUSED, PROPOSAL_ACCEPT};
use crate::auth::{Identity, Permission};
use crate::binding_registry;
use crate::error::{ApiError, BodyTooLarge};
use crate::store::{now_seconds, AuditEntry, Commit, ImportProvenance, ProposalAcceptance, Store};

#[derive(Deserialize)]
pub struct ImportRequest {
    /// The binding selector, e.g. `sysml-v1-xmi@2.4`.
    pub binding: String,
    pub branch: String,
    #[serde(default)]
    pub author: String,
    pub message: String,
    /// The source artifact, base64-encoded or as a raw UTF-8 body.
    pub artifact: String,
    /// The blocking loss subjects the request accepts by name.
    #[serde(default, rename = "acceptLosses")]
    pub accept_losses: Vec<String>,
    /// Who is importing; supplying it lets a lease holder proceed, omitting it means any
    /// live lease on a changed element refuses the commit.
    pub holder: Option<String>,
}

/// Split a binding selector like `sysml-v1-xmi@2.4` into its id and version. A missing or
/// empty id or version is a request problem, reported as 400.
pub fn parse_binding(selector: &str) -> Result<(String, String), ApiError> {
    match selector.rsplit_once('@') {
        Some((id, version)) if !id.is_empty() && !version.is_empty() => {
            Ok((id.to_string(), version.to_string()))
        }
        _ => Err(ApiError::bad_request(format!(
            "binding must be of the form id@version, got {}",
            selector
        ))),
    }
}

/// Decode the artifact field: base64 when it decodes, the raw UTF-8 bytes otherwise. An XMI
/// document is text, so a raw body is the common case; base64 is accepted for binary source
/// formats a later binding may read.
pub fn decode_artifact(artifact: &str) -> Vec<u8> {
    use base64::Engine as _;
    match base64::engine::general_purpose::STANDARD.decode(artifact) {
        Ok(bytes) => bytes,
        Err(_) => artifact.as_bytes().to_vec(),
    }
}

/// Serialize a `Serialize` value to a JSON string, mapping a serialization failure to a
/// generic internal error rather than leaking the type's internals.
fn to_json_string<T: serde::Serialize>(value: &T) -> Result<String, ApiError> {
    serde_json::to_string(value).map_err(|e| {
        eprintln!("could not serialize import record: {}", e);
        ApiError::internal("the import record could not be prepared")
    })
}

/// What an import attempt produced, once permission, scope, the author and the branch name
/// were resolved upstream. A refusal because blocking losses were not accepted is a RESULT
/// here, not an error: it carries the unaccepted losses so the caller can render them and
/// ask for a decision. Everything that can go wrong is an ApiError.
pub enum ImportOutcome {
    Committed {
        commit: Box<Commit>,
        artifact_hash: String,
        binding_id: String,
        binding_version: String,
        accepted_losses: Vec<String>,
        loss_report: binding::LossReport,
        fidelity: binding::FidelityOutcome,
    },
    Blocking {
        artifact_hash: String,
        binding_id: String,
        binding_version: String,
        unaccepted: Vec<binding::Mapping>,
        loss_report: binding::LossReport,
        fidelity: binding::FidelityOutcome,
    },
}

/// Everything the import core needs that the caller resolved upstream. artifact is the
/// decoded source bytes; author is already resolved against the identity; actor and
/// mechanism are the verified subject and how it authenticated; accept_losses names the
/// blocking losses the request accepts by subject; acceptance is Some when this import is a
/// human accepting an agent's proposal, and names that proposal, its agent and the human.
pub struct ImportCore<'a> {
    pub binding: &'a str,
    pub branch: &'a str,
    pub author: &'a str,
    pub message: &'a str,
    pub artifact: &'a [u8],
    pub accept_losses: &'a [String],
    pub holder: Option<&'a str>,
    pub actor: &'a str,
    pub mechanism: &'a str,
    pub authorizer: &'a str,
    pub acceptance: Option<ProposalAcceptance>,
}

/// Whether an acceptance name, as an ENTRY IDENTITY, names this loss mapping. The entry
/// identity ("subject [verdict]", from `agent::losses::entry_identity`) is the ONE key an
/// agent's proposal and the workbench form carry, and it names exactly one entry even when two
/// entries share a subject. A raw subject is NOT an entry identity: see [`resolve_acceptances`]
/// for how a legacy raw subject is reconciled against the whole blocking set.
pub fn acceptance_matches(mapping: &binding::Mapping, accepted: &str) -> bool {
    accepted == entry_identity(mapping)
}

/// Resolve the acceptance names against the blocking entries, returning the entry identities
/// actually accepted, in the order they were named.
///
/// An acceptance name is either an ENTRY IDENTITY ("subject [verdict]") or a legacy raw
/// subject. An entry identity names exactly one entry. A raw subject names every blocking
/// entry that shares it: when it matches exactly one it is accepted for backwards
/// compatibility with callers that still send raw subjects; when it matches more than one the
/// request is REFUSED (400) rather than silently accepting both, because two entries sharing a
/// subject are two decisions and the caller must name each by its entry identity. A name that
/// matches nothing names a non-blocking loss and is ignored.
pub fn resolve_acceptances(
    blocking: &[&binding::Mapping],
    accept_losses: &[String],
) -> Result<Vec<String>, ApiError> {
    let mut accepted: Vec<String> = Vec::new();
    for name in accept_losses {
        if let Some(mapping) = blocking
            .iter()
            .copied()
            .find(|m| entry_identity(m) == *name)
        {
            let identity = entry_identity(mapping);
            if !accepted.contains(&identity) {
                accepted.push(identity);
            }
            continue;
        }
        let matches: Vec<&binding::Mapping> = blocking
            .iter()
            .copied()
            .filter(|m| m.subject == *name)
            .collect();
        match matches.len() {
            0 => {}
            1 => {
                let identity = entry_identity(matches[0]);
                if !accepted.contains(&identity) {
                    accepted.push(identity);
                }
            }
            n => {
                return Err(ApiError::bad_request(format!(
                    "acceptLosses entry {:?} matches {} blocking entries; name each loss \
                     by its entry identity (subject [verdict])",
                    name, n
                )));
            }
        }
    }
    Ok(accepted)
}

/// The ONE implementation of an import: retain the artifact byte-for-byte, read it through
/// the binding, measure the binding's OKF->XMI->OKF round trip with the engine's diff, record
/// the report, refuse every blocking loss that is not accepted by name, and commit the
/// imported model through the shared commit core with its provenance written atomically. Both
/// import_artifact and the workbench import page call this, so they cannot diverge on the
/// sequence, the acceptance rule or the audit entries.
pub fn import_core(
    store: &dyn Store,
    project: &str,
    input: &ImportCore<'_>,
) -> Result<ImportOutcome, ApiError> {
    if store.project(project).map_err(map_store_error)?.is_none() {
        return Err(ApiError::not_found(format!("project {}", project)));
    }

    // A binding that does not exist is a request problem, not a server fault.
    let (binding_id, binding_version) = parse_binding(input.binding)?;
    let binding = binding_registry::resolve(&binding_id, &binding_version)
        .ok_or_else(|| ApiError::bad_request(format!("unknown binding {}", input.binding)))?;

    // Rule 1: retain the source artifact byte-for-byte and content-addressed BEFORE import
    // is even attempted, so nothing that follows can destroy the thing being migrated.
    let artifact_hash = store.put_blob(input.artifact).map_err(map_store_error)?;

    // Read the artifact through the binding. A binding that cannot read it is a clean 422,
    // never a panic; the artifact is already retained, so the refusal costs nothing.
    let (root, loss_report) = binding.import(input.artifact).map_err(|e| {
        ApiError::unprocessable(
            "the binding could not read the artifact",
            vec![e.to_string()],
        )
    })?;

    // Rule 3: the ONE thing the engine can measure is the binding's own OKF->XMI->OKF round
    // trip. The imported document is exported by the binding and imported back, and the
    // engine's own diff of that journey must agree nothing was lost. The native XMI->OKF read
    // that produced this document is the binding's word (the loss report), not something the
    // engine independently measured.
    let import_bytes = serde_json::to_vec(&root).map_err(|e| {
        eprintln!("imported model could not be serialised: {}", e);
        ApiError::internal("the imported model could not be prepared")
    })?;
    let fidelity = binding::round_trip(binding.as_ref(), &import_bytes).map_err(|e| {
        eprintln!("fidelity harness failed: {}", e);
        ApiError::internal("the fidelity harness could not measure the import")
    })?;

    // Record the report and the measurement keyed by the retained artifact, so a refusal
    // still has a retrievable report and the caller can read exactly which losses to accept.
    let loss_report_json = to_json_string(&loss_report)?;
    let fidelity_json = to_json_string(&fidelity.diff)?;
    store
        .record_import(
            project,
            &artifact_hash,
            &binding_id,
            &binding_version,
            &loss_report_json,
            &fidelity_json,
        )
        .map_err(map_store_error)?;

    // A binding whose own round trip is not lossless is refused: the engine disagrees with
    // the binding, and that is exactly the failure the harness exists to catch.
    if !fidelity.diff.equal {
        if let Err(e) = record_refusal(
            store,
            project,
            input.actor,
            input.mechanism,
            input.authorizer,
            IMPORT_REFUSED,
            &artifact_hash,
            "the binding could not round-trip the imported model",
        ) {
            eprintln!("could not record the import refusal: {:?}", e);
        }
        return Err(ApiError::unprocessable(
            "the binding could not round-trip the imported model; the import is refused",
            fidelity.diff.missing_elements.clone(),
        ));
    }

    // Rule 2: every blocking loss must be accepted by name. An acceptance names a loss by
    // its ENTRY IDENTITY ("subject [verdict]", the ONE key the agent's proposals and the
    // workbench form now carry) or, for backwards compatibility, by the binding's raw subject
    // when that subject names exactly one blocking entry. A raw subject shared by two blocking
    // entries is refused here (400) rather than silently accepting both. Anything not accepted
    // refuses the import and is returned so the caller can decide rather than lose it silently.
    let blocking = loss_report.blocking();
    let accepted_identities = resolve_acceptances(&blocking, input.accept_losses)?;
    let unaccepted: Vec<binding::Mapping> = blocking
        .iter()
        .filter(|m| {
            !accepted_identities
                .iter()
                .any(|id| id == &entry_identity(m))
        })
        .map(|m| (**m).clone())
        .collect();
    if !unaccepted.is_empty() {
        // A direct import that leaves blocking losses unaccepted is recorded as a refusal.
        // An ACCEPTANCE that leaves them unaccepted is an incomplete acceptance, not a
        // decision: the proposal stays undecided and the blocking losses are returned so the
        // human can complete the acceptance or refuse the proposal explicitly.
        if input.acceptance.is_none() {
            let subjects: Vec<String> = unaccepted.iter().map(entry_identity).collect();
            if let Err(e) = record_refusal(
                store,
                project,
                input.actor,
                input.mechanism,
                input.authorizer,
                IMPORT_REFUSED,
                &artifact_hash,
                &format!("blocking losses not accepted: {}", subjects.join(", ")),
            ) {
                eprintln!("could not record the import refusal: {:?}", e);
            }
        }
        return Ok(ImportOutcome::Blocking {
            artifact_hash,
            binding_id,
            binding_version,
            unaccepted,
            loss_report,
            fidelity,
        });
    }

    // The acceptance is recorded BEFORE the commit, so a committed import always has its
    // acceptance on the audit trail; the actor is the verified subject, which is the "who"
    // of "who accepted what". `accepted_identities` is the set `resolve_acceptances` produced
    // above: the entry identities the human accepted, never a raw subject two losses could
    // share.
    if !accepted_identities.is_empty() {
        // An acceptance of an agent's proposal records proposal.accept instead of
        // import.accept, naming the proposal and the agent that made it.
        let (action, subject, detail) = match input.acceptance.as_ref() {
            Some(acceptance) => (
                PROPOSAL_ACCEPT,
                acceptance.proposal_id.clone(),
                format!(
                    "proposal {} by {}; accepted items: {}",
                    acceptance.proposal_id,
                    acceptance.agent,
                    accepted_identities.join(", ")
                ),
            ),
            None => (
                IMPORT_ACCEPT,
                artifact_hash.clone(),
                format!(
                    "binding {}@{}; accepted losses: {}",
                    binding_id,
                    binding_version,
                    accepted_identities.join(", ")
                ),
            ),
        };
        store
            .append_audit(&AuditEntry {
                id: 0,
                project: project.to_string(),
                at: now_seconds(),
                actor: input.actor.to_string(),
                mechanism: input.mechanism.to_string(),
                authorizer: input.authorizer.to_string(),
                action: action.to_string(),
                subject,
                detail,
            })
            .map_err(map_store_error)?;
    }

    // I3: the document that was measured IS the document that is committed. The XMI binding
    // emits the empty state machine itself (OKF requires the section even when the model has
    // no state machine), so import_bytes - the bytes the round trip above measured - are the
    // exact bytes committed. Nothing is mutated after measurement.
    //
    // The import commit is a commit like any other: it goes through the shared commit core,
    // so validation, the lock guard and the commit.create audit entry behave exactly as they
    // do for a plain commit, a reset or a merge.
    let now = now_seconds();
    let tip_hash = store
        .branch_tip(project, input.branch)
        .map_err(map_store_error)?;
    let reference: Option<okf::types::OkfRoot> = match tip_hash.as_deref() {
        Some(tip) => Some(load_model(store, project, tip).map_err(map_store_error)?),
        None => None,
    };
    // Rule 4: provenance is durable and atomic. The import provenance rides the SAME commit
    // transaction as the commit row, so a committed import can never be unlinked from its
    // source artifact; if the link cannot be written, the commit rolls back with it.
    let provenance = ImportProvenance {
        artifact_hash: artifact_hash.clone(),
        binding_id: binding_id.clone(),
        binding_version: binding_version.clone(),
        accepted_losses: accepted_identities.clone(),
        acceptance: input.acceptance.clone(),
    };
    let commit = commit_core(
        store,
        &CommitCore {
            project,
            branch: input.branch,
            author: input.author,
            message: input.message,
            actor: input.actor,
            mechanism: input.mechanism,
            authorizer: input.authorizer,
            candidate: &root,
            bytes: &import_bytes,
            import: Some(&provenance),
            acceptance: None,
            holder: input.holder.unwrap_or(""),
            now,
            tip: tip_hash.as_deref(),
            reference: reference.as_ref(),
        },
    )
    .map_err(|failure| match failure {
        CommitFailure::Invalid { errors } => {
            ApiError::unprocessable("the imported model failed validation", errors)
        }
        CommitFailure::Store(error) => map_store_error(error),
    })?;

    Ok(ImportOutcome::Committed {
        commit: Box::new(commit),
        artifact_hash,
        binding_id,
        binding_version,
        accepted_losses: accepted_identities,
        loss_report,
        fidelity,
    })
}

pub async fn import_artifact(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    headers: HeaderMap,
    body: Result<Json<ImportRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, ApiError> {
    // Permission and scope before anything else, exactly as every other write path.
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    // A body the limit refused becomes the SAME useful 413 the streaming endpoint produces -
    // naming the configured limit and (when declared) the received size - rather than axum's
    // bare "length limit exceeded".
    let body = match body {
        Ok(body) => body,
        Err(rejection) => {
            return Ok(json_rejection_to_response(
                rejection,
                state.max_body_bytes,
                &headers,
            ));
        }
    };
    let author = resolve_author(&state.auth, &identity, &body.author)?;
    verify_actor(&state.auth, &identity, body.holder.as_deref())?;
    validate_name("branch name", &body.branch)?;

    // The artifact arrives base64-encoded or as a raw UTF-8 body; decode once, then hand the
    // bytes to the SAME core the workbench page calls, so the two cannot diverge.
    let artifact_bytes = decode_artifact(&body.artifact);
    match import_core(
        state.store.as_ref(),
        &project,
        &ImportCore {
            binding: &body.binding,
            branch: &body.branch,
            author: &author,
            message: &body.message,
            artifact: &artifact_bytes,
            accept_losses: &body.accept_losses,
            holder: body.holder.as_deref(),
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
            authorizer: state.auth.authorizer().unwrap_or(""),
            acceptance: None,
        },
    )? {
        ImportOutcome::Committed {
            commit,
            loss_report,
            fidelity,
            ..
        } => Ok((
            StatusCode::CREATED,
            Json(json!({
                "commit": commit_json(&commit),
                "lossReport": serde_json::to_value(&loss_report).unwrap_or(Value::Null),
                "fidelity": serde_json::to_value(&fidelity.diff).unwrap_or(Value::Null)
            })),
        )
            .into_response()),
        ImportOutcome::Blocking { unaccepted, .. } => {
            let blocking_json: Vec<Value> = unaccepted
                .iter()
                .map(|m| serde_json::to_value(m).unwrap_or(Value::Null))
                .collect();
            Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "the import has blocking losses that must be accepted by name",
                    "blocking": blocking_json
                })),
            )
                .into_response())
        }
    }
}

/// The declared `Content-Length`, if present and numeric.
fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
}

/// A request whose declared `Content-Length` already exceeds the configured limit. The
/// header is checked before a single byte is staged or buffered, so a caller learns the
/// exact size they attempted to send.
fn content_length_over_limit(headers: &HeaderMap, limit: u64) -> Option<BodyTooLarge> {
    let content_length = content_length(headers)?;
    (content_length > limit).then_some(BodyTooLarge {
        limit,
        received: Some(content_length),
    })
}

/// Turn a JSON body rejection into a response. The length-limit case becomes the SAME useful
/// 413 the streaming endpoint produces - configured limit plus, when declared, the received
/// size - while every other rejection (bad syntax, missing content type) keeps its own
/// response.
fn json_rejection_to_response(
    rejection: axum::extract::rejection::JsonRejection,
    limit: u64,
    headers: &HeaderMap,
) -> Response {
    use axum::extract::rejection::{BytesRejection, FailedToBufferBody, JsonRejection};
    match rejection {
        JsonRejection::BytesRejection(BytesRejection::FailedToBufferBody(
            FailedToBufferBody::LengthLimitError(_),
        )) => BodyTooLarge {
            limit,
            received: content_length(headers),
        }
        .into_response(),
        other => other.into_response(),
    }
}

/// Map a multipart parsing failure to an API error, keeping the 413 status for a body that
/// exceeded the limit rather than folding it into a generic 400.
fn multipart_error(error: &axum::extract::multipart::MultipartError) -> ApiError {
    ApiError {
        status: error.status(),
        message: format!("could not read the import upload: {}", error.body_text()),
    }
}

/// POST /projects/:project/import/stream - the STREAMING artifact path.
///
/// The artifact arrives as the `artifact` FILE field of a `multipart/form-data` body and is
/// STREAMED into the content-addressed blob store: the bytes are staged to a file in bounded
/// chunks while the limit is enforced, then moved into the blob store under their sha256 by
/// [`crate::store::Store::put_blob_file`] BEFORE import is attempted. Peak memory never
/// scales with the artifact size on this path (the XMI is read back into memory only for the
/// binding to parse, which a tree parser requires). The small metadata fields - `binding`,
/// `branch`, `message`, `author`, `holder`, repeated `acceptLosses` - ride the same form.
///
/// This is the same import as the JSON endpoint - same permission, scope, author, actor and
/// acceptance decisions, same [`import_core`] - but it never buffers the artifact as a
/// base64 JSON string, which is what lets a 36 MB (or 300 MB) vendor export arrive without
/// first ballooning through a JSON decoder.
pub async fn import_artifact_stream(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    use std::io::Write as _;

    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }

    // Refuse a declared oversize body up front with the exact size, before staging anything.
    if let Some(too_large) = content_length_over_limit(&headers, state.max_body_bytes) {
        return Ok(too_large.into_response());
    }

    let mut binding = String::new();
    let mut branch = String::new();
    let mut message = String::new();
    let mut author = String::new();
    let mut holder = String::new();
    let mut accept_losses: Vec<String> = Vec::new();

    // The artifact is staged to a file, not a buffer: a content address is only known after
    // the last byte, so the bytes must land somewhere durable before they can be placed
    // under their hash. The hash itself is computed incrementally inside the store.
    let mut staged = tempfile::NamedTempFile::new()
        .map_err(|e| ApiError::internal(format!("could not stage the artifact: {}", e)))?;
    let mut received: u64 = 0;
    let mut got_artifact = false;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => {
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    return Ok(BodyTooLarge {
                        limit: state.max_body_bytes,
                        received: Some(received),
                    }
                    .into_response());
                }
                return Err(multipart_error(&error));
            }
        };
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "artifact" => {
                got_artifact = true;
                let mut field = field;
                loop {
                    let chunk = match field.chunk().await {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => break,
                        Err(error) => {
                            if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                                return Ok(BodyTooLarge {
                                    limit: state.max_body_bytes,
                                    received: Some(received),
                                }
                                .into_response());
                            }
                            return Err(multipart_error(&error));
                        }
                    };
                    received = received.saturating_add(chunk.len() as u64);
                    if received > state.max_body_bytes {
                        return Ok(BodyTooLarge {
                            limit: state.max_body_bytes,
                            received: Some(received),
                        }
                        .into_response());
                    }
                    staged.write_all(&chunk).map_err(|e| {
                        ApiError::internal(format!("could not stage the artifact: {}", e))
                    })?;
                }
            }
            other => {
                let value = field.text().await.map_err(|e| multipart_error(&e))?;
                match other {
                    "binding" => binding = value,
                    "branch" => branch = value,
                    "message" => message = value,
                    "author" => author = value,
                    "holder" => holder = value,
                    "acceptLosses" => accept_losses.push(value),
                    _ => {}
                }
            }
        }
    }

    if !got_artifact {
        return Err(ApiError::bad_request(
            "the source artifact must be supplied as the `artifact` file field",
        ));
    }
    if binding.is_empty() {
        return Err(ApiError::bad_request("a binding must be selected"));
    }
    if branch.is_empty() {
        branch = "main".to_string();
    }
    validate_name("branch name", &branch)?;
    if message.trim().is_empty() {
        return Err(ApiError::bad_request("commit message must not be empty"));
    }
    let author = resolve_author(&state.auth, &identity, &author)?;
    let holder_opt = if holder.is_empty() {
        None
    } else {
        Some(holder.as_str())
    };
    verify_actor(&state.auth, &identity, holder_opt)?;

    // Rule 1: retain the artifact byte-for-byte, content-addressed, BEFORE import is even
    // attempted. The staged file is moved into the blob store under its hash now; the
    // bytes are read back only to hand them to the binding, which needs the whole document.
    let temp_path = staged.into_temp_path();
    let artifact_hash = state
        .store
        .put_blob_file(temp_path.as_ref())
        .map_err(map_store_error)?;
    let artifact_bytes = state
        .store
        .blob(&artifact_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| {
            eprintln!(
                "missing blob {} immediately after storing it",
                artifact_hash
            );
            ApiError::internal("the retained artifact could not be read back")
        })?;

    match import_core(
        state.store.as_ref(),
        &project,
        &ImportCore {
            binding: &binding,
            branch: &branch,
            author: &author,
            message: &message,
            artifact: &artifact_bytes,
            accept_losses: &accept_losses,
            holder: holder_opt,
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
            authorizer: state.auth.authorizer().unwrap_or(""),
            acceptance: None,
        },
    )? {
        ImportOutcome::Committed {
            commit,
            loss_report,
            fidelity,
            ..
        } => Ok((
            StatusCode::CREATED,
            Json(json!({
                "commit": commit_json(&commit),
                "artifactHash": artifact_hash,
                "lossReport": serde_json::to_value(&loss_report).unwrap_or(Value::Null),
                "fidelity": serde_json::to_value(&fidelity.diff).unwrap_or(Value::Null)
            })),
        )
            .into_response()),
        ImportOutcome::Blocking { unaccepted, .. } => {
            let blocking_json: Vec<Value> = unaccepted
                .iter()
                .map(|m| serde_json::to_value(m).unwrap_or(Value::Null))
                .collect();
            Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "the import has blocking losses that must be accepted by name",
                    "artifactHash": artifact_hash,
                    "blocking": blocking_json
                })),
            )
                .into_response())
        }
    }
}

pub async fn import_report(
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, artifact_hash)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let record = state
        .store
        .import_report(&project, &artifact_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("import {}", artifact_hash)))?;
    Ok(Json(json!({
        "artifactHash": record.artifact_hash,
        "bindingId": record.binding_id,
        "bindingVersion": record.binding_version,
        "lossReport": serde_json::from_str::<Value>(&record.loss_report).unwrap_or(Value::Null),
        "fidelity": serde_json::from_str::<Value>(&record.fidelity_diff).unwrap_or(Value::Null)
    })))
}

/// GET /projects/:project/import/:artifactHash/artifact - the retained source artifact,
/// byte for byte.
///
/// A reader with Read on the project can re-fetch the exact bytes the migration started
/// from, so the "retained byte-for-byte" promise is CHECKABLE: compare these bytes to the
/// hash the import provenance names (sha256 of the body equals artifactHash). The route is
/// scoped to the project, not just to a global content address: the import record for
/// (project, artifactHash) must exist, so one project cannot use this route to read another
/// project's artifact by guessing a hash.
pub async fn get_artifact(
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, artifact_hash)): Path<(String, String)>,
) -> Result<axum::response::Response, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let record = state
        .store
        .import_report(&project, &artifact_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("import {}", artifact_hash)))?;
    let bytes = state
        .store
        .blob(&record.artifact_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| {
            eprintln!("missing blob {}", record.artifact_hash);
            ApiError::internal("the retained artifact is missing")
        })?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    Ok((StatusCode::OK, headers, bytes).into_response())
}
