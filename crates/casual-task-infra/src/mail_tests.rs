use super::*;

fn config() -> SmtpConfig {
    SmtpConfig {
        host: "smtp.example.com".to_owned(),
        port: SmtpConfig::DEFAULT_PORT,
        user: "postmaster".to_owned(),
        password: "hunter2".to_owned(),
        from: "noreply@example.com".to_owned(),
    }
}

fn address(value: &str) -> Address {
    value.parse().expect("a valid address")
}

fn rendered(subject: &str, body: &str) -> Result<String, MailError> {
    format_rfc5322(
        &address("noreply@example.com"),
        &address("user@example.com"),
        subject,
        body,
        OffsetDateTime::UNIX_EPOCH,
        "0000@example.com",
    )
    .map(|bytes| String::from_utf8(bytes).expect("ascii and utf-8"))
}

#[test]
fn a_message_body_does_not_survive_debug_formatting() {
    // The reason this impl is hand-written. A reset link IS the credential,
    // and a derived Debug puts it in any log line that prints the struct.
    let message = Message::new(
        "user@example.com",
        "Reset your password",
        "https://tasks.example.com/reset?token=abcdef.0123456789",
    );
    let rendered = format!("{message:?}");
    assert!(!rendered.contains("abcdef"), "{rendered}");
    assert!(!rendered.contains("token="), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
    // The envelope is still legible, or the impl would be useless for
    // debugging and someone would print the fields individually instead.
    assert!(rendered.contains("user@example.com"), "{rendered}");
}

#[test]
fn the_smtp_password_does_not_survive_debug_formatting() {
    let mailer = SmtpMailer::connect(&config()).expect("configures");
    let rendered = format!("{mailer:?}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(!rendered.contains("postmaster"), "{rendered}");
}

#[test]
fn an_empty_host_disables_email() {
    // docs/48 §Configuration, stated as a property. Profile 1 has no relay.
    for host in ["", "   "] {
        let config = SmtpConfig {
            host: host.to_owned(),
            ..config()
        };
        assert!(!config.enabled(), "{host:?} was treated as a relay");
        let mailer = from_config(&config).expect("the no-op never fails");
        assert_eq!(format!("{mailer:?}"), "LoggingMailer");
    }
}

#[test]
fn a_configured_host_selects_smtp() {
    // The companion to the test above: a selector that always returned the
    // no-op would satisfy it and silently disable email everywhere.
    let mailer = from_config(&config()).expect("configures");
    assert!(format!("{mailer:?}").starts_with("SmtpMailer"));
}

#[test]
fn a_sender_that_is_not_a_bare_address_refuses_to_configure() {
    // docs/48: "A misconfigured deployment must not start." There is no
    // RFC 2047 encoder here, so a display name cannot be rendered safely —
    // it is refused at startup rather than mangled in every email.
    for from in ["not an address", "TaskForge <noreply@example.com>", ""] {
        assert!(
            matches!(
                SmtpMailer::connect(&SmtpConfig {
                    from: from.to_owned(),
                    ..config()
                }),
                Err(MailError::Configuration(_))
            ),
            "accepted {from:?}"
        );
    }
}

#[test]
fn the_rendered_message_is_exactly_these_bytes() {
    // Asserted whole rather than by substring: a header composer is a
    // string concatenation, and the failures that matter — a missing blank
    // line before the body, LF instead of CRLF — are invisible to a
    // `contains` check.
    let message = rendered("Reset your password", "Open this link:\nhttps://x/y").expect("ok");
    assert_eq!(
        message,
        "Date: Thu, 01 Jan 1970 00:00:00 +0000\r\n\
             From: TaskForge <noreply@example.com>\r\n\
             To: user@example.com\r\n\
             Subject: Reset your password\r\n\
             Message-ID: <0000@example.com>\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Transfer-Encoding: 8bit\r\n\
             \r\n\
             Open this link:\r\nhttps://x/y"
    );
}

#[test]
fn a_subject_carrying_a_newline_is_refused() {
    // Header injection. Without this, whoever chooses a subject chooses the
    // headers after it, and `Bcc:` is one of them.
    for subject in [
        "Reset\r\nBcc: attacker@example.com",
        "Reset\nBcc: attacker@example.com",
        "Reset\u{0}",
    ] {
        assert_eq!(rendered(subject, "body").err(), Some(MailError::Header));
    }
}

#[test]
fn a_non_ascii_subject_is_encoded_rather_than_refused() {
    // This test used to assert the opposite, and the assertion was right
    // for the mail that existed then: one password reset with a fixed
    // English subject, where refusing was visible and sending `Ã©` was not.
    //
    // C-016 makes the subject `[WR-125] Task title` (docs/29 §Email
    // content), and a title is whatever language the customer writes in. So
    // refusing stopped meaning "a developer wrote a bad subject" and
    // started meaning "this tenant does not get notification email",
    // silently. `crate::header` carries the RFC 2047 encoder; the header is
    // still pure ASCII on the wire, which is the property that mattered.
    let message = rendered("Réinitialiser", "body").expect("encoded, not refused");
    assert!(message.contains("Subject: =?UTF-8?B?"), "{message:?}");
    assert!(
        message.is_ascii(),
        "the rendered headers must be ASCII on the wire"
    );
}

#[test]
fn a_non_ascii_subject_carrying_a_newline_is_still_refused() {
    // The encoder must not become a way around the injection guard: base64
    // would carry a CRLF through intact and the decoded header would inject
    // after all.
    assert_eq!(
        rendered("Réinitialiser\r\nBcc: attacker@example.com", "body").err(),
        Some(MailError::Header)
    );
}

#[test]
fn a_body_line_ending_is_normalised_exactly_once() {
    // The bug this guards: replacing '\n' with "\r\n" on text that already
    // has CRLF produces "\r\r\n", which some relays reject and others
    // render as a blank line between every line.
    let message = rendered("s", "a\r\nb\nc").expect("ok");
    assert!(message.ends_with("a\r\nb\r\nc"), "{message:?}");
    assert!(!message.contains("\r\r"), "{message:?}");
}

#[tokio::test]
async fn the_no_op_mailer_accepts_a_message() {
    // It must not be an error path: an empty TF_SMTP_HOST is a supported
    // deployment, so a reset request there has to succeed.
    let mailer = LoggingMailer;
    let message = Message::new("user@example.com", "Reset your password", "a link");
    assert_eq!(mailer.send(&message).await, Ok(()));
}

#[test]
fn the_send_timeout_is_finite() {
    // AGENTS.md: "every external call timed". A relay that accepts the
    // connection and then stops talking holds the task forever without it.
    assert!(SEND_TIMEOUT > Duration::ZERO);
    assert!(SEND_TIMEOUT <= Duration::from_secs(30));
}
