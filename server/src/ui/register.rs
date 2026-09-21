// SPDX-License-Identifier: AGPL-3.0-or-later
//! The no-JS registration flow: email form, single-use code, session cookie, and the account
//! page where the marketing consent is visible and changeable. Every page is server-rendered
//! HTML; no JavaScript is required or shipped.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Form, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use maud::{html, Markup};

use crate::store;
use crate::trial::{self, TrialError, TrialService};

/// A minimal shared stylesheet for the auth pages, matching the workbench palette.
const STYLE: &str = "body{font-family:system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;background:#f6f8fa;color:#1f2328;margin:0}main{max-width:30rem;margin:4rem auto;padding:0 1rem}.card{background:#fff;border:1px solid #d1d9e0;border-radius:6px;padding:1.5rem}label{display:block;font-weight:600;font-size:13px;margin:0.75rem 0 0.25rem}input[type=email],input[type=text]{width:100%;box-sizing:border-box;font-size:15px;padding:0.5rem;border:1px solid #d1d9e0;border-radius:4px}button{background:#2563eb;color:#fff;border:0;border-radius:4px;padding:0.5rem 1rem;font-size:14px;font-weight:600;cursor:pointer;margin-top:1rem}.err{background:#ffebe9;color:#cf222e;border:1px solid #cf222e;border-radius:4px;padding:0.6rem 0.8rem;margin-bottom:1rem}.meta{color:#59636e;font-size:13px}.ok{background:#dafbe1;color:#1a7f37;border:1px solid #1a7f37;border-radius:4px;padding:0.6rem 0.8rem;margin-bottom:1rem}";

fn auth_page(title: &str, body: Markup) -> Response {
    let doc = html! {
        (maud::DOCTYPE)
        html {
            head { meta charset="utf-8"; meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) } style { (maud::PreEscaped(STYLE)) } }
            body { main { div class="card" { h1 { (title) } (body) } } }
        }
    };
    (StatusCode::OK, Html(doc.into_string())).into_response()
}

/// The best-effort client address, for the per-IP registration rate limit. Behind the
/// cloudflared tunnel the real address rides the Cf-Connecting-Ip header.
fn client_ip(headers: &HeaderMap) -> String {
    for name in ["cf-connecting-ip", "x-forwarded-for", "x-real-ip"] {
        if let Some(value) = headers.get(name).and_then(|v| v.to_str().ok()) {
            let first = value.split(',').next().unwrap_or("").trim();
            if !first.is_empty() {
                return first.to_string();
            }
        }
    }
    "unknown".to_string()
}

/// The session token from the cookie or bearer header (shared with the auth layer's reading).
fn session_token(headers: &HeaderMap) -> Option<String> {
    crate::auth::session_token(headers).map(str::to_string)
}

fn error_notice(message: &str) -> Markup {
    html! { div class="err" { (message) } }
}

/// `GET /` in registered mode: land a signed-in visitor in the workbench, everyone else on
/// the registration form.
pub async fn welcome(State(tier): State<Arc<TrialService>>, headers: HeaderMap) -> Response {
    match session_token(&headers) {
        Some(token) if tier.resolve_session(&token).is_some() => {
            Redirect::to("/ui").into_response()
        }
        _ => Redirect::to("/register").into_response(),
    }
}

/// `GET /register` - the email form, with an UNCHECKED marketing opt-in that is never bundled
/// with the login code.
pub async fn register_form() -> Response {
    auth_page(
        "Start your Modelwrite trial",
        html! {
            p class="meta" { "No password. We email you a single-use code, and your trial is empty and private." }
            form method="post" action="/register" {
                label for="email" { "Email" }
                input type="email" id="email" name="email" required;
                p {
                    input type="checkbox" id="consent" name="consent";
                    label for="consent" style="display:inline;font-weight:400" { " Send me occasional updates about Modelwrite (optional)" }
                }
                button type="submit" { "Email me a login code" }
            }
            p class="meta" { "Already have a code? " a href="/login" { "Enter it" } }
        },
    )
}

/// `POST /register` - request a code. The consent choice is recorded with a timestamp but
/// NEVER gates the code; the code is transactional.
pub async fn submit_register(
    State(tier): State<Arc<TrialService>>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let email = form.get("email").cloned().unwrap_or_default();
    let consent = form.contains_key("consent");
    let ip = client_ip(&headers);
    match tier.request_code(&email, &ip, consent, store::now_seconds()) {
        Ok(_) => auth_page(
            "Check your email",
            html! {
                div class="ok" { "If that address is registered or eligible, a login code is on its way." }
                p class="meta" { "Enter the code on the next page to land in your empty workspace." }
                p { a href="/login" { "I have my code" } }
            },
        ),
        Err(e) => register_form_with_error(&e),
    }
}

fn register_form_with_error(error: &TrialError) -> Response {
    let message = match error {
        TrialError::RateLimited(m) => m.clone(),
        TrialError::CapReached => error.to_string(),
        TrialError::InvalidEmail(_) => "enter a valid email address".to_string(),
        other => other.to_string(),
    };
    auth_page(
        "Start your Modelwrite trial",
        html! {
            (error_notice(&message))
            form method="post" action="/register" {
                label for="email" { "Email" }
                input type="email" id="email" name="email" required;
                p { input type="checkbox" id="consent" name="consent"; label for="consent" style="display:inline;font-weight:400" { " Send me occasional updates about Modelwrite (optional)" } }
                button type="submit" { "Email me a login code" }
            }
        },
    )
}

/// `GET /login` - the code entry form.
pub async fn login_form() -> Response {
    auth_page(
        "Enter your login code",
        html! {
            p class="meta" { "The code was emailed to you and is single-use; it expires in 10 minutes." }
            form method="post" action="/login" {
                label for="email" { "Email" }
                input type="email" id="email" name="email" required;
                label for="code" { "Login code" }
                input type="text" id="code" name="code" inputmode="numeric" autocomplete="one-time-code" required;
                button type="submit" { "Enter my trial" }
            }
            p class="meta" { "No code? " a href="/register" { "Request one" } }
        },
    )
}

/// `POST /login` - redeem the code for a session, set the cookie, land in the workspace.
pub async fn submit_login(
    State(tier): State<Arc<TrialService>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let email = form.get("email").cloned().unwrap_or_default();
    let code = form.get("code").cloned().unwrap_or_default();
    match tier.redeem(&email, &code, store::now_seconds()) {
        Ok(session) => {
            let cookie = format!(
                "mw_session={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
                session.token, tier.config.session_ttl_seconds
            );
            ([(header::SET_COOKIE, cookie)], Redirect::to("/ui")).into_response()
        }
        Err(e) => {
            let message = match &e {
                TrialError::InvalidCode | TrialError::ExpiredCode | TrialError::CodeAlreadyUsed => {
                    "that code did not work; request a fresh one".to_string()
                }
                other => other.to_string(),
            };
            auth_page(
                "Enter your login code",
                html! {
                    (error_notice(&message))
                    form method="post" action="/login" {
                        label for="email" { "Email" }
                        input type="email" id="email" name="email" value=(email) required;
                        label for="code" { "Login code" }
                        input type="text" id="code" name="code" required;
                        button type="submit" { "Enter my trial" }
                    }
                },
            )
        }
    }
}

/// `POST /logout` - drop the session and clear the cookie.
pub async fn logout(State(tier): State<Arc<TrialService>>, headers: HeaderMap) -> Response {
    if let Some(token) = session_token(&headers) {
        let _ = tier.logout(&token);
    }
    let cookie = "mw_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0";
    (
        [(header::SET_COOKIE, cookie.to_string())],
        Redirect::to("/register"),
    )
        .into_response()
}

/// `GET /account` - the account page: the email, the trial's lifecycle state, and the
/// marketing consent (visible and changeable).
pub async fn account_page(State(tier): State<Arc<TrialService>>, headers: HeaderMap) -> Response {
    let Some(token) = session_token(&headers) else {
        return Redirect::to("/login").into_response();
    };
    let Some(session) = tier.resolve_session(&token) else {
        return Redirect::to("/login").into_response();
    };
    let account = tier.identity.account(&session.email).ok().flatten();
    let trial_row = tier.identity.trial(&session.trial_id).ok().flatten();
    let phase = trial_row
        .as_ref()
        .map(|t| trial::phase(t.last_activity_at, store::now_seconds()));
    let consent = account
        .as_ref()
        .map(|a| a.marketing_consent)
        .unwrap_or(false);
    let consent_at = account.as_ref().map(|a| a.consent_changed_at).unwrap_or(0);
    let status = match phase {
        Some(trial::Phase::Active) => "active",
        Some(trial::Phase::ReadOnly) => "read-only (writes paused; use it to restart the window)",
        Some(trial::Phase::Frozen) => "frozen (archived)",
        None => "unknown",
    };
    let body = html! {
        p { "Signed in as " b { (session.email) } }
        p class="meta" { "Trial status: " (status) }
        form method="post" action="/account" {
            input type="hidden" name="consent" value=(!consent);
            @if consent {
                p { "Marketing updates: " b { "on" } " (opted in" @if consent_at > 0 { " at " (consent_at) } ")" }
                button type="submit" { "Turn off updates" }
            } @else {
                p { "Marketing updates: " b { "off" } }
                button type="submit" { "Send me occasional updates" }
            }
        }
        form method="post" action="/logout" { button type="submit" { "Sign out" } }
    };
    auth_page("Your account", body)
}

/// `POST /account` - change the marketing consent, recording the timestamp.
pub async fn update_consent(
    State(tier): State<Arc<TrialService>>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let Some(token) = session_token(&headers) else {
        return Redirect::to("/login").into_response();
    };
    let Some(session) = tier.resolve_session(&token) else {
        return Redirect::to("/login").into_response();
    };
    let consent = form.get("consent").map(|v| v == "true").unwrap_or(false);
    let _ = tier
        .identity
        .set_consent(&session.email, consent, store::now_seconds());
    Redirect::to("/account").into_response()
}
