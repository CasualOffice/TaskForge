//! CSV, RFC 4180, with the formula-injection defence `docs/38` §83 requires.
//!
//! # The failure this file exists to prevent
//!
//! Remote code execution on a colleague's laptop, caused by someone with
//! permission to create a task.
//!
//! `docs/38` states it plainly: a cell whose first character is `=`, `+`, `-` or
//! `@` is executed as a formula when the file is opened in Excel or Sheets. A
//! task titled `=cmd|'/c calc'!A1` is a payload, the export is the delivery
//! mechanism, and the person who opens it is not the person who wrote it.
//!
//! > Every exported cell whose first character is one of those is prefixed with
//! > a single quote. This is non-negotiable and has its own test — it is the
//! > single most commonly shipped export vulnerability.
//!
//! # Two things that are easy to get subtly wrong here
//!
//! **Escaping must happen before quoting, not after.** `"=1+1"` is a *quoted
//! CSV field* whose content is still `=1+1`, and Excel parses the CSV first and
//! the formula second. Quoting is not a defence; it is orthogonal to one.
//!
//! **The leading-character check must not trim first.** A cell of `\t=1+1` is
//! whitespace to a CSV reader and a formula to Excel, which strips the tab
//! before parsing. So a leading tab or carriage return counts as dangerous in
//! its own right rather than being skipped over to find the real first
//! character.

use std::fmt::Write as _;

/// Characters that begin a formula in Excel, Sheets and LibreOffice.
///
/// `\t` and `\r` are in the list for the reason in the module docs: they are
/// invisible to a CSV reader and stripped by the spreadsheet, so they smuggle
/// the character that follows into first position.
const DANGEROUS_LEAD: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// `docs/38`: "UTF-8 with BOM so Excel opens it correctly".
///
/// Without it Excel on Windows reads a UTF-8 file as the local code page, and
/// every non-ASCII title arrives mojibake. It is three bytes, written once, and
/// it is the difference between an export that works for a German customer and
/// one that does not.
pub const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Render one row as an RFC 4180 line, terminated with CRLF.
///
/// CRLF because RFC 4180 says so and because Excel's importer is the reason
/// this format was chosen; a bare LF is tolerated by most readers and not by
/// all of them.
#[must_use]
pub fn row(cells: &[String]) -> String {
    let mut out = String::new();
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{}", field(cell));
    }
    out.push_str("\r\n");
    out
}

/// One cell: neutralised, then quoted if RFC 4180 requires it.
#[must_use]
pub fn field(value: &str) -> String {
    let neutral = neutralise(value);
    if needs_quoting(&neutral) {
        // RFC 4180: a literal quote inside a quoted field is doubled.
        format!("\"{}\"", neutral.replace('"', "\"\""))
    } else {
        neutral
    }
}

/// Prefix a formula-leading cell with a single quote.
///
/// The apostrophe is the spreadsheet convention for "this is text": Excel,
/// Sheets and LibreOffice all consume it and display the original string, so a
/// legitimate title beginning with `-` still *reads* correctly for a human.
///
/// The cost, stated: a value that genuinely began with an apostrophe now shows
/// two in some readers, and a CSV consumed by a *script* rather than a
/// spreadsheet sees the extra character. `docs/38` accepts that trade explicitly
/// — JSONL exists for pipelines, and is not escaped this way, because a JSON
/// parser has never executed a cell.
#[must_use]
pub fn neutralise(value: &str) -> String {
    match value.chars().next() {
        Some(first) if DANGEROUS_LEAD.contains(&first) => format!("'{value}"),
        _ => value.to_owned(),
    }
}

/// RFC 4180: quote when the field contains a comma, a quote, CR or LF.
fn needs_quoting(value: &str) -> bool {
    value.contains([',', '"', '\n', '\r'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_formula_title_is_inert() {
        // docs/38's own example. This is the test that document says the
        // defence must have.
        let payload = "=cmd|'/c calc'!A1";
        let rendered = field(payload);
        assert!(
            rendered.starts_with('\'') || rendered.starts_with("\"'"),
            "a formula reached the file unneutralised: {rendered}"
        );
        assert!(
            !rendered.starts_with('='),
            "the cell still begins with = and will execute on open: {rendered}"
        );
    }

    #[test]
    fn every_documented_lead_character_is_neutralised() {
        for lead in ['=', '+', '-', '@'] {
            let value = format!("{lead}1+1");
            assert_eq!(
                neutralise(&value),
                format!("'{value}"),
                "a cell beginning {lead} was left executable"
            );
        }
    }

    #[test]
    fn an_invisible_lead_character_does_not_smuggle_a_formula_through() {
        // The bypass a naive implementation ships with: a tab is whitespace to
        // a CSV reader and is stripped by Excel, which then parses what follows
        // as a formula.
        for lead in ['\t', '\r'] {
            let value = format!("{lead}=1+1");
            assert!(
                neutralise(&value).starts_with('\''),
                "a cell beginning with an invisible character reached the file \
                 with a formula behind it"
            );
        }
    }

    #[test]
    fn quoting_is_not_mistaken_for_a_defence() {
        // A value containing a comma is quoted by RFC 4180 — and quoting does
        // nothing about the formula, because Excel parses the CSV first and the
        // cell content second. The neutralisation must be INSIDE the quotes.
        let rendered = field("=1+1,x");
        assert_eq!(
            rendered, "\"'=1+1,x\"",
            "the apostrophe must be inside the quotes, applied to the value"
        );
    }

    #[test]
    fn ordinary_text_is_untouched() {
        // The counterweight: an escaper that prefixed everything would pass
        // every test above and make every export ugly.
        assert_eq!(field("Fix the login page"), "Fix the login page");
        assert_eq!(field("TF-1024"), "TF-1024");
        assert_eq!(field(""), "");
    }

    #[test]
    fn rfc_4180_quoting_survives_the_awkward_characters() {
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(field("line\nbreak"), "\"line\nbreak\"");
    }

    #[test]
    fn a_row_is_comma_separated_and_crlf_terminated() {
        assert_eq!(
            row(&["a".to_owned(), "b,c".to_owned()]),
            "a,\"b,c\"\r\n",
            "RFC 4180 terminates records with CRLF"
        );
    }

    #[test]
    fn a_row_of_payloads_is_entirely_inert() {
        // The property that matters at the row level: one safe cell beside an
        // unsafe one must not make the row safe by accident.
        let rendered = row(&["ok".to_owned(), "=1+1".to_owned(), "@SUM(A1)".to_owned()]);
        for cell in rendered.trim_end().split(',') {
            let unquoted = cell.trim_matches('"');
            assert!(
                !unquoted.starts_with(['=', '+', '-', '@']),
                "cell {cell} is executable"
            );
        }
    }

    #[test]
    fn the_bom_is_the_utf8_one() {
        // docs/38 asks for UTF-8 with BOM specifically. A UTF-16 BOM would make
        // Excel misread every byte of the file.
        assert_eq!(BOM, [0xEF, 0xBB, 0xBF]);
    }
}
