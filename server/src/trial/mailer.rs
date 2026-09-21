// SPDX-License-Identifier: AGPL-3.0-or-later
//! The pluggable mailer. The login code is TRANSACTIONAL email: it delivers the service the
//! person just requested, so no marketing consent is claimed and the code arrives regardless.
//!
//! [Mailer] is the transport interface. Three transports implement it: the console transport
//! (and its file twin) writes each code and its recipient to the operator log so the whole
//! flow is testable end to end WITHOUT sending anything, and the SMTP transport dials a real
//! mail provider and delivers the same message over the wire. Nothing above the trait changes
//! when the transport does.

use std::io::{Read, Write};
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

/// The transport interface. A console transport logs, the SMTP transport dials a server;
/// both implement this one method, so nothing above the transport changes when the
/// transport changes.
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

/// A transport that refuses every send with one fixed message. It is the honest answer to a
/// deployment that asked for a transport this build cannot provide (an SMTP selection with no
/// configuration, or a build without the smtp feature): the registration request FAILS with
/// the reason instead of silently writing the code to the operator log, which would look like
/// a delivered email to everyone but the person waiting for it.
pub struct FailingTransport {
    reason: String,
}

impl FailingTransport {
    pub fn new(reason: String) -> Self {
        Self { reason }
    }
}

impl Mailer for FailingTransport {
    fn send(&self, _email: &OutboundEmail) -> Result<(), TrialError> {
        Err(TrialError::Mailer(self.reason.clone()))
    }
}

// ---------------------------------------------------------------------------------------
// SMTP
// ---------------------------------------------------------------------------------------

/// The environment variable that selects the transport: console (default), file or smtp.
pub const MAILER_VAR: &str = "MW_MAILER";
/// The environment variable naming the file transport's log.
pub const MAILER_FILE_VAR: &str = "MW_MAILER_FILE";
/// The sender address. It is both the SMTP envelope sender and the From: header, so the
/// provider-confirmed sender is configured in exactly one place.
pub const MAIL_FROM_VAR: &str = "MW_MAIL_FROM";
/// The SMTP server hostname (required by MW_MAILER=smtp).
pub const SMTP_HOST_VAR: &str = "MW_MAIL_SMTP_HOST";
/// The SMTP port. Defaults to 587, the STARTTLS submission port.
pub const SMTP_PORT_VAR: &str = "MW_MAIL_SMTP_PORT";
/// The SMTP username (required by MW_MAILER=smtp). Providers that authenticate with an API
/// token use the same value for the username and the password.
pub const SMTP_USERNAME_VAR: &str = "MW_MAIL_SMTP_USERNAME";
/// The SMTP password (required by MW_MAILER=smtp). It is a credential: it is read from the
/// environment and is never logged, never echoed in an error and never rendered by Debug.
pub const SMTP_PASSWORD_VAR: &str = "MW_MAIL_SMTP_PASSWORD";
/// The STARTTLS submission port, used when MW_MAIL_SMTP_PORT is absent.
pub const SMTP_PORT_DEFAULT: u16 = 587;

/// An environment lookup, so configuration can be tested without touching the process
/// environment (which is global and shared by every test in the binary).
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The settings the SMTP transport needs. Deliberately does not carry the sender: the
/// envelope sender and the From: header are the message's own from, which [TrialService]
/// fills from [MAIL_FROM_VAR], so the confirmed sender has one home.
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

/// Debug never renders the password. A derived Debug here would put the credential one dbg!
/// call or one test-failure panic message away from a log.
impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Read one required, non-empty setting. The error names the VARIABLE, never a value, so a
/// misconfiguration can be reported without disclosing a credential.
fn required(get: EnvLookup<'_>, name: &str) -> Result<String, TrialError> {
    match get(name).map(|v| v.trim().to_string()) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(TrialError::Mailer(format!(
            "{} is required when {} = smtp; set it in the service environment",
            name, MAILER_VAR
        ))),
    }
}

/// Read the SMTP settings from a lookup. Host, username and password are required; the port
/// defaults to [SMTP_PORT_DEFAULT].
pub fn smtp_config_from(get: EnvLookup<'_>) -> Result<SmtpConfig, TrialError> {
    let host = required(get, SMTP_HOST_VAR)?;
    let username = required(get, SMTP_USERNAME_VAR)?;
    let password = required(get, SMTP_PASSWORD_VAR)?;
    let port = match get(SMTP_PORT_VAR).map(|v| v.trim().to_string()) {
        Some(value) if !value.is_empty() => value.parse::<u16>().map_err(|_| {
            TrialError::Mailer(format!(
                "{} must be a port number (1-65535), not a non-numeric value",
                SMTP_PORT_VAR
            ))
        })?,
        _ => SMTP_PORT_DEFAULT,
    };
    if port == 0 {
        return Err(TrialError::Mailer(format!(
            "{} must be a port number (1-65535)",
            SMTP_PORT_VAR
        )));
    }
    Ok(SmtpConfig {
        host,
        port,
        username,
        password,
    })
}

/// The SMTP transport. The flow is the one RFC 5321 prescribes for a submission port:
/// greeting, EHLO, STARTTLS, EHLO again on the encrypted connection, AUTH, MAIL, RCPT, DATA,
/// QUIT. TLS is rustls against the system trust store; there is no plaintext fallback, so a
/// server that will not upgrade is a failure rather than a silent downgrade.
#[cfg(feature = "smtp")]
pub struct SmtpTransport {
    config: SmtpConfig,
}

#[cfg(feature = "smtp")]
impl SmtpTransport {
    pub fn new(config: SmtpConfig) -> Self {
        Self { config }
    }
}

#[cfg(feature = "smtp")]
impl Mailer for SmtpTransport {
    fn send(&self, email: &OutboundEmail) -> Result<(), TrialError> {
        let plain = connect(&self.config.host, self.config.port)?;
        let accepted = converse(plain, &self.config, email, |plain| {
            start_tls(&self.config.host, plain)
        })?;
        // The operator log records the recipient and the provider's own acceptance line (its
        // queue id). It never records the credential.
        eprintln!(
            "[mailer:smtp] to={} accepted by {}:{}: {}",
            email.to, self.config.host, self.config.port, accepted
        );
        Ok(())
    }
}

/// Open a TCP connection to the first address the host resolves to, with a bounded connect
/// and bounded reads and writes: a registration request must not hang on a mail server that
/// accepts the socket and then says nothing.
#[cfg(feature = "smtp")]
fn connect(host: &str, port: u16) -> Result<std::net::TcpStream, TrialError> {
    use std::net::ToSocketAddrs;
    let timeout = std::time::Duration::from_secs(15);
    let addresses: Vec<std::net::SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| TrialError::Mailer(format!("cannot resolve {}: {}", host, e)))?
        .collect();
    if addresses.is_empty() {
        return Err(TrialError::Mailer(format!(
            "{}:{} resolved to no address",
            host, port
        )));
    }
    let mut last = None;
    for address in addresses {
        match std::net::TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(timeout))
                    .and_then(|_| stream.set_write_timeout(Some(timeout)))
                    .map_err(|e| {
                        TrialError::Mailer(format!("cannot time-bound the socket: {}", e))
                    })?;
                return Ok(stream);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(TrialError::Mailer(format!(
        "cannot connect to {}:{}: {}",
        host,
        port,
        last.expect("at least one address failed")
    )))
}

/// Upgrade a connected socket with TLS, verifying the certificate against the system trust
/// store. The certificate is verified against the host name the connection was made to, so a
/// certificate for another name is refused.
#[cfg(feature = "smtp")]
fn start_tls(
    host: &str,
    plain: std::net::TcpStream,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>, TrialError> {
    let mut roots = rustls::RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    for certificate in loaded.certs {
        roots
            .add(certificate)
            .map_err(|e| TrialError::Mailer(format!("cannot load a system root: {}", e)))?;
    }
    if roots.is_empty() {
        return Err(TrialError::Mailer(
            "no system trust roots are installed; cannot verify the mail server certificate"
                .to_string(),
        ));
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| {
        TrialError::Mailer(format!("{} is not a valid TLS server name: {}", host, e))
    })?;
    let session = rustls::ClientConnection::new(Arc::new(config), name)
        .map_err(|e| TrialError::Mailer(format!("cannot start TLS to {}: {}", host, e)))?;
    Ok(rustls::StreamOwned::new(session, plain))
}

/// The client name sent in EHLO. A bare name is enough: no server in this deployment keys off
/// it, and it identifies our traffic in the provider's logs.
const SMTP_CLIENT_NAME: &str = "mw-server";

/// The longest reply line accepted. RFC 5321 caps a reply line at 512 octets; the limit here
/// is generous and exists only so a hostile or broken server cannot make us buffer forever.
const MAX_REPLY_LINE: usize = 8192;

/// Read one CRLF-terminated line, one octet at a time. Deliberately unbuffered: an SMTP
/// conversation alternates reads and writes on the same socket, and a read that ran ahead of
/// the reply would swallow the next one.
fn read_line<S: Read>(stream: &mut S) -> Result<String, TrialError> {
    let mut bytes = Vec::new();
    loop {
        let mut octet = [0u8; 1];
        let read = stream
            .read(&mut octet)
            .map_err(|e| TrialError::Mailer(format!("smtp transport error: {}", e)))?;
        if read == 0 {
            return Err(TrialError::Mailer(
                "the smtp server closed the connection mid-reply".to_string(),
            ));
        }
        if octet[0] == b'\n' {
            break;
        }
        if octet[0] != b'\r' {
            bytes.push(octet[0]);
        }
        if bytes.len() > MAX_REPLY_LINE {
            return Err(TrialError::Mailer(format!(
                "the smtp server sent a reply line longer than {} octets",
                MAX_REPLY_LINE
            )));
        }
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// Read one full reply, following continuation lines. RFC 5321 continues a reply with
/// 250-text and ends it with 250 text, so the reply is complete only at the line whose fourth
/// character is a space.
fn read_reply<S: Read>(stream: &mut S) -> Result<String, TrialError> {
    let first = read_line(stream)?;
    let mut reply = first.clone();
    while first.len() >= 4 && first.as_bytes()[3] == b'-' {
        let next = read_line(stream)?;
        reply.push('\n');
        reply.push_str(&next);
        if next.len() >= 4 && next.as_bytes()[3] == b'-' {
            continue;
        }
        break;
    }
    Ok(reply)
}

/// Send one line (CRLF terminated) and require a reply with the given status code. The line
/// itself is never included in an error, so an AUTH step cannot leak the credential into a
/// log through a failure message; the reply IS included, because a provider's rejection
/// reason is exactly what an operator needs to read.
fn expect<S: Read + Write>(
    stream: &mut S,
    line: &str,
    code: &str,
    label: &str,
) -> Result<String, TrialError> {
    stream
        .write_all(format!("{}\r\n", line).as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|e| TrialError::Mailer(format!("smtp transport error: {}", e)))?;
    let reply = read_reply(stream)?;
    if !reply.starts_with(code) {
        return Err(TrialError::Mailer(format!(
            "smtp {} was rejected: {}",
            label, reply
        )));
    }
    Ok(reply)
}

/// The AUTH mechanisms the server advertised in its EHLO reply.
fn auth_mechanisms(ehlo_reply: &str) -> Vec<String> {
    let mut mechanisms = Vec::new();
    for line in ehlo_reply.lines() {
        let upper = line.to_uppercase();
        if let Some(rest) = upper
            .strip_prefix("250-")
            .or_else(|| upper.strip_prefix("250 "))
        {
            if let Some(list) = rest.trim().strip_prefix("AUTH ") {
                mechanisms.extend(list.split_whitespace().map(str::to_string));
            }
        }
    }
    mechanisms
}

/// An RFC 5322 date for the Date: header, from a Unix timestamp. Hand-rolled rather than
/// pulled from a crate: the whole conversion is the civil-from-days algorithm, and a
/// dependency for it is not a trade worth making here.
fn rfc5322_date(unix_seconds: i64) -> String {
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = unix_seconds.div_euclid(86_400);
    let seconds_of_day = unix_seconds.rem_euclid(86_400);
    let weekday = WEEKDAYS[days.rem_euclid(7) as usize];
    // Howard Hinnant's civil_from_days: shift the epoch to 0000-03-01 so leap days land at
    // the end of the year and the month arithmetic is linear.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} +0000",
        weekday,
        day,
        MONTHS[(month - 1) as usize],
        year,
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60
    )
}

/// A header value may not contain a line break. Without this check a recipient or a subject
/// carrying CRLF would inject extra headers (a Bcc, say) into the message we hand the
/// provider.
fn header_value<'a>(value: &'a str, label: &str) -> Result<&'a str, TrialError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(TrialError::Mailer(format!(
            "refusing to send: the {} contains a line break",
            label
        )));
    }
    Ok(value)
}

/// The complete DATA payload: headers, a blank line, the body, and the terminating line.
/// Every line ends CRLF and a body line that begins with a period is doubled, so a body can
/// never be mistaken for the end of the message.
fn message_payload(
    email: &OutboundEmail,
    date: &str,
    message_id: &str,
) -> Result<String, TrialError> {
    let from = header_value(&email.from, "sender")?;
    let to = header_value(&email.to, "recipient")?;
    let subject = header_value(&email.subject, "subject")?;
    if from.is_empty() || to.is_empty() {
        return Err(TrialError::Mailer(
            "refusing to send: the sender and the recipient must both be set".to_string(),
        ));
    }
    let mut message = String::new();
    for (name, value) in [
        ("From", from),
        ("To", to),
        ("Subject", subject),
        ("Date", date),
        ("Message-ID", message_id),
        ("MIME-Version", "1.0"),
        ("Content-Type", "text/plain; charset=utf-8"),
        ("Content-Transfer-Encoding", "8bit"),
    ] {
        message.push_str(&format!("{}: {}\r\n", name, value));
    }
    message.push_str("\r\n");
    for line in email.body_plain.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with('.') {
            message.push('.');
        }
        message.push_str(line);
        message.push_str("\r\n");
    }
    message.push_str(".\r\n");
    Ok(message)
}

/// The Message-ID for one message: the domain of the sender and a random local part, so two
/// messages never share an id and the id names the domain that sent it.
fn message_id(from: &str, nonce: u64) -> String {
    let domain = from
        .split_once('@')
        .map(|(_, d)| d)
        .unwrap_or("modelwrite.org");
    format!("<{:016x}.{:016x}@{}>", nonce, std::process::id(), domain)
}

/// Run one SMTP delivery over an already-connected stream, up to and including QUIT, and
/// return the provider's final acceptance line verbatim.
///
/// This is a free function over Read + Write rather than a method on the TLS socket so the
/// protocol can be tested against a fake server: the tests drive the same code the transport
/// does and inject the encrypted half, with no network and no credential anywhere near them.
fn converse<S, T, F>(
    mut plain: S,
    config: &SmtpConfig,
    email: &OutboundEmail,
    upgrade: F,
) -> Result<String, TrialError>
where
    S: Read + Write,
    T: Read + Write,
    F: FnOnce(S) -> Result<T, TrialError>,
{
    let greeting = read_reply(&mut plain)?;
    if !greeting.starts_with("220") {
        return Err(TrialError::Mailer(format!(
            "smtp greeting was rejected: {}",
            greeting
        )));
    }
    expect(
        &mut plain,
        &format!("EHLO {}", SMTP_CLIENT_NAME),
        "250",
        "EHLO",
    )?;
    expect(&mut plain, "STARTTLS", "220", "STARTTLS")?;
    let mut tls = upgrade(plain)?;
    let ehlo = expect(
        &mut tls,
        &format!("EHLO {}", SMTP_CLIENT_NAME),
        "250",
        "EHLO after STARTTLS",
    )?;

    let mechanisms = auth_mechanisms(&ehlo);
    if mechanisms.iter().any(|m| m == "LOGIN") {
        expect(&mut tls, "AUTH LOGIN", "334", "AUTH LOGIN")?;
        let user = base64_encode(config.username.as_bytes());
        expect(&mut tls, &user, "334", "AUTH username")?;
        let password = base64_encode(config.password.as_bytes());
        expect(&mut tls, &password, "235", "AUTH password")?;
    } else if mechanisms.iter().any(|m| m == "PLAIN") {
        let mut blob = vec![0u8];
        blob.extend_from_slice(config.username.as_bytes());
        blob.push(0u8);
        blob.extend_from_slice(config.password.as_bytes());
        let line = format!("AUTH PLAIN {}", base64_encode(&blob));
        expect(&mut tls, &line, "235", "AUTH PLAIN")?;
    } else {
        return Err(TrialError::Mailer(format!(
            "the smtp server offers no supported AUTH mechanism (offered: {})",
            mechanisms.join(" ")
        )));
    }

    let sender = header_value(&email.from, "sender")?;
    let recipient = header_value(&email.to, "recipient")?;
    expect(
        &mut tls,
        &format!("MAIL FROM:<{}>", sender),
        "250",
        "MAIL FROM",
    )?;
    expect(
        &mut tls,
        &format!("RCPT TO:<{}>", recipient),
        "250",
        "RCPT TO",
    )?;
    expect(&mut tls, "DATA", "354", "DATA")?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let payload = message_payload(
        email,
        &rfc5322_date(now),
        &message_id(&email.from, rand::random::<u64>()),
    )?;
    tls.write_all(payload.as_bytes())
        .and_then(|_| tls.flush())
        .map_err(|e| TrialError::Mailer(format!("smtp transport error: {}", e)))?;
    let accepted = read_reply(&mut tls)?;
    if !accepted.starts_with("250") {
        return Err(TrialError::Mailer(format!(
            "smtp DATA was rejected: {}",
            accepted
        )));
    }
    // QUIT is best-effort: the message is already accepted, and a broken goodbye must not
    // turn a delivered email into a failed registration.
    let _ = expect(&mut tls, "QUIT", "221", "QUIT");
    Ok(accepted)
}

/// Base64, the wire encoding AUTH requires.
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
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
/// domain. The spec requires the sender to be a real address on that domain. It is also the
/// SMTP envelope sender, so with MW_MAILER=smtp this must be the address the mail provider
/// has confirmed as a sender.
pub fn sender_from_env() -> String {
    std::env::var(MAIL_FROM_VAR).unwrap_or_else(|_| "no-reply@modelwrite.org".to_string())
}

/// Build the transport named by MW_MAILER from the process environment.
pub fn mailer_from_env() -> Arc<dyn Mailer> {
    transport_from(&|name| std::env::var(name).ok())
}

/// The SMTP transport, or the honest failure when this build has none compiled in.
#[cfg(feature = "smtp")]
fn smtp_transport(config: SmtpConfig) -> Arc<dyn Mailer> {
    Arc::new(SmtpTransport::new(config))
}

#[cfg(not(feature = "smtp"))]
fn smtp_transport(_config: SmtpConfig) -> Arc<dyn Mailer> {
    Arc::new(FailingTransport::new(
        "this build has no SMTP transport: rebuild the server with the smtp feature".to_string(),
    ))
}

/// Choose the transport from a lookup: MW_MAILER=console (default) selects the console
/// transport, file selects the file transport at MW_MAILER_FILE, and smtp selects the SMTP
/// transport configured by MW_MAIL_SMTP_HOST, MW_MAIL_SMTP_PORT (default 587),
/// MW_MAIL_SMTP_USERNAME and MW_MAIL_SMTP_PASSWORD. An SMTP selection that is missing its
/// configuration yields a transport that FAILS every send with the reason: the deployment
/// asked for real delivery, so writing the code to the log instead would be a lie the person
/// waiting for the email pays for.
fn transport_from(get: EnvLookup<'_>) -> Arc<dyn Mailer> {
    match get(MAILER_VAR).as_deref() {
        Some("file") => {
            let path = get(MAILER_FILE_VAR)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("mailer.log"));
            Arc::new(FileTransport::new(path))
        }
        Some("smtp") => match smtp_config_from(get) {
            Ok(config) => smtp_transport(config),
            Err(e) => Arc::new(FailingTransport::new(e.to_string())),
        },
        _ => Arc::new(ConsoleTransport),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SmtpConfig {
        SmtpConfig {
            host: "smtp.example.test".to_string(),
            port: 587,
            username: "server-token-user".to_string(),
            password: "server-token-secret".to_string(),
        }
    }

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

    /// A scripted server: it hands out canned replies in order and records everything the
    /// client wrote. It is the whole point of the converse seam - the tests below exercise
    /// the real protocol code with no socket, no network and no real credential.
    struct FakeServer {
        replies: std::collections::VecDeque<String>,
        pending: Vec<u8>,
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl FakeServer {
        fn new<I, S>(replies: I, written: Arc<Mutex<Vec<u8>>>) -> Self
        where
            I: IntoIterator<Item = S>,
            S: Into<String>,
        {
            Self {
                replies: replies.into_iter().map(Into::into).collect(),
                pending: Vec::new(),
                written,
            }
        }
    }

    impl Read for FakeServer {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pending.is_empty() {
                match self.replies.pop_front() {
                    Some(reply) => self.pending = format!("{}\r\n", reply).into_bytes(),
                    None => return Ok(0),
                }
            }
            let take = buf.len().min(self.pending.len());
            buf[..take].copy_from_slice(&self.pending[..take]);
            self.pending.drain(..take);
            Ok(take)
        }
    }

    impl Write for FakeServer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn written(handle: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(handle.lock().unwrap().clone()).unwrap()
    }

    /// The scripted exchange after STARTTLS: EHLO with AUTH LOGIN, a successful LOGIN, the
    /// envelope, DATA and the provider's acceptance.
    fn accepting_replies() -> Vec<&'static str> {
        vec![
            "250-smtp.example.test Hello",
            "250-SIZE 10240000",
            "250 AUTH LOGIN PLAIN",
            "334 VXNlcm5hbWU6",
            "334 UGFzc3dvcmQ6",
            "235 2.7.0 Authentication successful",
            "250 2.1.0 Sender OK",
            "250 2.1.5 Recipient OK",
            "354 End data with <CR><LF>.<CR><LF>",
            "250 2.0.0 Ok: queued as QUEUE-ID-123",
            "221 2.0.0 Bye",
        ]
    }

    fn deliver(
        email: &OutboundEmail,
        post_tls: Vec<&'static str>,
        post_written: Arc<Mutex<Vec<u8>>>,
    ) -> Result<String, TrialError> {
        let pre_written = Arc::new(Mutex::new(Vec::new()));
        let plain = FakeServer::new(
            vec![
                "220 smtp.example.test ESMTP ready",
                "250-smtp.example.test Hello",
                "250 AUTH LOGIN PLAIN",
                "220 2.0.0 Ready to start TLS",
            ],
            pre_written,
        );
        let tls = FakeServer::new(post_tls, post_written);
        converse(plain, &config(), email, move |_plain| Ok(tls))
    }

    #[test]
    fn smtp_conversation_authenticates_and_delivers_the_message() {
        let post_written = Arc::new(Mutex::new(Vec::new()));
        let email = login_code_email("alex@axoquant.com", "alex@axoquant.com", "424242");
        let accepted = deliver(&email, accepting_replies(), post_written.clone()).unwrap();
        assert!(
            accepted.contains("queued as QUEUE-ID-123"),
            "the acceptance line is returned verbatim: {}",
            accepted
        );
        let sent = written(&post_written);
        assert!(sent.starts_with("EHLO mw-server\r\n"), "{}", sent);
        assert!(sent.contains("AUTH LOGIN\r\n"));
        assert!(sent.contains(&format!("{}\r\n", base64_encode(b"server-token-user"))));
        assert!(
            sent.contains("MAIL FROM:<alex@axoquant.com>\r\n"),
            "{}",
            sent
        );
        assert!(sent.contains("RCPT TO:<alex@axoquant.com>\r\n"));
        assert!(sent.contains("DATA\r\n"));
        assert!(sent.contains("Subject: Your Modelwrite login code\r\n"));
        assert!(sent.contains("424242"));
        assert!(sent.trim_end().ends_with("QUIT"));
    }

    #[test]
    fn smtp_credentials_are_never_in_an_error_message() {
        let post_written = Arc::new(Mutex::new(Vec::new()));
        let mut replies = accepting_replies();
        replies[4] = "535 5.7.8 Authentication credentials invalid";
        let email = login_code_email("alex@axoquant.com", "alex@axoquant.com", "424242");
        let error = deliver(&email, replies, post_written).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("535 5.7.8 Authentication credentials invalid"),
            "the provider line is reported verbatim: {}",
            message
        );
        assert!(!message.contains("server-token-secret"), "{}", message);
        assert!(!message.contains(&base64_encode(b"server-token-secret")));
    }

    #[test]
    fn smtp_reports_a_rejected_data_step_verbatim() {
        let post_written = Arc::new(Mutex::new(Vec::new()));
        let mut replies = accepting_replies();
        replies[9] = "550 5.7.1 Sender signature not confirmed";
        let email = login_code_email("alex@axoquant.com", "alex@axoquant.com", "424242");
        let error = deliver(&email, replies, post_written).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("550 5.7.1 Sender signature not confirmed"),
            "{}",
            error
        );
    }

    #[test]
    fn smtp_prefers_plain_when_the_server_does_not_offer_login() {
        let post_written = Arc::new(Mutex::new(Vec::new()));
        let mut replies = accepting_replies();
        replies[2] = "250 AUTH PLAIN";
        // LOGIN's two prompts disappear, so the rest of the script shifts up by two.
        replies.remove(3);
        replies.remove(3);
        let email = login_code_email("alex@axoquant.com", "alex@axoquant.com", "424242");
        let accepted = deliver(&email, replies, post_written.clone()).unwrap();
        assert!(accepted.starts_with("250"));
        let sent = written(&post_written);
        assert!(sent.contains("AUTH PLAIN "), "{}", sent);
        assert!(!sent.contains("AUTH LOGIN"));
    }

    #[test]
    fn smtp_refuses_a_server_with_no_supported_auth_mechanism() {
        let post_written = Arc::new(Mutex::new(Vec::new()));
        let mut replies = accepting_replies();
        replies[2] = "250 AUTH CRAM-MD5";
        let email = login_code_email("alex@axoquant.com", "alex@axoquant.com", "424242");
        let error = deliver(&email, replies, post_written).unwrap_err();
        assert!(error.to_string().contains("CRAM-MD5"), "{}", error);
        assert!(error.to_string().contains("no supported AUTH"));
    }

    #[test]
    fn smtp_refuses_a_plaintext_fallback_when_starttls_is_refused() {
        let pre_written = Arc::new(Mutex::new(Vec::new()));
        let email = login_code_email("alex@axoquant.com", "alex@axoquant.com", "424242");
        let plain = FakeServer::new(
            vec![
                "220 smtp.example.test ESMTP ready",
                "250-smtp.example.test Hello",
                "250 AUTH LOGIN PLAIN",
                "502 5.5.1 STARTTLS not supported",
            ],
            pre_written.clone(),
        );
        let error = converse(
            plain,
            &config(),
            &email,
            |_plain| -> Result<FakeServer, TrialError> {
                unreachable!("a refused STARTTLS must not reach the upgrade")
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("502 5.5.1 STARTTLS not supported"),
            "{}",
            error
        );
        // The TLS upgrade never happened, so no credential was offered in the clear.
        let sent = written(&pre_written);
        assert!(sent.contains("STARTTLS\r\n"));
        assert!(!sent.contains("AUTH"));
        assert!(!sent.contains(&base64_encode(b"server-token-secret")));
    }

    #[test]
    fn smtp_dot_stuffs_the_body_and_terminates_it() {
        let email = OutboundEmail {
            from: "alex@axoquant.com".to_string(),
            to: "alex@axoquant.com".to_string(),
            subject: "Subject".to_string(),
            body_plain: "first\n.\nthird".to_string(),
        };
        let payload =
            message_payload(&email, "Mon, 21 Sep 2026 22:52:25 +0000", "<id@x.test>").unwrap();
        assert!(
            payload.contains("\r\nfirst\r\n..\r\nthird\r\n.\r\n"),
            "{}",
            payload
        );
        assert_eq!(payload.matches("\r\n.\r\n").count(), 1);
        assert!(payload.contains("Date: Mon, 21 Sep 2026 22:52:25 +0000\r\n"));
    }

    #[test]
    fn smtp_refuses_header_injection() {
        let email = OutboundEmail {
            from: "alex@axoquant.com".to_string(),
            to: "alex@axoquant.com\r\nBcc: victim@example.com".to_string(),
            subject: "Subject".to_string(),
            body_plain: "body".to_string(),
        };
        let error = message_payload(&email, "now", "<id@x.test>").unwrap_err();
        assert!(error.to_string().contains("line break"), "{}", error);
    }

    #[test]
    fn smtp_config_reads_the_documented_variables_and_defaults_the_port() {
        let get = |name: &str| -> Option<String> {
            match name {
                "MW_MAIL_SMTP_HOST" => Some("smtp.postmarkapp.com".to_string()),
                "MW_MAIL_SMTP_USERNAME" => Some("username-is-the-token".to_string()),
                "MW_MAIL_SMTP_PASSWORD" => Some("password-is-the-token".to_string()),
                _ => None,
            }
        };
        let config = smtp_config_from(&get).unwrap();
        assert_eq!(config.host, "smtp.postmarkapp.com");
        assert_eq!(config.port, 587);
        assert_eq!(config.username, "username-is-the-token");
        assert_eq!(config.password, "password-is-the-token");

        let with_port = |name: &str| -> Option<String> {
            if name == "MW_MAIL_SMTP_PORT" {
                Some("2525".to_string())
            } else {
                get(name)
            }
        };
        assert_eq!(smtp_config_from(&with_port).unwrap().port, 2525);
    }

    #[test]
    fn smtp_config_names_every_missing_variable_without_leaking_a_value() {
        let empty = |_name: &str| -> Option<String> { None };
        let error = smtp_config_from(&empty).unwrap_err().to_string();
        assert!(error.contains("MW_MAIL_SMTP_HOST"), "{}", error);

        let no_password = |name: &str| -> Option<String> {
            match name {
                "MW_MAIL_SMTP_HOST" => Some("smtp.postmarkapp.com".to_string()),
                "MW_MAIL_SMTP_USERNAME" => Some("token".to_string()),
                _ => None,
            }
        };
        let error = smtp_config_from(&no_password).unwrap_err().to_string();
        assert!(error.contains("MW_MAIL_SMTP_PASSWORD"), "{}", error);
        assert!(!error.contains("token"), "{}", error);

        let bad_port = |name: &str| -> Option<String> {
            match name {
                "MW_MAIL_SMTP_PORT" => Some("not-a-port".to_string()),
                "MW_MAIL_SMTP_HOST" => Some("smtp.postmarkapp.com".to_string()),
                "MW_MAIL_SMTP_USERNAME" => Some("token".to_string()),
                "MW_MAIL_SMTP_PASSWORD" => Some("secret".to_string()),
                _ => None,
            }
        };
        let error = smtp_config_from(&bad_port).unwrap_err().to_string();
        assert!(error.contains("MW_MAIL_SMTP_PORT"), "{}", error);
        assert!(!error.contains("secret"), "{}", error);
    }

    #[test]
    fn debug_never_renders_the_password() {
        let rendered = format!("{:?}", config());
        assert!(rendered.contains("smtp.example.test"));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("server-token-secret"));
    }

    #[test]
    fn smtp_selection_without_configuration_fails_the_send_instead_of_logging_the_code() {
        let get = |name: &str| -> Option<String> {
            if name == "MW_MAILER" {
                Some("smtp".to_string())
            } else {
                None
            }
        };
        let mailer = transport_from(&get);
        let error = mailer
            .send(&login_code_email(
                "a@example.com",
                "b@example.com",
                "123456",
            ))
            .unwrap_err()
            .to_string();
        assert!(error.contains("MW_MAIL_SMTP_HOST"), "{}", error);
    }

    #[test]
    fn an_unknown_transport_still_falls_back_to_the_console() {
        let get = |name: &str| -> Option<String> {
            if name == "MW_MAILER" {
                Some("nowhere".to_string())
            } else {
                None
            }
        };
        transport_from(&get)
            .send(&login_code_email(
                "a@example.com",
                "b@example.com",
                "123456",
            ))
            .unwrap();
    }

    #[test]
    fn rfc5322_date_renders_known_instants() {
        assert_eq!(rfc5322_date(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        assert_eq!(
            rfc5322_date(1_000_000_000),
            "Sun, 09 Sep 2001 01:46:40 +0000"
        );
        // A leap day, and an ordinary instant later in the same decade.
        assert_eq!(
            rfc5322_date(1_709_164_800),
            "Thu, 29 Feb 2024 00:00:00 +0000"
        );
        assert_eq!(
            rfc5322_date(1_790_031_145),
            "Mon, 21 Sep 2026 22:52:25 +0000"
        );
    }
}
