//! Opaque pagination cursors. See `docs/26-SEARCH-INDEXING-AND-QUERY.md`.
//!
//! Offset pagination is banned: it scans, and it duplicates or skips rows under
//! concurrent writes — both of which a live board guarantees.
//!
//! A cursor encodes the **sort key plus an id tiebreaker**. The tiebreaker is
//! mandatory: ties in `updated_at` happen constantly on bulk operations, and
//! without it the cursor is non-deterministic.
//!
//! Cursors are opaque to clients. The internal shape is free to change
//! (`docs/07-QUALITY-SECURITY-AND-COMPATIBILITY.md` §Compatibility contract).

use crate::error::{Error, Result, codes};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    /// Sort key values, in the order the sort fields were requested.
    #[serde(rename = "k")]
    pub keys: Vec<String>,
    /// The mandatory tiebreaker.
    #[serde(rename = "id")]
    pub id: Uuid,
}

impl Cursor {
    pub fn new(keys: Vec<String>, id: Uuid) -> Self {
        Self { keys, id }
    }

    /// Encode for transport. Base64url without padding, so a cursor is safe in
    /// a query string without escaping.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("Cursor is always serializable");
        base64url_encode(&json)
    }

    pub fn decode(raw: &str) -> Result<Self> {
        let bytes = base64url_decode(raw)
            .ok_or_else(|| Error::new(codes::QRY_BAD_CURSOR, "Malformed pagination cursor"))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(codes::QRY_BAD_CURSOR, "Malformed pagination cursor"))
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        let idx = [n >> 18 & 63, n >> 12 & 63, n >> 6 & 63, n & 63];
        for (i, v) in idx.iter().enumerate() {
            if i <= chunk.len() {
                out.push(ALPHABET[*v as usize] as char);
            }
        }
    }
    out
}

fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    for c in input.bytes() {
        let v = ALPHABET.iter().position(|&a| a == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let c = Cursor::new(vec!["2026-08-08T10:14:22Z".into()], Uuid::now_v7());
        assert_eq!(Cursor::decode(&c.encode()).unwrap(), c);
    }

    #[test]
    fn round_trips_at_every_padding_boundary() {
        for n in 0..8 {
            let c = Cursor::new(vec!["x".repeat(n)], Uuid::now_v7());
            assert_eq!(Cursor::decode(&c.encode()).unwrap(), c, "length {n}");
        }
    }

    #[test]
    fn encoding_is_url_safe() {
        let c = Cursor::new(vec!["a/b+c=d".repeat(4)], Uuid::now_v7());
        let e = c.encode();
        assert!(
            !e.contains('/') && !e.contains('+') && !e.contains('='),
            "cursor must be safe in a query string without escaping: {e}"
        );
    }

    #[test]
    fn rejects_garbage_with_the_registry_code() {
        let err = Cursor::decode("!!!not-a-cursor!!!").unwrap_err();
        assert_eq!(err.code, codes::QRY_BAD_CURSOR);
    }
}
