//! Outbound mail, behind a trait, with a local fallback (`docs/48` Profile 1).
//!
//! # STARTTLS is required, never negotiated down (D-046)
//!
//! `docs/14` opened D-046 because nothing specified what happens on the way
//! *out*: "Silently falling back to cleartext is the failure mode to design
//! out, and which way to fail is a security decision rather than a
//! client-library default to inherit." It was **Accepted** as "STARTTLS +
//! certificate/hostname verification".
//!
//! That is not a setting here, it is the constructor. [`SmtpMailer::connect`]
//! builds a `starttls_relay` transport, which refuses to send at all when the
//! relay does not offer `STARTTLS`, and verifies the relay's certificate chain
//! and hostname against the platform trust store. There is no flag that turns
//! either off, because what crosses that connection is a password-reset link
//! and `TF_SMTP_PASS`.
//!
//! # An empty host disables email, and that is a supported configuration
//!
//! `docs/48` §Configuration: "`TF_SMTP_HOST/PORT/USER/PASS/FROM` — empty host
//! disables email". [`LoggingMailer`] is what an empty host selects. It is not
//! a stub awaiting an implementation: a single-node deployment with no relay is
//! Profile 1, and `docs/29` §Channels makes in-app the system of record — "every
//! notification lands there regardless of other channel settings, so nothing is
//! ever *only* in an email someone deleted".
//!
//! # The body never reaches a log
//!
//! A reset link **is** the credential. [`Message`] therefore keeps its body
//! private and prints `<redacted>` from `Debug`, so the value cannot reach a log
//! through the one path that needs no author to be careless — a `tracing` field
//! or a `{:?}` in an error. `docs/46` §Redaction guard is the same mechanism;
//! it is re-implemented in miniature here rather than imported, because
//! depending on `casual-task-observability` would add an edge to the dependency
//! DAG `docs/19` fixes, and ADR-003 makes that an ADR rather than a convenience.
//!
//! # Why the message is composed here rather than by `lettre`'s builder
//!
//! `lettre`'s `builder` feature pulls `quoted_printable`, which is `0BSD` — not
//! on `deny.toml`'s allow-list, so `cargo deny check licenses` fails. Widening
//! that list is a licensing policy decision, not a build fix; D-050 is the
//! precedent, where the same situation turned a database TLS feature *off*
//! rather than adding `CDLA-Permissive-2.0`.
//!
//! So `lettre` supplies the protocol — STARTTLS, authentication, dot-stuffing,
//! the connection — and [`format_rfc5322`] supplies the twelve lines of headers
//! this system's only outbound mail needs. **The cost is stated:** there is no
//! MIME encoder here, so a subject must be ASCII without control characters and
//! [`SmtpConfig::from`] must be a bare address with no display name. Both are
//! refused rather than mangled, and both are asserted by tests.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

/// How long a send may take before it is abandoned.
///
/// `AGENTS.md` §Engineering priorities: "every external call timed". A relay
/// that accepts the TCP connection and then stops talking would otherwise hold
/// the task forever.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(15);

/// The display name on the `From` header.
///
/// A constant rather than configuration: rendering an operator-supplied name
/// safely means RFC 2047 encoding, and the encoder is the dependency this
/// module exists without — see the module docs.
pub const FROM_DISPLAY_NAME: &str = "TaskForge";

/// Why a message was not delivered.
///
/// Deliberately carries no body: an error is the value most likely to be logged
/// verbatim, and the body of a reset mail is a working credential.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MailError {
    /// The recipient or sender address is not one this transport can use.
    #[error("the address is not valid")]
    Address,
    /// A header value carried a newline or a non-ASCII character.
    ///
    /// Separate from [`Self::Address`] because it is the **header-injection**
    /// refusal: a subject containing `\r\n` would otherwise let its author
    /// append headers, and `Bcc:` is one of them.
    #[error("a header value is not a single line of ASCII")]
    Header,
    /// The relay refused, was unreachable, or did not offer STARTTLS.
    #[error("the relay did not accept the message: {0}")]
    Transport(String),
    /// The transport could not be built from the configuration given.
    #[error("the mail transport cannot be configured: {0}")]
    Configuration(String),
}

/// One plain-text message, ready to send.
///
/// The body is **private**. See the module docs: it holds a reset link, and a
/// derived `Debug` would put that link into any log line that prints the
/// struct.
#[derive(Clone, PartialEq, Eq)]
pub struct Message {
    to: String,
    subject: String,
    body: String,
}

impl Message {
    /// Compose a plain-text message.
    #[must_use]
    pub fn new(to: impl Into<String>, subject: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            to: to.into(),
            subject: subject.into(),
            body: body.into(),
        }
    }

    /// The recipient address.
    #[must_use]
    pub fn to(&self) -> &str {
        &self.to
    }

    /// The subject line.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The body. Named `expose_body` rather than `body` so that reaching for it
    /// is a visible decision at the call site, as `Redacted::expose` is in
    /// `docs/46`.
    #[must_use]
    pub fn expose_body(&self) -> &str {
        &self.body
    }
}

impl fmt::Debug for Message {
    /// Prints the recipient and `<redacted>` for everything else.
    ///
    /// The subject used to be printed, with a note that it was "safe by
    /// construction for the mail this system sends today" and that the day a
    /// notification subject existed was the day to revisit it. That day is
    /// C-016: `docs/29` §Email content makes the subject `[WR-125] Task
    /// title`, and a task title is customer content that `docs/46` forbids
    /// logging at any level. So the subject is redacted here, and `to` stays —
    /// an address is who the mail went to, which is what an operator debugging
    /// delivery needs and is already in the SMTP envelope.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Message")
            .field("to", &self.to)
            .field("subject", &"<redacted>")
            .field("body", &"<redacted>")
            .finish()
    }
}

/// Somewhere to send mail.
///
/// # Why the future is boxed
///
/// The API holds this as `Arc<dyn Mailer>` so that the SMTP and no-op paths are
/// one type at every call site — which is what makes "email is disabled" a
/// configuration rather than a branch in a handler. An `async fn` in a trait is
/// not dyn-compatible on the pinned toolchain, so the future is boxed here
/// rather than each caller learning which implementation it holds.
pub trait Mailer: Send + Sync + fmt::Debug {
    /// Deliver one message.
    ///
    /// # Errors
    ///
    /// [`MailError`] when the relay refuses, is unreachable, or does not offer
    /// STARTTLS.
    fn send<'a>(
        &'a self,
        message: &'a Message,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>>;
}

/// What an SMTP relay needs to be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    /// `TF_SMTP_HOST`. **Empty means email is disabled** — see the module docs.
    pub host: String,
    /// `TF_SMTP_PORT`.
    pub port: u16,
    /// `TF_SMTP_USER`. Empty means the relay wants no authentication, which is
    /// normal for a relay reachable only on a private network.
    pub user: String,
    /// `TF_SMTP_PASS`.
    pub password: String,
    /// `TF_SMTP_FROM` — a **bare** address, `noreply@example.com`. A display
    /// name is refused rather than mangled; see the module docs.
    pub from: String,
}

impl SmtpConfig {
    /// The default submission port. STARTTLS is negotiated on 587; 465 is
    /// implicit TLS from the first byte and speaks a different opening, so a
    /// deployment pointing this at 465 is misconfigured rather than unusual.
    pub const DEFAULT_PORT: u16 = 587;

    /// Whether this configuration selects a real relay.
    ///
    /// `docs/48`: an empty host disables email. Trimmed, because a variable set
    /// to a space in a compose file is an operator who meant to leave it empty.
    #[must_use]
    pub fn enabled(&self) -> bool {
        !self.host.trim().is_empty()
    }
}

/// Render one RFC 5322 message.
///
/// `date` and `message_id` are arguments rather than read from the clock and
/// the entropy source inside, so the output is a pure function of its inputs
/// and a test can assert the exact bytes. A composer that reads a clock can
/// only be tested for substrings.
///
/// # Errors
///
/// [`MailError::Header`] if the subject carries a control character, or the
/// message id is not a single ASCII line. That is the header-injection guard:
/// `\r\n` in a header appends headers.
///
/// A non-ASCII subject is **encoded**, not refused — `crate::header` carries the
/// RFC 2047 encoder, because `docs/29` puts a task title in a notification
/// subject and titles are customer content in whatever language the customer
/// writes.
pub fn format_rfc5322(
    from: &Address,
    to: &Address,
    subject: &str,
    body: &str,
    date: OffsetDateTime,
    message_id: &str,
) -> Result<Vec<u8>, MailError> {
    let subject = crate::header::encode_subject(subject).ok_or(MailError::Header)?;
    if !crate::header::is_safe_line(message_id) {
        return Err(MailError::Header);
    }

    // Unwrapping is not an option here — a failure would send a message with no
    // Date header — but Rfc2822 formatting of a real OffsetDateTime cannot
    // fail, so the fallback is unreachable rather than approximate.
    let date = date
        .format(&Rfc2822)
        .map_err(|error| MailError::Configuration(error.to_string()))?;

    // CRLF everywhere, including inside the body: RFC 5321 §2.3.8 makes CRLF
    // the line terminator on the wire, and a bare LF is what makes a message
    // arrive with its lines run together on some relays and rejected by others.
    let mut out = String::new();
    for (name, value) in [
        ("Date", date.as_str()),
        ("From", &format!("{FROM_DISPLAY_NAME} <{from}>")),
        ("To", to.as_ref()),
        ("Subject", subject.as_str()),
        ("Message-ID", &format!("<{message_id}>")),
        ("MIME-Version", "1.0"),
        ("Content-Type", "text/plain; charset=utf-8"),
        // 8bit, not 7bit: the declared charset is UTF-8 and the body is
        // caller-supplied, so claiming 7bit would be a lie the moment anyone
        // sends an accented character. Every relay this product supports
        // announces 8BITMIME.
        ("Content-Transfer-Encoding", "8bit"),
    ] {
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out.push_str(&crlf(body));

    Ok(out.into_bytes())
}

/// Normalise line endings to CRLF without doubling the ones already correct.
fn crlf(body: &str) -> String {
    body.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Sends over SMTP with STARTTLS required and the relay's certificate verified.
pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Address,
}

impl fmt::Debug for SmtpMailer {
    /// Names the sender and nothing else. `AsyncSmtpTransport` holds the relay
    /// credentials, and a derived `Debug` on this struct is one `{:?}` away
    /// from `TF_SMTP_PASS` in a log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpMailer")
            .field("from", &self.from.to_string())
            .finish_non_exhaustive()
    }
}

impl SmtpMailer {
    /// Build the transport.
    ///
    /// **`starttls_relay`, not `builder_dangerous`.** The first upgrades the
    /// connection and refuses to continue if the relay will not; the second
    /// sends in cleartext, which is exactly what D-046 rules out. Certificate
    /// and hostname verification are the transport's defaults and are not
    /// weakened here — there is no configuration key that could.
    ///
    /// # Errors
    ///
    /// [`MailError::Configuration`] if `TF_SMTP_FROM` is not a bare address or
    /// the transport cannot be built. Both are startup failures — `docs/48`:
    /// "Startup validation fails fast and specifically."
    pub fn connect(config: &SmtpConfig) -> Result<Self, MailError> {
        let from: Address = config.from.trim().parse().map_err(|_| {
            MailError::Configuration(format!(
                "TF_SMTP_FROM must be a bare address such as noreply@example.com, not {:?}",
                config.from
            ))
        })?;

        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(config.host.trim())
            .map_err(|error| MailError::Configuration(error.to_string()))?
            .port(config.port)
            .timeout(Some(SEND_TIMEOUT));

        // Only when a user is configured. Offering empty credentials to a relay
        // that wants none turns a working send into an authentication failure.
        if !config.user.is_empty() {
            builder = builder.credentials(Credentials::new(
                config.user.clone(),
                config.password.clone(),
            ));
        }

        Ok(Self {
            transport: builder.build(),
            from,
        })
    }
}

impl Mailer for SmtpMailer {
    fn send<'a>(
        &'a self,
        message: &'a Message,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>> {
        Box::pin(async move {
            // Parsed, not interpolated. `Address` rejects the newline that
            // would otherwise let a recipient address carry its own headers,
            // and it does so before the value reaches the envelope.
            let to: Address = message
                .to()
                .trim()
                .parse()
                .map_err(|_| MailError::Address)?;
            let envelope = Envelope::new(Some(self.from.clone()), vec![to.clone()])
                .map_err(|_| MailError::Address)?;

            let message_id = format!("{}@{}", uuid::Uuid::now_v7(), self.from.domain());
            let raw = format_rfc5322(
                &self.from,
                &to,
                message.subject(),
                message.expose_body(),
                OffsetDateTime::now_utc(),
                &message_id,
            )?;

            self.transport
                .send_raw(&envelope, &raw)
                .await
                .map_err(|error| MailError::Transport(error.to_string()))?;
            Ok(())
        })
    }
}

/// The mailer an empty `TF_SMTP_HOST` selects: records that a message would
/// have been sent, and sends nothing.
///
/// It logs the recipient and the subject at `info`, and **never the body**.
/// That is the whole difference between a useful operational record and a
/// credential in a log file — `docs/46`: never log a credential, at any level.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoggingMailer;

impl Mailer for LoggingMailer {
    fn send<'a>(
        &'a self,
        message: &'a Message,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>> {
        Box::pin(async move {
            // The subject is NOT logged: since C-016 it carries a task title,
            // and docs/46 forbids customer content in a log line at any level.
            // The recipient is enough to answer "did this person get mail".
            tracing::info!(
                to = message.to(),
                "email is disabled (TF_SMTP_HOST is empty); the message was not sent"
            );
            Ok(())
        })
    }
}

/// The mailer this configuration selects.
///
/// One function rather than a branch at the call site: "is email configured" is
/// answered once, at startup, and every handler afterwards holds a [`Mailer`]
/// and does not ask.
///
/// # Errors
///
/// [`MailError::Configuration`] when a relay *is* configured and cannot be
/// built. An empty host never fails — it is a supported deployment.
pub fn from_config(config: &SmtpConfig) -> Result<std::sync::Arc<dyn Mailer>, MailError> {
    if config.enabled() {
        Ok(std::sync::Arc::new(SmtpMailer::connect(config)?))
    } else {
        Ok(std::sync::Arc::new(LoggingMailer))
    }
}

#[cfg(test)]
#[path = "mail_tests.rs"]
mod tests;
