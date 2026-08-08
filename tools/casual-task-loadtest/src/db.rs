//! A long-lived `psql` session driven over a pipe.
//!
//! This is the seam described in the crate docs. It exists because the
//! workspace has no PostgreSQL client dependency and adding one is a
//! workspace-level decision. When one arrives, this module is what gets
//! replaced; nothing above it changes.
//!
//! # How a sample is produced
//!
//! `psql`'s `\timing` reports the interval libpq spent on the query round trip.
//! The process is started once per run, so process start-up, connection setup,
//! and authentication are outside every sample. Result rows are routed to
//! `/dev/null` for timed queries: the rows are still transferred from the
//! server into the client result set — that transfer is part of the latency the
//! product will pay — but `psql` does not spend time rendering them, which the
//! product never would.
//!
//! # Costs of this approach, stated
//!
//! - Timing resolution is `psql`'s three decimal places of a millisecond, i.e.
//!   one microsecond. Every number in a report is therefore quantised to 1 µs.
//! - `psql`'s own per-statement bookkeeping is inside the sample. The
//!   `roundtrip_floor` case exists to make that constant visible instead of
//!   invisible.
//! - Framing is textual and sentinel-delimited. A statement whose *output*
//!   contained the sentinel would desynchronise the stream; the catalogue's
//!   statements return uuids, counts, and timestamps, and the sentinel is not a
//!   substring of any of them.
//! - There is no query cancellation. A statement that hangs would hang the run,
//!   so the session sets `statement_timeout` (docs/21 §Query limits).

use anyhow::{Context, Result, anyhow, bail};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// `docs/21` §Query limits — nothing runs away. This bounds every *measured*
/// statement: a case that cannot answer inside it has already failed the target
/// it exists to check.
const STATEMENT_TIMEOUT: &str = "5s";

/// Preflight is not a measurement, and must not be bounded as if it were.
///
/// The corpus description counts whole tables, and at the `docs/30` reference
/// capacity `count(*) FROM activity_event` reads 20 M rows across the partition
/// set — 1.8 s against a warm cache and well past `STATEMENT_TIMEOUT` against a
/// cold one. Bounding it at the measurement timeout aborts the run before a
/// single case is measured, and does so *only* at reference scale, which is the
/// one scale the harness exists for. It still gets a ceiling, because a
/// preflight that hangs forever is no better than a case that does.
const PREFLIGHT_TIMEOUT: &str = "10min";

/// How the `psql` binary is reached. Split out because `psql` is frequently not
/// on the host (the schema gate routes through a container the same way).
#[derive(Debug, Clone)]
pub struct ConnectSpec {
    /// Program and any leading arguments, e.g. `["docker", "exec", "-i", "pg",
    /// "psql"]`, or `["psql"]` alone.
    pub command: Vec<String>,
    /// Connection string passed to `psql` as its positional `dbname` argument.
    pub dsn: String,
}

#[derive(Debug)]
pub struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr_path: PathBuf,
    sentinel_counter: u64,
}

impl Session {
    pub fn connect(spec: &ConnectSpec) -> Result<Self> {
        let (program, prefix) = spec
            .command
            .split_first()
            .ok_or_else(|| anyhow!("empty psql command"))?;

        // stderr goes to a file rather than a pipe: reading a second pipe
        // without a reader thread deadlocks as soon as psql fills it, and a
        // reader thread is more machinery than a diagnostic path needs.
        let stderr_path = std::env::temp_dir().join(format!(
            "casual-task-loadtest-{}.stderr",
            std::process::id()
        ));
        let stderr = std::fs::File::create(&stderr_path)
            .with_context(|| format!("creating {}", stderr_path.display()))?;

        let mut child = Command::new(program)
            .args(prefix)
            .args([
                "-X", // ignore ~/.psqlrc: a developer's settings must not enter a measurement
                "-q", // no command tags
                "-A", // unaligned output: no column-width computation
                "-t", // no headers or row counts
                "-v",
                "ON_ERROR_STOP=1",
            ])
            .arg(&spec.dsn)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr))
            .spawn()
            .with_context(|| format!("spawning `{program}`"))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?);

        let mut session = Self {
            child,
            stdin,
            stdout,
            stderr_path,
            sentinel_counter: 0,
        };

        session.send_raw("\\timing on\n\\pset pager off\n")?;
        session
            .execute(&format!("SET statement_timeout = '{STATEMENT_TIMEOUT}'"))
            .context("setting statement_timeout — is the server reachable?")?;
        Ok(session)
    }

    /// Bind a `psql` variable, referenced from SQL as `:'name'` (quoted) or
    /// `:name` (bare). Values are escaped for `psql`'s single-quoted literal
    /// syntax; nothing is interpolated into SQL text.
    pub fn set_var(&mut self, name: &str, value: &str) -> Result<()> {
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            bail!("variable name `{name}` is not snake_case ascii");
        }
        self.send_raw(&format!("\\set {name} '{}'\n", escape_psql_literal(value)))
    }

    /// Run `f` under `PREFLIGHT_TIMEOUT`, then restore the measurement timeout.
    ///
    /// The restore runs on the error path too: a preflight that fails must not
    /// leave the session able to run a measured case unbounded, because that
    /// case would then be reported as a slow success rather than a failure.
    pub fn preflight<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.execute(&format!("SET statement_timeout = '{PREFLIGHT_TIMEOUT}'"))
            .context("raising statement_timeout for preflight")?;
        let out = f(self);
        let restored = self
            .execute(&format!("SET statement_timeout = '{STATEMENT_TIMEOUT}'"))
            .context("restoring the measurement statement_timeout");
        // On the error path `out`'s error is the one worth reporting; a failed
        // restore is a consequence of it, not an independent fact.
        match out {
            Ok(value) => restored.map(|()| value),
            Err(e) => Err(e),
        }
    }

    /// Run a statement for effect, discarding rows.
    pub fn execute(&mut self, sql: &str) -> Result<()> {
        self.round_trip(&format!("{sql};\n"))?;
        Ok(())
    }

    /// First column of every returned row.
    pub fn fetch_column(&mut self, sql: &str) -> Result<Vec<String>> {
        let lines = self.round_trip(&format!("{sql};\n"))?;
        Ok(lines
            .into_iter()
            .filter(|l| !l.starts_with("Time: "))
            .collect())
    }

    /// First column of the first row, or `None` when the statement returned no
    /// rows. Returning `None` rather than an empty string matters: a fixture
    /// probe that finds nothing must abort the run, not bind an empty variable.
    pub fn fetch_scalar(&mut self, sql: &str) -> Result<Option<String>> {
        Ok(self.fetch_column(sql)?.into_iter().next())
    }

    /// Execute `sql` once and report the round trip in microseconds.
    pub fn time_query(&mut self, sql: &str) -> Result<u64> {
        // \o /dev/null suppresses rendering; \o with no argument restores
        // stdout so the sentinel is visible again.
        let lines = self.round_trip(&format!("\\o /dev/null\n{sql};\n\\o\n"))?;
        let timing = lines
            .iter()
            .find(|l| l.starts_with("Time: "))
            .ok_or_else(|| anyhow!("psql reported no timing for statement: {sql}"))?;
        parse_timing_us(timing)
    }

    /// Write a script fragment and read every line up to a fresh sentinel.
    fn round_trip(&mut self, script: &str) -> Result<Vec<String>> {
        self.sentinel_counter += 1;
        let sentinel = format!("--CTLT-{}--", self.sentinel_counter);
        self.send_raw(script)?;
        self.send_raw(&format!("\\echo {sentinel}\n"))?;

        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            let read = self
                .stdout
                .read_line(&mut line)
                .context("reading psql stdout")?;
            if read == 0 {
                // psql exited. With ON_ERROR_STOP=1 that is what a failing
                // statement looks like, so surface its stderr rather than a
                // bare "unexpected EOF".
                let stderr = std::fs::read_to_string(&self.stderr_path).unwrap_or_default();
                bail!(
                    "psql exited before completing:\n{}\nwhile running:\n{}",
                    stderr.trim(),
                    script.trim()
                );
            }
            let line = line.trim_end_matches(['\n', '\r']).to_owned();
            if line == sentinel {
                return Ok(lines);
            }
            lines.push(line);
        }
    }

    fn send_raw(&mut self, s: &str) -> Result<()> {
        self.stdin
            .write_all(s.as_bytes())
            .context("writing to psql")?;
        self.stdin.flush().context("flushing psql stdin")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Closing stdin lets psql exit on its own; kill is the fallback for a
        // session wedged inside a statement.
        let _ = self.stdin.write_all(b"\\q\n");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.stderr_path);
    }
}

/// Escape for `psql`'s single-quoted variable syntax, which processes
/// backslash escapes.
fn escape_psql_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// Parse `Time: 1.234 ms` — or `Time: 1234.567 ms (00:00:01.234)` for long
/// statements — into microseconds.
fn parse_timing_us(line: &str) -> Result<u64> {
    let rest = line
        .strip_prefix("Time: ")
        .ok_or_else(|| anyhow!("not a psql timing line: {line}"))?;
    let ms_text = rest
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("empty psql timing line"))?;
    let ms: f64 = ms_text
        .parse()
        .with_context(|| format!("parsing milliseconds from `{line}`"))?;
    if !ms.is_finite() || ms < 0.0 {
        bail!("implausible psql timing: {line}");
    }
    Ok((ms * 1_000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_lines_parse_to_microseconds() {
        assert_eq!(parse_timing_us("Time: 0.159 ms").expect("parse"), 159);
        assert_eq!(parse_timing_us("Time: 12.500 ms").expect("parse"), 12_500);
        assert_eq!(parse_timing_us("Time: 0.000 ms").expect("parse"), 0);
    }

    #[test]
    fn long_statement_timing_with_a_duration_suffix_parses() {
        assert_eq!(
            parse_timing_us("Time: 1234.567 ms (00:00:01.234)").expect("parse"),
            1_234_567
        );
    }

    #[test]
    fn a_non_timing_line_is_an_error_not_a_zero() {
        assert!(parse_timing_us("1").is_err());
        assert!(parse_timing_us("Time: banana ms").is_err());
    }

    #[test]
    fn literals_are_escaped_for_psql_set() {
        assert_eq!(escape_psql_literal("payment retry"), "payment retry");
        assert_eq!(escape_psql_literal("o'brien"), "o\\'brien");
        assert_eq!(escape_psql_literal("a\\b"), "a\\\\b");
        assert_eq!(escape_psql_literal("a\nb"), "a\\nb");
    }
}
