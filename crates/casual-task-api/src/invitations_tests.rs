use super::*;

#[test]
fn the_email_carries_the_link_and_nothing_about_the_workspace() {
    // Delivered to an address nobody has proved they control. A workspace
    // name here reveals who is working with whom to whoever holds the
    // mailbox — or to whoever the mail was forwarded to.
    let body = invite_body("https://tasks.example.com", "abc.def");
    assert!(body.contains("https://tasks.example.com/accept-invitation?token=abc.def"));
    assert!(
        body.contains("once"),
        "the single-use property is not stated"
    );
    assert!(body.contains("seven days"), "the expiry is not stated");
    assert!(
        body.contains("tied to this"),
        "the address binding is not stated"
    );
}

#[test]
fn a_trailing_slash_on_the_public_url_does_not_double() {
    let body = invite_body("https://tasks.example.com/", "abc.def");
    assert!(body.contains("com/accept-invitation?"), "{body}");
    assert!(!body.contains("com//"), "{body}");
}

#[test]
fn the_subject_is_something_casual_task_infra_will_accept() {
    assert!(INVITE_SUBJECT.is_ascii());
    assert!(!INVITE_SUBJECT.chars().any(|c| c.is_ascii_control()));
}

#[test]
fn the_acceptance_message_reveals_nothing() {
    // Byte-identical for an address with an account and one without, which
    // is the docs/40 enumeration gate this endpoint had to close.
    assert!(Accepted::TEXT.starts_with("If that address"));
}

#[test]
fn an_address_that_could_carry_a_header_is_refused() {
    // The injection case. casual-task-infra refuses it too; refusing twice
    // is cheaper than deciding which layer owns it.
    for bad in [
        "user@example.com\r\nBcc: attacker@example.com",
        "user@example.com\nBcc: x@y.com",
        "user @example.com",
        "",
        "no-at-sign",
        "a@b@c.com",
        "@example.com",
        "user@",
        "user@nodot",
    ] {
        assert!(valid_email(bad, "r").is_err(), "accepted {bad:?}");
    }
}

#[test]
fn an_ordinary_address_is_accepted() {
    // The companion: a validator that refused everything would satisfy the
    // test above and break every invitation.
    for good in [
        "user@example.com",
        "first.last+tag@sub.example.co.uk",
        "  spaced@example.com  ",
    ] {
        assert_eq!(valid_email(good, "r").expect("accepted"), good.trim());
    }
}

#[test]
fn a_display_name_falls_back_to_the_local_part() {
    assert_eq!(local_part("ada@example.com"), "ada");
    assert_eq!(local_part("malformed"), "malformed");
}
