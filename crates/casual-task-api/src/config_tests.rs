use super::*;

fn valid() -> Vec<(&'static str, &'static str)> {
    vec![
        ("DATABASE_URL", "postgres://app:pw@localhost/tf"),
        ("TF_PUBLIC_URL", "https://tasks.example.com"),
        ("TF_ATTACHMENT_ORIGIN", "https://files.example.com"),
        ("TF_SECRET_KEY", "0123456789012345678901234567890123"),
    ]
}

fn with(overrides: &[(&'static str, &'static str)]) -> Result<Config, ConfigError> {
    let mut env = valid();
    for (key, value) in overrides {
        env.retain(|(k, _)| k != key);
        if !value.is_empty() {
            env.push((key, value));
        }
    }
    Config::from_source(|name| {
        env.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| (*v).to_owned())
    })
}

#[test]
fn a_complete_environment_is_accepted() {
    let config = with(&[]).expect("valid");
    assert_eq!(config.bind_addr.port(), 8080);
    assert_eq!(config.pool.max_connections, 32);
}

#[test]
fn a_missing_required_variable_names_itself() {
    // The error text is the entire diagnostic an operator gets from a
    // container that exited before writing a log line.
    for name in [
        "DATABASE_URL",
        "TF_PUBLIC_URL",
        "TF_ATTACHMENT_ORIGIN",
        "TF_SECRET_KEY",
    ] {
        assert_eq!(with(&[(name, "")]).err(), Some(ConfigError::Missing(name)));
    }
}

#[test]
fn a_shared_attachment_origin_refuses_to_start() {
    // docs/28: sharing the origin means a stored HTML or SVG attachment
    // executes with access to application cookies. Starting is worse than
    // not starting.
    assert_eq!(
        with(&[("TF_ATTACHMENT_ORIGIN", "https://tasks.example.com")]).err(),
        Some(ConfigError::SharedAttachmentOrigin)
    );
}

#[test]
fn a_shared_origin_is_caught_through_cosmetic_differences() {
    // The check compares origins, not strings. A trailing slash, a path, or
    // different case would otherwise let the exact misconfiguration it
    // exists to reject sail through.
    for disguise in [
        "https://tasks.example.com/",
        "https://TASKS.example.com",
        "https://tasks.example.com/files",
        "  https://tasks.example.com  ",
    ] {
        assert_eq!(
            with(&[("TF_ATTACHMENT_ORIGIN", disguise)]).err(),
            Some(ConfigError::SharedAttachmentOrigin),
            "{disguise} was accepted as a distinct origin"
        );
    }
}

#[test]
fn a_different_host_is_accepted() {
    // And the check must not be so eager that a correct deployment fails.
    assert!(with(&[("TF_ATTACHMENT_ORIGIN", "https://files.example.com/x")]).is_ok());
    assert!(with(&[("TF_ATTACHMENT_ORIGIN", "https://tasks.example.com:9000")]).is_ok());
}

#[test]
fn a_short_secret_refuses_to_start() {
    assert_eq!(
        with(&[("TF_SECRET_KEY", "too-short")]).err(),
        Some(ConfigError::WeakSecret {
            minimum: MIN_SECRET_LEN
        })
    );
}

#[test]
fn a_malformed_bind_address_says_what_was_expected() {
    let error = with(&[("TF_BIND_ADDR", "8080")]).expect_err("rejected");
    let ConfigError::Invalid { name, reason } = error else {
        panic!("wrong variant")
    };
    assert_eq!(name, "TF_BIND_ADDR");
    assert!(reason.contains("host:port"), "{reason}");
}

#[test]
fn pool_bounds_are_configurable_and_bounded_by_default() {
    // D-039: both bounds stated. A default acquire timeout of "forever"
    // would make the 503 path unreachable.
    let default = PoolConfig::default();
    assert!(default.max_connections > 0);
    assert!(default.acquire_timeout > Duration::ZERO);
    assert!(
        default.acquire_timeout <= Duration::from_secs(5),
        "a caller waiting this long has usually been abandoned by its client"
    );

    let config = with(&[
        ("TF_DB_MAX_CONNECTIONS", "8"),
        ("TF_DB_ACQUIRE_TIMEOUT_SECONDS", "1"),
    ])
    .expect("valid");
    assert_eq!(config.pool.max_connections, 8);
    assert_eq!(config.pool.acquire_timeout, Duration::from_secs(1));
}

#[test]
fn email_is_disabled_by_default_and_that_is_not_an_error() {
    // docs/48: an empty host disables email. Profile 1 is a single node
    // with no relay, and it has to start.
    let config = with(&[]).expect("valid");
    assert!(!config.smtp.enabled());
    assert_eq!(config.smtp.port, 587, "the STARTTLS submission port");
}

#[test]
fn a_relay_without_a_sender_refuses_to_start() {
    // The alternative is a deployment that looks configured and fails on
    // the first password reset — found by the user, not by the operator.
    assert_eq!(
        with(&[("TF_SMTP_HOST", "smtp.example.com")]).err(),
        Some(ConfigError::Missing("TF_SMTP_FROM"))
    );
    assert!(
        with(&[
            ("TF_SMTP_HOST", "smtp.example.com"),
            ("TF_SMTP_FROM", "noreply@example.com"),
        ])
        .is_ok()
    );
}

#[test]
fn a_port_outside_the_tcp_range_is_refused() {
    // 70000 parses as a u32 and truncates to 4464 as a u16. A relay quietly
    // contacted on the wrong port is worse than a refusal to start.
    let error = with(&[("TF_SMTP_PORT", "70000")]).expect_err("rejected");
    assert!(matches!(
        error,
        ConfigError::Invalid {
            name: "TF_SMTP_PORT",
            ..
        }
    ));
}

#[test]
fn a_non_numeric_pool_bound_is_refused_rather_than_defaulted() {
    // Falling back to the default on a typo would mean a deployment that
    // asked for 4 connections silently gets 32.
    let error = with(&[("TF_DB_MAX_CONNECTIONS", "lots")]).expect_err("rejected");
    assert!(matches!(
        error,
        ConfigError::Invalid {
            name: "TF_DB_MAX_CONNECTIONS",
            ..
        }
    ));
}
