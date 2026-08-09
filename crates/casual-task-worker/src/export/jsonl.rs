//! JSON Lines — one object per line (`docs/38` §Formats).
//!
//! # The failure this file exists to prevent
//!
//! A file that is *nearly* JSONL. The format's whole value is that a consumer
//! can read it a line at a time without a streaming parser, and that breaks the
//! moment one record contains a raw newline or the file uses pretty-printed
//! objects. Both are easy to produce by accident and neither fails loudly — the
//! consumer reads half a record, gets a parse error, and blames its own code.
//!
//! So: `serde_json::to_string` (never `to_string_pretty`), one `\n` after each
//! record, and nothing else on the line.
//!
//! # Why this is not formula-escaped
//!
//! [`super::csv`] prefixes a leading `=` with an apostrophe because a
//! spreadsheet executes it. Doing that here would corrupt the data for the only
//! consumer this format has. `docs/38` gives the two formats different audiences
//! for exactly this reason: CSV is for spreadsheets, JSONL is "for pipelines and
//! scripts", and a pipeline that shells out to `eval` on a field has a problem
//! no exporter can fix.

use serde_json::{Map, Value};

/// Render one record as a JSONL line, newline included.
///
/// # Errors
///
/// Never in practice — the values come from a `Map<String, Value>`, which is
/// always serialisable. Returned as a `Result` rather than unwrapped so a future
/// value type that *can* fail cannot turn an export into a panic in a worker.
pub fn line(record: &Map<String, Value>) -> Result<String, serde_json::Error> {
    let mut out = serde_json::to_string(record)?;
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record() -> Map<String, Value> {
        json!({"key": "TF-1", "title": "Fix the login page"})
            .as_object()
            .expect("an object")
            .clone()
    }

    #[test]
    fn a_record_is_one_line() {
        let rendered = line(&record()).expect("serialises");
        assert_eq!(
            rendered.matches('\n').count(),
            1,
            "a record spanned more than one line, so a line-at-a-time reader \
             cannot parse it: {rendered}"
        );
        assert!(rendered.ends_with('\n'));
    }

    #[test]
    fn a_title_containing_a_newline_does_not_break_the_line_discipline() {
        // The failure this format dies of. A task description with a newline in
        // it is completely ordinary, and an exporter that wrote it raw would
        // produce a file that parses for the first thousand records and not the
        // thousand-and-first.
        let mut map = record();
        map.insert("title".to_owned(), json!("first\nsecond"));
        let rendered = line(&map).expect("serialises");
        assert_eq!(
            rendered.matches('\n').count(),
            1,
            "a raw newline reached the output: {rendered}"
        );
        assert!(rendered.contains(r"first\nsecond"), "{rendered}");
    }

    #[test]
    fn each_line_parses_back_on_its_own() {
        // The property a consumer relies on: split on newline, parse each piece.
        let mut file = String::new();
        for n in 0..3 {
            let mut map = record();
            map.insert("n".to_owned(), json!(n));
            file.push_str(&line(&map).expect("serialises"));
        }
        let parsed: Vec<Value> = file
            .lines()
            .map(|l| serde_json::from_str(l).expect("each line is a whole object"))
            .collect();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[2]["n"], 2);
    }

    #[test]
    fn a_formula_is_left_exactly_as_it_was() {
        // Deliberate, and the counterweight to the CSV escaping test: adding an
        // apostrophe here would corrupt the value for the pipeline this format
        // exists to feed.
        let mut map = record();
        map.insert("title".to_owned(), json!("=1+1"));
        let rendered = line(&map).expect("serialises");
        let parsed: Value = serde_json::from_str(rendered.trim_end()).expect("parses");
        assert_eq!(
            parsed["title"], "=1+1",
            "JSONL must not apply the spreadsheet defence; it changes the data"
        );
    }
}
