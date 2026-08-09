//! Header values that are safe to concatenate (RFC 5322 §2.2, RFC 2047).
//!
//! # The failure this module prevents
//!
//! Two, and they pull in opposite directions.
//!
//! **Header injection.** A header is assembled by concatenation, so a `\r\n`
//! inside a value appends whatever follows it as new headers — a `Bcc`, a
//! second `To`. [`is_safe_line`] is the guard, and it is why every value that
//! reaches a header goes through this module.
//!
//! **Silently undeliverable mail.** `docs/29` §Email content puts a task title
//! in the subject: `[WR-125] Task title`. Titles are customer content and
//! customer content is not ASCII. Before C-016 the composer refused any
//! non-ASCII subject outright, with the note that "a non-ASCII byte needs an
//! RFC 2047 encoder this module deliberately does not carry" — correct while the
//! only outbound mail was a password reset with a fixed English subject, and
//! wrong the moment a notification subject carries a title somebody wrote in
//! French. Every notification about a task called `Café` would have failed to
//! send.
//!
//! [`encode_subject`] is that encoder. It is ~30 lines because RFC 2047's
//! `B` encoding is base64 of UTF-8 and nothing else, and because the
//! alternative — widening `deny.toml`'s licence allow-list to pull in a MIME
//! crate — is the decision D-050 already refused once.
//!
//! # Why this is not in `mail.rs`
//!
//! Different reason to change. `mail.rs` changes when the *transport* changes —
//! a relay, a TLS policy, an authentication mechanism. This changes when the
//! *encoding* changes, which is governed by an RFC that has not moved since
//! 1996. Keeping them apart is also what stopped `mail.rs` growing past the
//! point where anyone reads it.

/// Whether a header value can be concatenated as-is: one line, ASCII, no
/// control characters.
///
/// The header-injection guard. `\r\n` in a value appends headers.
#[must_use]
pub fn is_safe_line(value: &str) -> bool {
    value.is_ascii() && !value.chars().any(|c| c.is_ascii_control())
}

/// The largest encoded-word RFC 2047 §2 permits, in bytes.
const MAX_ENCODED_WORD: usize = 75;

/// The overhead of `=?UTF-8?B?` and `?=` around the payload.
const WRAPPER: usize = "=?UTF-8?B?".len() + "?=".len();

/// A subject line safe to put in a header.
///
/// Returns the subject unchanged when it is already a safe ASCII line — the
/// common case, and one where an encoded word would only make the message
/// harder to read in a client that renders headers raw.
///
/// Otherwise returns RFC 2047 encoded words. `None` when the value cannot be
/// made safe at all, which is only when it carries a control character:
/// stripping those silently would let a caller put `\r\n` in a subject and get
/// a *different* subject than they asked for, and this returning `None` is what
/// turns that into a refused send rather than a forged header.
#[must_use]
pub fn encode_subject(subject: &str) -> Option<String> {
    // A control character is never encodable: RFC 2047 would carry it through
    // base64 intact, and the decoded header would inject after all.
    if subject.chars().any(|c| c.is_control()) {
        return None;
    }
    if subject.is_ascii() {
        return Some(subject.to_owned());
    }

    // RFC 2047 §2 caps an encoded word at 75 bytes including the wrapper, so
    // the payload is split into chunks whose base64 fits. Base64 is 4 bytes out
    // per 3 in, and a chunk boundary must not fall inside a UTF-8 character or
    // the decoder reassembles mojibake.
    let budget = (MAX_ENCODED_WORD - WRAPPER) / 4 * 3;
    let mut words: Vec<String> = Vec::new();
    let mut chunk = String::new();
    for character in subject.chars() {
        if chunk.len() + character.len_utf8() > budget {
            words.push(encoded_word(&chunk));
            chunk.clear();
        }
        chunk.push(character);
    }
    if !chunk.is_empty() {
        words.push(encoded_word(&chunk));
    }
    // Folded with CRLF + a space, which is how RFC 5322 §2.2.3 continues a
    // header. Adjacent encoded words are joined by the decoder with the
    // whitespace between them removed, which is what makes this reassemble
    // into one subject rather than one with spaces injected.
    Some(words.join("\r\n "))
}

fn encoded_word(chunk: &str) -> String {
    format!("=?UTF-8?B?{}?=", base64(chunk.as_bytes()))
}

/// Standard base64 with padding (RFC 4648 §4).
///
/// Written here for the reason the module docs give: the cost of a dependency
/// for this is a licence review, and the cost of this is twelve lines.
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> shift & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ascii_subject_is_left_alone() {
        // The common case. Encoding it anyway would make every subject
        // unreadable in a client that renders headers raw, for no gain.
        assert_eq!(
            encode_subject("[WR-125] Ship the thing").as_deref(),
            Some("[WR-125] Ship the thing")
        );
    }

    #[test]
    fn a_title_with_an_accent_is_encoded_rather_than_refused() {
        // The bug this module exists for: before it, every notification about a
        // task whose title was not ASCII failed to send, and the in-app row was
        // the only trace.
        let encoded = encode_subject("[WR-125] Café").expect("encodable");
        assert!(encoded.starts_with("=?UTF-8?B?"), "{encoded}");
        assert!(encoded.ends_with("?="), "{encoded}");
        assert!(encoded.is_ascii(), "an encoded word must be ASCII");
        assert!(is_safe_line(&encoded.replace("\r\n ", "")));
    }

    #[test]
    fn an_encoded_subject_decodes_back_to_the_original() {
        // The property that matters to the person reading the mail. Asserted by
        // decoding rather than by matching a golden string, so it holds for
        // inputs nobody thought to write down.
        for subject in [
            "[WR-125] Café",
            "[WR-1] 日本語のタスク",
            "[OPS-9] Ünïcödé everywhere, and then some more text to force folding across words",
            "[X-1] emoji 🎉 in a title",
        ] {
            let encoded = encode_subject(subject).expect("encodable");
            assert_eq!(decode(&encoded), subject, "round trip failed for {subject}");
        }
    }

    #[test]
    fn no_encoded_word_exceeds_the_rfc_limit() {
        // RFC 2047 §2: 75 bytes including the wrapper. A relay is entitled to
        // reject a longer one, and the failure would look like "email is
        // broken" rather than "this subject was too long".
        let long = format!("[WR-125] {}", "é".repeat(200));
        let encoded = encode_subject(&long).expect("encodable");
        for word in encoded.split("\r\n ") {
            assert!(
                word.len() <= MAX_ENCODED_WORD,
                "{} bytes: {word}",
                word.len()
            );
        }
    }

    #[test]
    fn a_control_character_is_refused_and_never_stripped() {
        // The header-injection case. Stripping would send a subject the caller
        // did not ask for; refusing turns it into a failed send, which the
        // in-app notification survives.
        assert_eq!(
            encode_subject("[WR-1] a\r\nBcc: attacker@example.com"),
            None
        );
        assert_eq!(encode_subject("[WR-1] a\nb"), None);
        assert_eq!(encode_subject("[WR-1] tab\there"), None);
        // And with non-ASCII beside it, so the encoding path is guarded too.
        assert_eq!(encode_subject("[WR-1] Café\r\nBcc: x@example.com"), None);
    }

    #[test]
    fn is_safe_line_rejects_what_it_should() {
        assert!(is_safe_line("[WR-125] Ship it"));
        assert!(!is_safe_line("two\r\nlines"));
        assert!(!is_safe_line("Café"));
    }

    #[test]
    fn base64_matches_the_rfc_4648_test_vectors() {
        // RFC 4648 §10. A hand-written encoder with no vectors is a hand-written
        // encoder nobody has checked.
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected, "input {input:?}");
        }
    }

    /// Decode RFC 2047 encoded words back to text. Test-only: nothing in the
    /// product reads mail.
    fn decode(encoded: &str) -> String {
        encoded
            .split("\r\n ")
            .map(|word| {
                word.strip_prefix("=?UTF-8?B?")
                    .and_then(|w| w.strip_suffix("?="))
                    .map_or_else(
                        || word.to_owned(),
                        |payload| String::from_utf8(unbase64(payload)).expect("valid utf-8"),
                    )
            })
            .collect()
    }

    fn unbase64(input: &str) -> Vec<u8> {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut acc: u32 = 0;
        let mut bits = 0_u32;
        let mut out = Vec::new();
        for byte in input.bytes().filter(|b| *b != b'=') {
            let value = ALPHABET
                .iter()
                .position(|a| *a == byte)
                .expect("base64 alphabet") as u32;
            acc = acc << 6 | value;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        out
    }
}
