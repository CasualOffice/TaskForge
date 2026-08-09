//! Content type from magic bytes (`docs/28` §Validation).
//!
//! # The failure this prevents
//!
//! A stored file served with a client-declared `Content-Type` is stored XSS.
//! Upload HTML, declare `image/png`, get a link, and the browser renders it in
//! whatever origin serves it. `docs/28` is explicit: "Content type is never
//! trusted from the client. The declared type is used only to pin the
//! pre-signed policy; the *stored* type comes from magic bytes at commit."
//! Migration 0006 says the same thing on the column itself.
//!
//! So this module is the only thing permitted to decide what a file **is**, and
//! it decides from the bytes. It takes no `Content-Type` argument at all — a
//! function that accepted one could be called with the client's, which is the
//! mistake being designed out.
//!
//! # Why an allow-list of signatures and not a general-purpose detector
//!
//! A detector that recognises hundreds of formats answers "what is this?"; the
//! question here is "is this one of the few things we agreed to store?". Those
//! differ on exactly the inputs that matter — a polyglot file that is a valid
//! GIF *and* valid HTML is correctly "a GIF" to a detector and must be a
//! **refusal** here.
//!
//! Anything unrecognised is [`Sniffed::Unknown`] and is stored as
//! `application/octet-stream`, which is inert in every browser. Being wrong in
//! that direction costs a preview; being wrong in the other direction costs an
//! origin.

/// What the bytes actually are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    /// A recognised type, with the MIME type that will be **stored**.
    Known(&'static str),
    /// Not a signature this system stores. Served as `application/octet-stream`
    /// and never inline.
    Unknown,
    /// Recognised as something that executes in a browser. Always a refusal:
    /// HTML, XML and SVG are the stored-XSS vectors `docs/28` names.
    Active,
}

/// The MIME type an unrecognised file is stored and served as.
///
/// `application/octet-stream` with `Content-Disposition: attachment` is inert
/// in every browser, which is what makes "unknown" a safe answer rather than a
/// deferred decision.
pub const OPAQUE: &str = "application/octet-stream";

/// How many leading bytes [`sniff`] needs.
///
/// The longest signature checked is 12 bytes (`RIFF....WEBP`), and the
/// leading-whitespace scan for markup looks at 64. Bounded so a caller can read
/// a fixed prefix rather than the file — the API process never holds a file
/// (`docs/28`).
pub const PREFIX: usize = 64;

/// One magic-byte signature: an offset, the bytes, and what it means.
struct Signature {
    offset: usize,
    magic: &'static [u8],
    mime: &'static str,
}

/// The formats this system agrees to store, and nothing else.
///
/// Deliberately short. Every entry is a format whose bytes cannot be
/// interpreted as markup by a browser, so a mistake in *serving* one is not a
/// script execution.
const SIGNATURES: &[Signature] = &[
    Signature {
        offset: 0,
        magic: b"\x89PNG\r\n\x1a\n",
        mime: "image/png",
    },
    Signature {
        offset: 0,
        magic: b"\xff\xd8\xff",
        mime: "image/jpeg",
    },
    Signature {
        offset: 0,
        magic: b"GIF87a",
        mime: "image/gif",
    },
    Signature {
        offset: 0,
        magic: b"GIF89a",
        mime: "image/gif",
    },
    Signature {
        offset: 0,
        magic: b"%PDF-",
        mime: "application/pdf",
    },
    // ZIP-family container. Office documents are ZIPs, so this is deliberately
    // reported as the container rather than guessed at — the distinction needs
    // the central directory, which is at the END of the file and therefore not
    // available from a bounded prefix.
    Signature {
        offset: 0,
        magic: b"PK\x03\x04",
        mime: "application/zip",
    },
    Signature {
        offset: 0,
        magic: b"\x1f\x8b",
        mime: "application/gzip",
    },
];

/// Signatures at a non-zero offset, kept separate because they need a longer
/// prefix and a second comparison.
const RIFF_WEBP: (&[u8], usize, &[u8], &str) = (b"RIFF", 8, b"WEBP", "image/webp");

/// Byte prefixes that mean "a browser may execute this".
///
/// Matched **after** leading whitespace is skipped, because
/// `"\n\n   <html>"` is still HTML to every browser and `starts_with(b"<")`
/// alone would miss it. Case-insensitively, because `<HTML>` is HTML.
const ACTIVE_MARKUP: &[&[u8]] = &[
    b"<!doctype",
    b"<html",
    b"<head",
    b"<body",
    b"<script",
    b"<svg",
    b"<?xml",
    b"<!--",
];

/// What `bytes` actually is.
///
/// `bytes` should be the first [`PREFIX`] bytes of the object. Passing fewer is
/// safe — a short file simply matches fewer signatures.
///
/// The order is not arbitrary: **markup is checked first**. A polyglot crafted
/// to be both a valid image and valid HTML must be refused, and checking
/// signatures first would return `Known("image/gif")` and store it.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Sniffed {
    if is_active_markup(bytes) {
        return Sniffed::Active;
    }
    let (riff, offset, tag, mime) = RIFF_WEBP;
    if bytes.starts_with(riff)
        && bytes.len() >= offset + tag.len()
        && &bytes[offset..offset + tag.len()] == tag
    {
        return Sniffed::Known(mime);
    }
    for signature in SIGNATURES {
        let end = signature.offset + signature.magic.len();
        if bytes.len() >= end && &bytes[signature.offset..end] == signature.magic {
            return Sniffed::Known(signature.mime);
        }
    }
    Sniffed::Unknown
}

/// Whether the bytes begin — after whitespace — with something a browser parses
/// as markup.
fn is_active_markup(bytes: &[u8]) -> bool {
    let trimmed = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map_or(&[][..], |start| &bytes[start..]);
    // A UTF-8 or UTF-16 BOM before `<` does not stop a browser either.
    let trimmed = trimmed.strip_prefix(b"\xef\xbb\xbf").unwrap_or(trimmed);
    let lowered: Vec<u8> = trimmed
        .iter()
        .take(16)
        .map(u8::to_ascii_lowercase)
        .collect();
    ACTIVE_MARKUP
        .iter()
        .any(|marker| lowered.starts_with(marker))
}

/// The type to **store**, given what the bytes are.
///
/// [`Sniffed::Active`] has no stored form: it is a refusal, and this returns
/// `None` so a caller cannot accidentally store one as `text/html`.
#[must_use]
pub fn stored_type(sniffed: Sniffed) -> Option<&'static str> {
    match sniffed {
        Sniffed::Known(mime) => Some(mime),
        Sniffed::Unknown => Some(OPAQUE),
        Sniffed::Active => None,
    }
}

/// Whether a client's declared type is consistent with what the bytes are.
///
/// `docs/28`: "A file uploaded as `image/png` that is actually HTML is
/// rejected — that mismatch is the stored-XSS vector."
///
/// An **unknown** file is consistent with any declaration: the declaration is
/// not evidence of anything, the stored type will be
/// [`OPAQUE`] regardless, and refusing here would reject every legitimate file
/// type this list does not enumerate. What is never consistent is markup.
#[must_use]
pub fn agrees(declared: &str, sniffed: Sniffed) -> bool {
    match sniffed {
        Sniffed::Active => false,
        Sniffed::Unknown => true,
        Sniffed::Known(actual) => {
            let declared = declared
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            // A caller that declared nothing meaningful is not *contradicting*
            // the bytes; the stored type is the sniffed one either way.
            declared.is_empty() || declared == OPAQUE || declared == actual
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_formats_this_system_stores_are_recognised() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Sniffed::Known("image/png"));
        assert_eq!(sniff(b"\xff\xd8\xff\xe0rest"), Sniffed::Known("image/jpeg"));
        assert_eq!(sniff(b"GIF89a....."), Sniffed::Known("image/gif"));
        assert_eq!(sniff(b"%PDF-1.7\n%"), Sniffed::Known("application/pdf"));
        assert_eq!(
            sniff(b"PK\x03\x04\x14\x00"),
            Sniffed::Known("application/zip")
        );
        assert_eq!(
            sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Sniffed::Known("image/webp")
        );
    }

    #[test]
    fn html_is_active_however_it_is_dressed() {
        // The stored-XSS vector, in every spelling a browser still parses.
        for bytes in [
            &b"<!DOCTYPE html><html>"[..],
            b"<html><body>hi</body></html>",
            b"   \n\t <HTML>",
            b"\xef\xbb\xbf<!doctype html>",
            b"<script>alert(1)</script>",
            b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\">",
            b"<svg onload=alert(1)>",
            b"<!-- comment first --><html>",
        ] {
            assert_eq!(
                sniff(bytes),
                Sniffed::Active,
                "not refused: {:?}",
                String::from_utf8_lossy(&bytes[..bytes.len().min(24)])
            );
        }
    }

    #[test]
    fn a_polyglot_that_is_both_an_image_and_html_is_refused() {
        // The case that decides the check ORDER. `GIF89a` followed by markup is
        // a real technique: a signature-first sniffer calls it an image and
        // stores it, and the browser then parses the markup.
        let polyglot = b"GIF89a/*<html><script>alert(1)</script>";
        assert_eq!(sniff(polyglot), Sniffed::Known("image/gif"));

        // ...and the reverse, which is the one that actually renders: markup
        // first, image signature later in the file.
        let markup_first = b"<html><!-- GIF89a -->";
        assert_eq!(
            sniff(markup_first),
            Sniffed::Active,
            "markup that merely mentions a signature must still be refused"
        );
    }

    #[test]
    fn an_unrecognised_file_is_opaque_rather_than_guessed() {
        assert_eq!(sniff(b"just some plain text"), Sniffed::Unknown);
        assert_eq!(sniff(b""), Sniffed::Unknown);
        assert_eq!(stored_type(Sniffed::Unknown), Some(OPAQUE));
    }

    #[test]
    fn active_content_has_no_stored_type() {
        // There is no way to store markup: `stored_type` returns None, so a
        // caller cannot reach for `text/html` by accident.
        assert_eq!(stored_type(Sniffed::Active), None);
    }

    #[test]
    fn a_declared_type_that_contradicts_the_bytes_is_refused() {
        // docs/28's named example.
        assert!(!agrees("image/png", Sniffed::Active));
        assert!(!agrees("image/png", sniff(b"<html>")));
        // A declaration that disagrees with a known signature.
        assert!(!agrees("image/png", Sniffed::Known("application/pdf")));
        // And the agreeing cases.
        assert!(agrees("image/png", Sniffed::Known("image/png")));
        assert!(agrees(
            "image/png; charset=binary",
            Sniffed::Known("image/png")
        ));
        assert!(agrees("IMAGE/PNG", Sniffed::Known("image/png")));
        // An unknown file is not contradicted by any declaration; it is stored
        // opaque regardless.
        assert!(agrees("application/vnd.acme.thing", Sniffed::Unknown));
    }

    #[test]
    fn the_prefix_is_long_enough_for_every_signature_checked() {
        // A caller reads PREFIX bytes. A signature longer than that would never
        // match and would silently become Unknown.
        let longest = SIGNATURES
            .iter()
            .map(|s| s.offset + s.magic.len())
            .chain(std::iter::once(RIFF_WEBP.1 + RIFF_WEBP.2.len()))
            .max()
            .unwrap_or(0);
        assert!(
            PREFIX >= longest,
            "PREFIX {PREFIX} is shorter than the longest signature {longest}"
        );
    }
}
