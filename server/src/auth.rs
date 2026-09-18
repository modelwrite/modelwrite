// SPDX-License-Identifier: AGPL-3.0-or-later
//! Identity, roles and the opt-in authentication layer.
//!
//! Authentication is OPT-IN. With no configuration the service runs in OPEN mode, every
//! request is accepted, and the identity is an anonymous admin (`Identity::open()`). When
//! a static bearer token is configured, a request must present it (SHA-256, compared in
//! constant time) or be refused with 401. When a JWKS is configured, a request must
//! present a signed JWT whose RS256/RS384/RS512 signature verifies against one of the
//! configured keys and whose expiry, not-before, issuer and audience are valid; roles and
//! projects arrive from configurable claims.

use std::collections::HashMap;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use sha2::{Digest, Sha256};

use crate::api::ApiState;
use crate::error::ApiError;

/// The verified actor behind a request: who they are, what roles they hold, and which
/// projects those roles reach. In open mode this is an anonymous admin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    pub roles: Vec<String>,
    pub projects: Vec<String>,
}

impl Identity {
    /// The open-mode identity: nobody was authenticated, so the caller is an anonymous
    /// admin with every permission and every project. This is the ONLY identity open mode
    /// ever produces, so open mode can never degrade into a permission denial.
    pub fn open() -> Self {
        Self {
            subject: "anonymous".to_string(),
            roles: vec!["admin".to_string()],
            projects: vec!["*".to_string()],
        }
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// Whether any of the caller's roles grants the permission.
    pub fn may(&self, permission: Permission) -> bool {
        self.roles
            .iter()
            .any(|role| permission.granted_roles().contains(&role.as_str()))
    }

    /// Whether the caller's roles reach the given project. A projects entry of `"*"`
    /// reaches every project.
    pub fn may_reach(&self, project: &str) -> bool {
        self.projects.iter().any(|p| p == "*" || p == project)
    }
}

/// The four capabilities a route can demand. Each maps to the roles that grant it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
    Review,
    Administer,
}

impl Permission {
    /// The roles that grant this permission. `viewer` reads; `author` writes (commits,
    /// branches, resets, merges and locks); `reviewer` reads gate runs and evidence;
    /// `admin` administers (project creation, branch deletion, lock breaking) and holds
    /// every lesser permission.
    pub fn granted_roles(self) -> &'static [&'static str] {
        match self {
            Permission::Read => &["viewer", "author", "reviewer", "admin"],
            Permission::Write => &["author", "admin"],
            Permission::Review => &["reviewer", "admin"],
            Permission::Administer => &["admin"],
        }
    }
}

/// How the service decides who a request is. Authentication is opt-in: the `Open` variant
/// is the default, and a request is accepted without a credential. The `Static` and
/// `Jwt` variants require a bearer token.
#[derive(Clone)]
pub enum AuthConfig {
    Open,
    /// A single shared bearer token, stored as its SHA-256 hex digest so the secret itself
    /// is never held and comparison is over a fixed-width digest.
    Static {
        token_hash: String,
    },
    /// Signed JWTs verified against a JWKS. `keys` is keyed by the `kid` each key
    /// declares; `issuer` and `audience`, when present, must match the corresponding
    /// claim. `role_claim` and `project_claim` name the claims that carry the caller's
    /// roles and projects.
    Jwt {
        keys: HashMap<String, DecodingKey>,
        issuer: Option<String>,
        audience: Option<String>,
        role_claim: String,
        project_claim: String,
    },
    /// Every request resolves to this exact identity. Only tests construct this: it lets
    /// the permission and project-scope decisions be exercised with a specific role or
    /// scope. `from_env` never produces it.
    Fixed(Identity),
}

/// Redacted on purpose: a derived Debug would print the token digest or the JWKS keys, and
/// a configuration that can be printed into a log is a configuration that eventually will
/// be. The digest is not the secret but it is the thing compared to the secret, so it stays
/// out of logs; the JWKS keys are public, but their count is all a log needs.
impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthConfig::Open => write!(f, "AuthConfig::Open"),
            AuthConfig::Static { .. } => {
                write!(f, "AuthConfig::Static {{ token_hash: <redacted> }}")
            }
            AuthConfig::Jwt {
                keys,
                issuer,
                audience,
                role_claim,
                project_claim,
            } => f
                .debug_struct("AuthConfig::Jwt")
                .field("keys", &keys.len())
                .field("issuer", issuer)
                .field("audience", audience)
                .field("role_claim", role_claim)
                .field("project_claim", project_claim)
                .finish(),
            AuthConfig::Fixed(identity) => f
                .debug_struct("AuthConfig::Fixed")
                .field("subject", &identity.subject)
                .field("roles", &identity.roles)
                .field("projects", &identity.projects)
                .finish(),
        }
    }
}

impl AuthConfig {
    /// Read the configuration from the environment. A static token (`MW_AUTH_TOKEN`) is
    /// the simplest option; signed JWTs arrive from a JWKS at the file path `MW_AUTH_JWKS`
    /// (a mounted secret, which is what an air-gapped install has). With neither the service
    /// runs OPEN. A malformed JWKS is a startup error, not a silent fall back to open mode.
    pub fn from_env() -> anyhow::Result<AuthConfig> {
        if let Ok(token) = std::env::var("MW_AUTH_TOKEN") {
            if !token.trim().is_empty() {
                return Ok(AuthConfig::static_token(token.trim()));
            }
        }
        if let Ok(path) = std::env::var("MW_AUTH_JWKS") {
            if !path.trim().is_empty() {
                return AuthConfig::from_jwks_file(path.trim());
            }
        }
        Ok(AuthConfig::Open)
    }

    /// Build the static-token configuration, hashing the token ONCE so every later
    /// comparison is over a fixed-width digest and no token length is leaked.
    pub fn static_token(token: &str) -> AuthConfig {
        AuthConfig::Static {
            token_hash: hash_token(token),
        }
    }

    /// The JWT configuration with the default claim names: roles in `roles`, projects in
    /// `projects`. Use `jwt_with_claims` to rename them.
    pub fn jwt(
        keys: HashMap<String, DecodingKey>,
        issuer: Option<String>,
        audience: Option<String>,
    ) -> AuthConfig {
        AuthConfig::jwt_with_claims(
            keys,
            issuer,
            audience,
            "roles".to_string(),
            "projects".to_string(),
        )
    }

    /// The JWT configuration with explicit claim names.
    pub fn jwt_with_claims(
        keys: HashMap<String, DecodingKey>,
        issuer: Option<String>,
        audience: Option<String>,
        role_claim: String,
        project_claim: String,
    ) -> AuthConfig {
        AuthConfig::Jwt {
            keys,
            issuer,
            audience,
            role_claim,
            project_claim,
        }
    }

    /// Build a JWT configuration from a JWKS FILE (a mounted secret, which is what an
    /// air-gapped install has). Issuer, audience and claim names come from the environment.
    pub fn from_jwks_file(path: &str) -> anyhow::Result<AuthConfig> {
        let jwks = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read the JWKS at {}: {}", path, e))?;
        let keys = parse_jwks(&jwks)?;
        Ok(AuthConfig::jwt_with_claims(
            keys,
            jwt_env_issuer(),
            jwt_env_audience(),
            jwt_env_claim("MW_AUTH_ROLES_CLAIM", "roles"),
            jwt_env_claim("MW_AUTH_PROJECTS_CLAIM", "projects"),
        ))
    }

    /// Pin every request to one identity. This is a test hook: `from_env` never produces
    /// it, and it exists so the role and project-scope decisions on every route can be
    /// exercised with a viewer, an author or a single-project scope.
    pub fn fixed(identity: Identity) -> AuthConfig {
        AuthConfig::Fixed(identity)
    }
}

/// SHA-256 of a token as lowercase hex. The configured token is hashed once at startup so
/// the stored secret is a fixed-width digest, never the token itself.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Constant-time equality over byte slices. There is no early return on the first differing
/// byte, so the comparison time does not depend on where a difference is. Both operands are
/// SHA-256 digests (fixed width), so a length mismatch is a configuration error rather than
/// a secret-length leak; it still returns `false` without panicking.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// The identity a correct static token maps to. The single shared secret carries no
/// per-user claims, so it is the administrator's credential and the subject is fixed.
fn static_identity() -> Identity {
    Identity {
        subject: "admin".to_string(),
        roles: vec!["admin".to_string()],
        projects: vec!["*".to_string()],
    }
}

/// Resolve a presented token against the static configuration: `Some(identity)` when the
/// SHA-256 of the token equals the configured digest (constant time), `None` otherwise. A
/// non-static configuration has no static token to match, so it is `None`.
pub fn parse_identity_from_static(token: &str, config: &AuthConfig) -> Option<Identity> {
    let AuthConfig::Static { token_hash } = config else {
        return None;
    };
    let presented = hash_token(token);
    if constant_time_eq(presented.as_bytes(), token_hash.as_bytes()) {
        Some(static_identity())
    } else {
        None
    }
}

/// Parse a JWKS document into decoding keys, keyed by each key's `kid` (an empty string
/// when a key declares none). Only RSA keys are kept: they are the only kind that can
/// verify the RSA signatures this service accepts, and a set that also carries an EC or
/// octet key is common rather than erroneous.
pub fn parse_jwks(jwks: &str) -> anyhow::Result<HashMap<String, DecodingKey>> {
    let set: jsonwebtoken::jwk::JwkSet = serde_json::from_str(jwks)
        .map_err(|e| anyhow::anyhow!("the JWKS is not valid JSON: {}", e))?;
    let mut keys = HashMap::new();
    for jwk in set.keys {
        let jsonwebtoken::jwk::AlgorithmParameters::RSA(rsa) = jwk.algorithm else {
            continue;
        };
        let key = DecodingKey::from_rsa_components(&rsa.n, &rsa.e)
            .map_err(|e| anyhow::anyhow!("a JWKS RSA key is unusable: {}", e))?;
        keys.insert(jwk.common.key_id.unwrap_or_default(), key);
    }
    if keys.is_empty() {
        return Err(anyhow::anyhow!("the JWKS contains no RSA keys"));
    }
    Ok(keys)
}

/// The expected issuer from `MW_AUTH_ISSUER`, or `None` when it is unset or empty.
fn jwt_env_issuer() -> Option<String> {
    std::env::var("MW_AUTH_ISSUER")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// The expected audience from `MW_AUTH_AUDIENCE`, or `None` when it is unset or empty.
fn jwt_env_audience() -> Option<String> {
    std::env::var("MW_AUTH_AUDIENCE")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// A claim name from the environment, with a default. Empty values fall back to the
/// default rather than producing a claim that can never match.
fn jwt_env_claim(var: &str, default: &str) -> String {
    std::env::var(var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// A roles or projects claim is either a JSON array of strings or a single string. Anything
/// else - absent, a number, an object - yields nothing, because absence must deny rather
/// than default to something permissive.
fn string_list_claim(claims: &serde_json::Value, key: &str) -> Vec<String> {
    match claims.get(key) {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Resolve a presented token against the JWT configuration: `Some(identity)` when the
/// signature, expiry, not-before, issuer and audience all verify, `None` otherwise. A
/// non-JWT configuration has no keys to verify against, so it is `None`.
pub fn parse_identity_from_jwt(token: &str, config: &AuthConfig) -> Option<Identity> {
    let AuthConfig::Jwt {
        keys,
        issuer,
        audience,
        role_claim,
        project_claim,
    } = config
    else {
        return None;
    };

    // The header selects the key (kid) and must declare a supported RSA algorithm. An
    // `alg: none` header does not even parse here, and the HMAC family is refused before
    // any key is consulted, so a signed-token check cannot be downgraded to a shared-secret
    // check.
    let header = decode_header(token).ok()?;
    if !matches!(
        header.alg,
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512
    ) {
        return None;
    }

    let kid = header.kid.as_deref().unwrap_or("");
    let key = keys.get(kid)?;

    let mut validation = Validation::new(header.alg);
    // 60 seconds of clock skew, so a service and an identity provider whose clocks differ
    // by seconds do not lock users out. Validation::new already requires `exp` and
    // rejects a malformed or absent expiry, so a token cannot shed its expiry by giving it
    // the wrong JSON type.
    validation.leeway = 60;
    // Enforce `nbf` exactly like `exp`: a future not-before is refused, and because `nbf`
    // is required, a token whose `nbf` has the wrong JSON type is refused rather than
    // treated as absent (the same type-confusion that motivated requiring `exp`).
    validation.validate_nbf = true;
    validation.required_spec_claims.insert("nbf".to_string());

    if let Some(iss) = issuer {
        validation.set_issuer(&[iss.as_str()]);
        // Requiring the claim makes a missing OR malformed issuer a hard failure rather
        // than a silently skipped check.
        validation.required_spec_claims.insert("iss".to_string());
    }

    match audience {
        Some(aud) => {
            validation.set_audience(&[aud.as_str()]);
            validation.required_spec_claims.insert("aud".to_string());
        }
        None => {
            // No audience is configured, so a token carrying an audience is not a mismatch:
            // audience verification is simply not enforced.
            validation.validate_aud = false;
        }
    }

    let data = decode::<serde_json::Value>(token, key, &validation).ok()?;
    let claims = data.claims;

    let subject = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let roles = string_list_claim(&claims, role_claim);
    let projects = string_list_claim(&claims, project_claim);

    Some(Identity {
        subject,
        roles,
        projects,
    })
}

/// The `Authorization: Bearer <token>` value, or `None` when the header is missing or does
/// not carry a non-empty bearer token.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(axum::http::header::AUTHORIZATION)?;
    let value = value.to_str().ok()?;
    let rest = value.strip_prefix("Bearer ")?;
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// Resolve the request's identity against the configured authentication. In open mode every
/// request is the anonymous admin; in static mode the bearer token must match; in JWT mode
/// the bearer token must verify against the configured JWKS. A missing or bad token is 401.
pub async fn identity(state: &ApiState, headers: &HeaderMap) -> Result<Identity, ApiError> {
    match &state.auth {
        AuthConfig::Open => Ok(Identity::open()),
        AuthConfig::Fixed(identity) => Ok(identity.clone()),
        AuthConfig::Static { .. } => {
            let token = bearer_token(headers)
                .ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
            parse_identity_from_static(token, &state.auth)
                .ok_or_else(|| ApiError::unauthorized("invalid bearer token"))
        }
        AuthConfig::Jwt { .. } => {
            let token = bearer_token(headers)
                .ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
            parse_identity_from_jwt(token, &state.auth)
                .ok_or_else(|| ApiError::unauthorized("invalid bearer token"))
        }
    }
}

#[axum::async_trait]
impl FromRequestParts<ApiState> for Identity {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ApiState,
    ) -> Result<Self, Self::Rejection> {
        identity(state, &parts.headers).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_with_roles(roles: &[&str]) -> Identity {
        Identity {
            subject: "test".to_string(),
            roles: roles.iter().map(|s| s.to_string()).collect(),
            projects: vec!["*".to_string()],
        }
    }

    #[test]
    fn open_identity_is_anonymous_admin_everywhere() {
        let id = Identity::open();
        assert_eq!(id.subject, "anonymous");
        assert!(id.has_role("admin"));
        assert!(id.may(Permission::Read));
        assert!(id.may(Permission::Write));
        assert!(id.may(Permission::Review));
        assert!(id.may(Permission::Administer));
        assert!(id.may_reach("any-project"));
    }

    #[test]
    fn role_mapping_grants_each_permission_to_its_roles() {
        assert!(identity_with_roles(&["viewer"]).may(Permission::Read));
        assert!(!identity_with_roles(&["viewer"]).may(Permission::Write));
        assert!(!identity_with_roles(&["viewer"]).may(Permission::Review));
        assert!(!identity_with_roles(&["viewer"]).may(Permission::Administer));

        assert!(identity_with_roles(&["author"]).may(Permission::Read));
        assert!(identity_with_roles(&["author"]).may(Permission::Write));
        assert!(!identity_with_roles(&["author"]).may(Permission::Administer));

        assert!(identity_with_roles(&["reviewer"]).may(Permission::Read));
        assert!(identity_with_roles(&["reviewer"]).may(Permission::Review));
        assert!(!identity_with_roles(&["reviewer"]).may(Permission::Write));

        assert!(identity_with_roles(&["admin"]).may(Permission::Write));
        assert!(identity_with_roles(&["admin"]).may(Permission::Review));
        assert!(identity_with_roles(&["admin"]).may(Permission::Administer));
    }

    #[test]
    fn viewer_cannot_write_and_author_cannot_administer() {
        assert!(!identity_with_roles(&["viewer"]).may(Permission::Write));
        assert!(!identity_with_roles(&["author"]).may(Permission::Administer));
    }

    #[test]
    fn has_role_is_exact() {
        let id = identity_with_roles(&["author"]);
        assert!(id.has_role("author"));
        assert!(!id.has_role("admin"));
        assert!(!id.has_role(""));
    }

    #[test]
    fn project_scope_reaches_exact_and_wildcard() {
        let scoped = Identity {
            subject: "s".to_string(),
            roles: vec!["author".to_string()],
            projects: vec!["coffee".to_string()],
        };
        assert!(scoped.may_reach("coffee"));
        assert!(!scoped.may_reach("tea"));

        let wildcard = Identity {
            subject: "s".to_string(),
            roles: vec!["author".to_string()],
            projects: vec!["*".to_string()],
        };
        assert!(wildcard.may_reach("tea"));
    }

    #[test]
    fn constant_time_equality_agrees_on_same_and_different() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"abc", b"abd")); // last byte differs
        assert!(!constant_time_eq(b"abc", b"xbc")); // first byte differs
        assert!(!constant_time_eq(b"abc", b"ab")); // length differs
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn static_token_is_a_fixed_width_digest_and_matches_in_constant_time() {
        let token = "s3cret-token";
        let config = AuthConfig::static_token(token);
        let AuthConfig::Static { token_hash } = &config else {
            panic!("expected a static configuration");
        };
        assert_eq!(token_hash.len(), 64);
        assert!(token_hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(
            !token_hash.contains(token),
            "the token must never be stored or echoed"
        );

        let good = parse_identity_from_static(token, &config).expect("correct token is accepted");
        assert_eq!(good.subject, "admin");
        assert!(good.has_role("admin"));
        assert!(good.may_reach("any-project"));

        assert!(parse_identity_from_static("wrong-token", &config).is_none());
        // A token that only shares a prefix must not match: the digest is compared in full.
        assert!(parse_identity_from_static("s3cret-toke", &config).is_none());
        // A static token cannot be resolved against a non-static configuration.
        assert!(parse_identity_from_static(token, &AuthConfig::Open).is_none());
        assert!(
            parse_identity_from_static(token, &AuthConfig::jwt(HashMap::new(), None, None))
                .is_none()
        );
    }

    #[test]
    fn hashing_is_stable_hex_and_reveals_nothing() {
        let a = hash_token("the-token");
        let b = hash_token("the-token");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(!a.contains("the-token"));
        assert_ne!(hash_token("the-token"), hash_token("other-token"));
    }

    #[test]
    fn parse_jwks_keeps_rsa_keys_by_kid_and_skips_others() {
        // Two RSA keys (one kid-less) and one octet key that must be skipped.
        let jwks = serde_json::json!({
            "keys": [
                { "kty": "RSA", "kid": "k1", "n": "AQID", "e": "AQAB" },
                { "kty": "RSA", "n": "BAUG", "e": "AQAB" },
                { "kty": "oct", "kid": "k2", "k": "c2VjcmV0" },
            ]
        });
        let keys = parse_jwks(&jwks.to_string()).expect("a JWKS with RSA keys parses");
        assert_eq!(keys.len(), 2, "the octet key must be skipped");
        assert!(keys.contains_key("k1"), "kid k1 must be present");
        assert!(
            keys.contains_key(""),
            "the kid-less key maps to the empty string"
        );
        assert!(!keys.contains_key("k2"), "the octet key must not be kept");
    }

    #[test]
    fn parse_jwks_rejects_a_set_with_no_rsa_keys() {
        let jwks = serde_json::json!({
            "keys": [ { "kty": "oct", "kid": "k", "k": "c2VjcmV0" } ]
        });
        assert!(parse_jwks(&jwks.to_string()).is_err());
    }

    #[test]
    fn string_list_claim_reads_array_single_string_and_denies_absence() {
        let claims = serde_json::json!({
            "roles": ["author", "reviewer"],
            "one": "admin",
            "wrong": 7,
        });
        assert_eq!(
            string_list_claim(&claims, "roles"),
            vec!["author".to_string(), "reviewer".to_string()]
        );
        assert_eq!(string_list_claim(&claims, "one"), vec!["admin".to_string()]);
        assert_eq!(string_list_claim(&claims, "wrong"), Vec::<String>::new());
        assert_eq!(string_list_claim(&claims, "absent"), Vec::<String>::new());
    }

    #[test]
    fn jwt_debug_redacts_the_keys() {
        // Build the config from a REAL parsed JWKS: an empty key map would pass this
        // assertion even if the Debug impl printed every key, because there would be no
        // key material to print.
        let jwks = serde_json::json!({
            "keys": [ { "kty": "RSA", "kid": "k1", "n": "AQID", "e": "AQAB" } ]
        });
        let keys = parse_jwks(&jwks.to_string()).expect("the JWKS parses");
        let config = AuthConfig::jwt(keys, Some("iss".into()), Some("aud".into()));
        let debug = format!("{:?}", config);
        assert!(debug.contains("AuthConfig::Jwt"));
        assert!(
            debug.contains("keys: 1"),
            "the key COUNT is reported, not the keys themselves"
        );
        assert!(
            !debug.contains("k1"),
            "a JWKS kid must not appear in the debug output"
        );
        assert!(
            !debug.contains("AQID"),
            "a JWKS modulus must not appear in the debug output"
        );
    }
}
