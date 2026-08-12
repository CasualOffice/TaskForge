//! Malware scanning — step 4 of `docs/28`.
//!
//! # Why an attachment is invisible until this runs
//!
//! `docs/28` sets `committed_at` only on `PENDING → CLEAN`, and every read of an
//! attachment requires `committed_at IS NOT NULL`. That is not belt and braces:
//! it means a forgotten `WHERE` clause cannot leak an unscanned file, because
//! there is no state in which an unscanned file is listed. The cost is that
//! **a deployment with no scanner stores files nobody can ever see** — which is
//! D-062, countersigned, and deliberately not something an implementation may
//! quietly reverse by treating "no scanner" as "clean".
//!
//! This module is what makes the other half true: with a scanner configured,
//! uploads become visible.
//!
//! # Why ClamAV over a socket and not a library
//!
//! `docs/28` §Validation: "ClamAV by default; pluggable". `clamd` is a daemon
//! that already exists in every deployment target's package manager, holds its
//! signature database in memory once for all callers, and updates it on its own
//! schedule through `freshclam`. Linking a scanner into this process would mean
//! this process owning signature updates, and a worker restart would cost the
//! several seconds it takes to load them.
//!
//! `INSTREAM` rather than `SCAN /path`: the bytes may live in object storage
//! that clamd cannot see, and a shared filesystem between the worker and the
//! scanner is a deployment constraint this design does not otherwise need.
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, ScanError>> + Send + 'a>>;

/// What a scanner concluded about some bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Clean,
    /// The signature name, which is what the uploader is told and what an
    /// administrator needs in order to judge a false positive.
    Infected(String),
}

#[derive(Debug)]
pub enum ScanError {
    /// The scanner could not be reached, or did not answer in time.
    Unavailable(String),
    /// It answered something this code does not understand. Distinct from
    /// `Unavailable` because retrying will not help.
    Unintelligible(String),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(why) => write!(f, "the scanner is unavailable: {why}"),
            Self::Unintelligible(what) => {
                write!(f, "the scanner said something unexpected: {what}")
            }
        }
    }
}

impl std::error::Error for ScanError {}

/// Something that can decide whether bytes are safe to serve.
pub trait Scanner: Send + Sync + Debug {
    /// Scan `bytes`.
    ///
    /// # Errors
    ///
    /// [`ScanError`] when the scanner could not be reached or gave an answer
    /// this code cannot read. Neither is a verdict: an attachment whose scan
    /// *failed* must stay unreadable rather than be assumed either way.
    fn scan<'a>(&'a self, bytes: &'a [u8]) -> Fut<'a, Verdict>;
}

/// `clamd` over TCP (`TF_CLAMD_ADDR`).
#[derive(Debug, Clone)]
pub struct Clamd {
    address: String,
    timeout: Duration,
}

impl Clamd {
    /// `docs/28` §Limits: a 60 s scan timeout.
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

    #[must_use]
    pub fn new(address: String) -> Self {
        Self {
            address,
            timeout: Self::DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// One `INSTREAM` exchange.
    ///
    /// The wire format, because it is not guessable from the name: `zINSTREAM\0`
    /// then a sequence of chunks, each a **big-endian `u32` length followed by
    /// that many bytes**, terminated by a zero length. `z` asks clamd to
    /// null-terminate its reply, which is what makes the read below able to stop
    /// without guessing.
    async fn instream(&self, bytes: &[u8]) -> Result<String, ScanError> {
        let mut stream = TcpStream::connect(&self.address)
            .await
            .map_err(|error| ScanError::Unavailable(error.to_string()))?;

        stream
            .write_all(b"zINSTREAM\0")
            .await
            .map_err(|error| ScanError::Unavailable(error.to_string()))?;

        // 64 KiB, which is comfortably under clamd's default `StreamMaxLength`
        // chunk handling and large enough that a 100 MB attachment is 1,600
        // writes rather than 100,000.
        for chunk in bytes.chunks(64 * 1024) {
            let length = u32::try_from(chunk.len())
                .map_err(|_| ScanError::Unintelligible("chunk too large".to_owned()))?;
            stream
                .write_all(&length.to_be_bytes())
                .await
                .map_err(|error| ScanError::Unavailable(error.to_string()))?;
            stream
                .write_all(chunk)
                .await
                .map_err(|error| ScanError::Unavailable(error.to_string()))?;
        }

        // A zero-length chunk is the terminator. Without it clamd waits.
        stream
            .write_all(&0_u32.to_be_bytes())
            .await
            .map_err(|error| ScanError::Unavailable(error.to_string()))?;

        let mut reply = Vec::new();
        stream
            .read_to_end(&mut reply)
            .await
            .map_err(|error| ScanError::Unavailable(error.to_string()))?;

        String::from_utf8(reply)
            .map(|text| text.trim_end_matches('\0').trim().to_owned())
            .map_err(|_| ScanError::Unintelligible("the reply was not UTF-8".to_owned()))
    }
}

impl Scanner for Clamd {
    fn scan<'a>(&'a self, bytes: &'a [u8]) -> Fut<'a, Verdict> {
        Box::pin(async move {
            let reply = tokio::time::timeout(self.timeout, self.instream(bytes))
                .await
                .map_err(|_| ScanError::Unavailable("the scan timed out".to_owned()))??;
            verdict_of(&reply)
        })
    }
}

/// Read clamd's one-line answer.
///
/// `stream: OK`, `stream: Eicar-Test-Signature FOUND`, or `… ERROR`. Parsed in
/// its own function so the three shapes can be tested without a daemon — the
/// alternative is a test suite that needs ClamAV installed to check a string.
///
/// # Errors
///
/// [`ScanError::Unintelligible`] for anything that is not one of the three.
pub fn verdict_of(reply: &str) -> Result<Verdict, ScanError> {
    if reply.ends_with("OK") {
        return Ok(Verdict::Clean);
    }
    if let Some(rest) = reply.strip_suffix("FOUND") {
        // `stream: Eicar-Test-Signature FOUND` → `Eicar-Test-Signature`.
        let signature = rest.rsplit(':').next().unwrap_or(rest).trim().to_owned();
        return Ok(Verdict::Infected(if signature.is_empty() {
            "unnamed signature".to_owned()
        } else {
            signature
        }));
    }
    // `ERROR` is clamd telling us it could not do the job — a size limit, a
    // broken stream. Not a verdict, and not something to retry blindly either,
    // which is why it is an error and the caller decides.
    Err(ScanError::Unintelligible(reply.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_stream_is_clean() {
        assert_eq!(verdict_of("stream: OK").expect("verdict"), Verdict::Clean);
    }

    #[test]
    fn a_signature_is_carried_through() {
        // The name matters: it is what the uploader is told and what an
        // administrator needs to judge a false positive.
        assert_eq!(
            verdict_of("stream: Eicar-Test-Signature FOUND").expect("verdict"),
            Verdict::Infected("Eicar-Test-Signature".to_owned()),
        );
    }

    #[test]
    fn an_error_is_not_a_verdict() {
        // The failure this prevents: reading "ERROR" as "not FOUND" and
        // therefore as clean, which would publish an unscanned file.
        assert!(verdict_of("INSTREAM size limit exceeded. ERROR").is_err());
        assert!(verdict_of("").is_err());
        assert!(verdict_of("something else entirely").is_err());
    }
}
