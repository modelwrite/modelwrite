// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Every failure path returns this: a status code and a JSON body, never a panic.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    /// A well-formed OKF document that fails validation: the model is the problem,
    /// not the request, so the code is 422 and the report travels with it.
    pub fn unprocessable(message: impl Into<String>, errors: Vec<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: format!("{}: {}", message.into(), errors.join("; ")),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    /// A request with no bearer token, or one that does not match the configured
    /// authentication. The token itself is never echoed.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    /// An authenticated caller whose roles do not grant the required permission.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    /// The service cannot honour the request because a capability is not configured - for
    /// example, the live assistant has no API key. The request itself is well formed; the
    /// deployment is missing the configuration the capability needs.
    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

/// A request body larger than the configured limit. This is its own response type rather
/// than an `ApiError` because a refusal must NAME the configured limit and the size that
/// was received, so an operator (or a caller) can tell "raise the limit" from "something
/// is wrong with the upload". `received` is the exact size when the request declared a
/// `Content-Length` and otherwise the number of bytes actually read before the refusal.
#[derive(Debug)]
pub struct BodyTooLarge {
    pub limit: u64,
    pub received: Option<u64>,
}

impl IntoResponse for BodyTooLarge {
    fn into_response(self) -> Response {
        let message = match self.received {
            Some(received) => format!(
                "request body of {} bytes exceeds the configured limit of {} bytes",
                received, self.limit
            ),
            None => format!(
                "request body exceeds the configured limit of {} bytes",
                self.limit
            ),
        };
        let mut body = json!({
            "error": message,
            "limit": self.limit,
        });
        if let Some(received) = self.received {
            body["received"] = json!(received);
        }
        (StatusCode::PAYLOAD_TOO_LARGE, Json(body)).into_response()
    }
}
