// SPDX-License-Identifier: AGPL-3.0-or-later
//! The pluggable mailer. The login code is TRANSACTIONAL email: it delivers the service the
//! person just requested, so no marketing consent is claimed and the code arrives regardless.
//!
//! [Mailer] is the transport interface. Until an email-provider credential exists, the
//! console transport (and its file twin) writes each code and its recipient to the operator
//! log so the whole flow is testable end to end WITHOUT sending anything. Real delivery is
//! one credential away and does not change the flow: a future SMTP transport implements the
//! same trait behind that credential.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::trial::TrialError;

/// One outbound email. The login-code email is built by [login_code_email]; a future
/// marketing transport reuses the same shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundEmail {
    pub from: String,
    pub to: String,
    pub subject: String,
    pub body_plain: String,
}

/// The transport interface. A console transport logs, an SMTP transport would dial a server;
/// both implement this one method, so nothing above the transport changes when a credential
/// is added.
pub trait Mailer: Send + Sync {
    fn send(&self, email: &OutboundEmail) -> Result<(), TrialError>;
}

/// Writes every email to the operator log (stderr). The code and its recipient are therefore
/// visible to the operator without any network delivery.
pub struct ConsoleTransport;

impl Mailer for ConsoleTransport {
    fn send(&self, email: &OutboundEmail) -> Result<(), TrialError> {
        eprintln!(
            "[mailer:console] to={} subject={} body={}",
            email.to, email.subject, email.body_plain
        );
        Ok(())
    }
}

/// Appends every email to a file, one line per email. The file is the operator log: a line
/// carries the recipient and the code, never anything else.
pub struct FileTransport {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl FileTransport {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            write_lock: Mutex::new(()),
        }
    }
}

impl Mailer for FileTransport {
    fn send(&self, email: &OutboundEmail) -> Result<(), TrialError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| TrialError::Backend("mailer log lock poisoned".to_string()))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| TrialError::Backend(format!("cannot open mailer log: {}", e)))?;
        writeln!(
            file,
            "[mailer:file] to={} subject={} body={}",
            email.to, email.subject, email.body_plain
        )
        .map_err(|e| TrialError::Backend(format!("cannot write mailer log: {}", e)))?;
        Ok(())
    }
}

/// Build the transactional login-code email. The code is the credential; nothing else rides
/// in this email, and no marketing consent is claimed or required.
pub fn login_code_email(from: &str, to: &str, code: &str) -> OutboundEmail {
    OutboundEmail {
        from: from.to_string(),
        to: to.to_string(),
        subject: "Your Modelwrite login code".to_string(),
        body_plain: format!(
            "Your Modelwrite login code is {}. It expires in 10 minutes.",
            code
        ),
    }
}

/// The sender address, from MW_MAIL_FROM, defaulting to a real address on the modelwrite.org
/// domain. The spec requires the sender to be a real address on that domain.
pub fn sender_from_env() -> String {
    std::env::var("MW_MAIL_FROM").unwrap_or_else(|_| "no-reply@modelwrite.org".to_string())
}

/// Choose the transport from the environment. MW_MAILER=console (default) selects the
/// console transport; MW_MAILER=file selects the file transport at MW_MAILER_FILE. An SMTP
/// value is recognised but not yet wired: it falls back to the console transport with a loud
/// warning, because real SMTP delivery is one credential (and one client) away and must not
/// change this flow.
pub fn mailer_from_env() -> Arc<dyn Mailer> {
    match std::env::var("MW_MAILER").ok().as_deref() {
        Some("file") => {
            let path = std::env::var("MW_MAILER_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("mailer.log"));
            Arc::new(FileTransport::new(path))
        }
        Some("smtp") => {
            eprintln!("WARNING: MW_MAILER=smtp is not wired to a client yet; using the console transport. Real SMTP delivery is one credential away.");
            Arc::new(ConsoleTransport)
        }
        _ => Arc::new(ConsoleTransport),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_code_email_carries_only_the_code_and_recipient() {
        let email = login_code_email("no-reply@modelwrite.org", "a@example.com", "123456");
        assert_eq!(email.to, "a@example.com");
        assert_eq!(email.from, "no-reply@modelwrite.org");
        assert!(email.body_plain.contains("123456"));
        assert!(!email.body_plain.contains("consent"));
    }

    #[test]
    fn file_transport_appends_code_and_recipient_to_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mailer.log");
        let transport = FileTransport::new(path.clone());
        transport
            .send(&login_code_email(
                "from@example.com",
                "to@example.com",
                "654321",
            ))
            .unwrap();
        transport
            .send(&login_code_email(
                "from@example.com",
                "other@example.com",
                "111111",
            ))
            .unwrap();
        let log = std::fs::read_to_string(&path).unwrap();
        assert!(log.contains("to@example.com") && log.contains("654321"));
        assert!(log.contains("other@example.com") && log.contains("111111"));
        assert_eq!(log.lines().count(), 2);
    }
}
