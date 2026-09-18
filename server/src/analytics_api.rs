// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{load_model, map_store_error, ApiState};
use crate::auth::{Identity, Permission};
use crate::error::ApiError;
use crate::store::{Commit, Store};

/// The query surface of `GET /projects/:project/analytics`.
///
/// `requirements` is a comma-separated specification: each id is classified against the
/// model as Covered, Uncovered or Unknown, and the report's three counts always sum to the
/// specification's size, so an absent requirement can never hide as a blank. `commit`, when
/// present, names the exact model to read; otherwise `branch` (default `main`) is resolved
/// to its tip.
///
/// The `cost*` parameters are all optional and together describe ONE inline cost dataset.
/// When `costCsv` is absent the cost answer is still produced - every requirement reads
/// UNCOSTED, never zero and never omitted. When it is present, the source is declared in the
/// registry (its trust fixed at declaration), the CSV is snapshotted with a read time (default
/// now), and the join labels every costed figure with its value, source, trust and read time.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsQuery {
    /// The specification: a comma-separated list of requirement ids.
    pub requirements: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub cost_csv: Option<String>,
    pub cost_source: Option<String>,
    pub cost_trust: Option<String>,
    pub cost_requirement_column: Option<String>,
    pub cost_cost_column: Option<String>,
    pub cost_captured_at: Option<String>,
}

/// A read-only analytics answer over one model: the portfolio-compliance report for a
/// caller-supplied specification, and the cost state of the model's own requirements. Both
/// come straight from the analytics crate - this handler is the WIRING to a real model in
/// the store, not a second implementation of either question.
pub async fn project_analytics(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Query(query): Query<AnalyticsQuery>,
) -> Result<Json<Value>, ApiError> {
    // Identity first, then permission and scope, BEFORE the store is read - the same order
    // every read handler uses. A caller without Read gets a refusal, never an empty report.
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }

    // The specification is the comma-separated id list, trimmed and de-blanked. An empty
    // specification has nothing to classify; refusing it keeps the answer honest rather than
    // reporting a vacuously complete zero-row report.
    let spec: Vec<String> = query
        .requirements
        .as_deref()
        .map(|list| {
            list.split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if spec.is_empty() {
        return Err(ApiError::bad_request(
            "at least one requirement is required",
        ));
    }
    if state
        .store
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }

    let commit = resolve_commit(
        state.store.as_ref(),
        &project,
        query.commit.as_deref(),
        query.branch.as_deref(),
    )?;
    // The existing load path: commit row, then its blob, then parse. No second reader.
    let model =
        load_model(state.store.as_ref(), &project, &commit.hash).map_err(map_store_error)?;

    let spec_refs: Vec<&str> = spec.iter().map(String::as_str).collect();
    let report = analytics::portfolio_report(&spec_refs, &[&model]);

    let (registry, datasets) = inline_cost(&query)?;
    let dataset_refs: Vec<(&analytics::Dataset, &analytics::ColumnMapping)> = datasets
        .iter()
        .map(|(dataset, mapping)| (dataset, mapping))
        .collect();
    // An uncosted requirement is UNCOSTED, never zero and never omitted; a costed one carries
    // its value, its weakest-link trust and the full set of contributing sources and dates.
    let costs = analytics::cost_by_requirement(&[&model], &registry, &dataset_refs)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    Ok(Json(json!({
        "project": project,
        "commit": commit.hash,
        "branch": commit.branch,
        "compliance": serde_json::to_value(report).map_err(|e| {
            ApiError::internal(format!("the compliance report could not be serialised: {}", e))
        })?,
        "costs": serde_json::to_value(costs).map_err(|e| {
            ApiError::internal(format!("the cost answer could not be serialised: {}", e))
        })?,
    })))
}

/// Resolve the commit to read: an explicit `commit` hash wins, otherwise `branch`
/// (default `main`) is resolved to its tip. A missing commit or a missing branch tip is a
/// 404 naming only what the caller asked for - never internal detail.
fn resolve_commit(
    store: &dyn Store,
    project: &str,
    commit: Option<&str>,
    branch: Option<&str>,
) -> Result<Commit, ApiError> {
    let hash = if let Some(hash) = commit {
        hash.to_string()
    } else {
        let branch = branch.unwrap_or("main");
        store
            .branch_tip(project, branch)
            .map_err(map_store_error)?
            .ok_or_else(|| ApiError::not_found(format!("branch {}", branch)))?
    };
    store
        .commit(project, &hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))
}

/// Build the inline cost dataset the request may supply. The source is declared in the
/// registry (its trust fixed there, never graded later), the CSV is snapshotted with a read
/// time, and the column mapping names the requirement and cost columns. Absent `costCsv`
/// means no dataset was supplied: the cost question is still answered, with everything
/// UNCOSTED, because an empty registry is a truthful "no estimates" rather than a blank.
fn inline_cost(
    query: &AnalyticsQuery,
) -> Result<
    (
        analytics::Registry,
        Vec<(analytics::Dataset, analytics::ColumnMapping)>,
    ),
    ApiError,
> {
    let mut registry = analytics::Registry::new();
    let Some(csv) = query.cost_csv.as_deref() else {
        return Ok((registry, Vec::new()));
    };
    let source_id = query.cost_source.as_deref().unwrap_or("cost").to_string();
    let trust = match query.cost_trust.as_deref().unwrap_or("reported") {
        "measured" => analytics::TrustLevel::Measured,
        "reported" => analytics::TrustLevel::Reported,
        "estimated" => analytics::TrustLevel::Estimated,
        other => {
            return Err(ApiError::bad_request(format!(
                "costTrust must be measured, reported or estimated, got {}",
                other
            )))
        }
    };
    registry
        .register(analytics::Source {
            id: source_id.clone(),
            kind: analytics::SourceKind::Structured,
            trust,
            description: "inline cost data supplied with the request".to_string(),
        })
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let captured_at = query
        .cost_captured_at
        .clone()
        .unwrap_or_else(crate::store::now_epoch);
    let dataset = registry
        .snapshot(&source_id, captured_at, csv.as_bytes())
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let mapping = analytics::ColumnMapping::new(
        query
            .cost_requirement_column
            .as_deref()
            .unwrap_or("requirement_id"),
        query.cost_cost_column.as_deref().unwrap_or("unit_cost"),
    );
    Ok((registry, vec![(dataset, mapping)]))
}
