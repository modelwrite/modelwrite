// SPDX-License-Identifier: AGPL-3.0-or-later
//! Identity, roles and the opt-in authentication layer.
//!
//! Authentication is OPT-IN. With no configuration the service runs in OPEN mode, every
//! request is accepted, and the identity is an anonymous admin (`Identity::open()`). When
//! a static bearer token is configured, a request must present it (SHA-256, compared in
//! constant time) or be refused with 401. Signed JWTs arrive in Task 4.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;
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
/// is the default, and the only non-open variant Task 1's startup path constructs is
/// `Static`.
#[derive(Debug, Clone)]
pub enum AuthConfig {
    Open,
    /// A single shared bearer token, stored as its SHA-256 hex digest so the secret itself
    /// is never held and comparison is over a fixed-width digest.
    Static {
        token_hash: String,
    },
    /// Signed JWTs verified against a JWKS. Task 4 implements this variant; until then it
    /// is present only so the configuration is complete and matches stay exhaustive about
    /// the not-yet-implemented path.
    Jwt {
        jwks_source: String,
    },
}

impl AuthConfig {
    /// Read the configuration from the environment. Task 1 wires the static token only;
    /// Task 4 wires the JWKS. With no `MW_AUTH_TOKEN` the service runs OPEN.
    pub fn from_env() -> AuthConfig {
        match std::env::var("MW_AUTH_TOKEN") {
            Ok(token) if !token.trim().is_empty() => AuthConfig::static_token(token.trim()),
            _ => AuthConfig::Open,
        }
    }

    /// Build the static-token configuration, hashing the token ONCE so every later
    /// comparison is over a fixed-width digest and no token length is leaked.
    pub fn static_token(token: &str) -> AuthConfig {
        AuthConfig::Static {
            token_hash: hash_token(token),
        }
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
/// request is the anonymous admin; in static mode the bearer token must match, or the call
/// is refused with 401. This is the extraction layer Task 2 threads through every route.
pub async fn identity(state: &ApiState, headers: &HeaderMap) -> Result<Identity, ApiError> {
    match &state.auth {
        AuthConfig::Open => Ok(Identity::open()),
        AuthConfig::Static { .. } => {
            let token = bearer_token(headers)
                .ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
            parse_identity_from_static(token, &state.auth)
                .ok_or_else(|| ApiError::unauthorized("invalid bearer token"))
        }
        // Task 4 implements signed-token verification. Task 1's startup never constructs
        // this variant, so the arm is unreachable until then; it still refuses rather than
        // panicking or admitting an unverified caller.
        AuthConfig::Jwt { .. } => Err(ApiError::unauthorized("authentication unavailable")),
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
        assert!(parse_identity_from_static(
            token,
            &AuthConfig::Jwt {
                jwks_source: "x".to_string()
            }
        )
        .is_none());
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
}
