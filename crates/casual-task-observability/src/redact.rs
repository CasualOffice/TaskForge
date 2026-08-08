//! The redaction guard.
//!
//! `docs/46-OBSERVABILITY-AND-OPERATIONS.md` §What is not logged: task titles,
//! descriptions, and comment bodies are customer content and do not belong in
//! operational logs. IDs are logged; content is not.
//!
//! The document also says the primary control is that content is never passed to
//! the logger in the first place — a scrubber is the last resort. [`Redacted<T>`]
//! is that primary control, expressed as a type: the three formatting traits a
//! logger reaches for (`Debug`, `Display`, `Serialize`) all print
//! [`PLACEHOLDER`], so a wrapped title routed into a log field produces a line
//! that is visibly wrong in review and in the log itself, rather than a silent
//! leak.
//!
//! Reading the value back requires calling [`Redacted::expose`], which is a
//! deliberately unpleasant, greppable name (`docs/10` §3 — make the wrong thing
//! hard).

use std::fmt;

use serde::{Serialize, Serializer};

/// What every formatting impl on [`Redacted`] prints instead of the value.
pub const PLACEHOLDER: &str = "<redacted>";

/// A value that must never reach the logger.
///
/// `Debug`, `Display`, and `Serialize` all yield [`PLACEHOLDER`] regardless of
/// `T`. Note that the impls carry **no bound on `T`** — wrapping a type removes
/// its formatting rather than delegating to it, so there is no path by which the
/// inner value formats itself.
///
/// ```
/// use casual_task_observability::Redacted;
///
/// let title = Redacted::new("Migrate the billing ledger");
/// assert_eq!(format!("{title}"), "<redacted>");
/// assert_eq!(format!("{title:?}"), "<redacted>");
/// tracing::info!(task_title = %title, "task created"); // logs the placeholder
///
/// // The content is still reachable where it is legitimately needed, through a
/// // name that shows up in review and in `git grep expose`.
/// assert_eq!(*title.expose(), "Migrate the billing ledger");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    /// Wrap a value so it cannot be formatted into a log line.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Read the wrapped value.
    ///
    /// Named to be conspicuous: every call site is a claim that this particular
    /// use is not a log, a metric label, or an error message.
    pub const fn expose(&self) -> &T {
        &self.0
    }

    /// Consume the wrapper and return the value.
    pub fn into_inner(self) -> T {
        self.0
    }

    /// Apply a function to the wrapped value, keeping the result wrapped.
    ///
    /// Present so that deriving one piece of customer content from another
    /// (a truncated title, a normalized comment body) does not require an
    /// [`expose`](Self::expose) call that a reviewer must then re-audit.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Redacted<U> {
        Redacted(f(self.0))
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PLACEHOLDER)
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PLACEHOLDER)
    }
}

/// Structured JSON logs serialize their fields; this must redact there too,
/// otherwise `docs/46`'s rule holds for `Display` and leaks through `serde`.
impl<T> Serialize for Redacted<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(PLACEHOLDER)
    }
}

impl<T> From<T> for Redacted<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

/// Ergonomic wrapping: `title.redacted()`.
pub trait Redact: Sized {
    /// Wrap `self` so it cannot be formatted into a log line.
    fn redacted(self) -> Redacted<Self> {
        Redacted::new(self)
    }
}

impl<T: Sized> Redact for T {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A title chosen to be unmistakable if it ever appears in output.
    const SECRET_TITLE: &str = "Acme Corp Q3 layoff plan";

    #[test]
    fn display_prints_the_placeholder_not_the_content() {
        let title = Redacted::new(SECRET_TITLE.to_owned());
        let rendered = format!("{title}");
        assert!(
            !rendered.contains(SECRET_TITLE),
            "customer content leaked through Display: {rendered}"
        );
        assert_eq!(rendered, PLACEHOLDER);
    }

    #[test]
    fn debug_prints_the_placeholder_not_the_content() {
        let body = SECRET_TITLE.to_owned().redacted();
        let rendered = format!("{body:?}");
        assert!(
            !rendered.contains(SECRET_TITLE),
            "customer content leaked through Debug: {rendered}"
        );
        assert_eq!(rendered, PLACEHOLDER);
    }

    #[test]
    fn debug_of_a_containing_struct_redacts_the_field() {
        // The realistic leak: someone derives Debug on a command struct and
        // logs the whole thing. The field type has to defend itself.
        #[derive(Debug)]
        struct CreateTask {
            task_id: u32,
            title: Redacted<String>,
        }

        let cmd = CreateTask {
            task_id: 7,
            title: Redacted::new(SECRET_TITLE.to_owned()),
        };
        let rendered = format!("{cmd:?}");
        assert!(
            !rendered.contains(SECRET_TITLE),
            "customer content leaked through a derived Debug: {rendered}"
        );
        assert!(rendered.contains("task_id: 7"), "IDs are still logged");
        assert!(rendered.contains(PLACEHOLDER));
        assert_eq!(cmd.task_id, 7);
        assert_eq!(cmd.title.expose(), SECRET_TITLE);
    }

    #[test]
    fn serialization_prints_the_placeholder_not_the_content() {
        let value = Redacted::new(SECRET_TITLE.to_owned());
        let json = serde_json::to_string(&value).expect("Redacted always serializes");
        assert!(
            !json.contains(SECRET_TITLE),
            "customer content leaked through serde: {json}"
        );
        assert_eq!(json, format!("\"{PLACEHOLDER}\""));
    }

    #[test]
    fn nested_and_optional_content_stays_redacted() {
        // Vec<Redacted<_>> and Option<Redacted<_>> are the shapes a comment
        // thread or an optional description actually take.
        let comments = vec![
            Redacted::new(SECRET_TITLE.to_owned()),
            Redacted::new("second body".to_owned()),
        ];
        let description: Option<Redacted<String>> = Some(Redacted::new(SECRET_TITLE.to_owned()));

        let rendered = format!("{comments:?} {description:?}");
        assert!(
            !rendered.contains(SECRET_TITLE) && !rendered.contains("second body"),
            "content leaked from a collection: {rendered}"
        );
    }

    #[test]
    fn map_keeps_the_value_wrapped() {
        let title = Redacted::new(SECRET_TITLE.to_owned());
        let length = title.clone().map(|t| t.len());
        assert_eq!(format!("{length:?}"), PLACEHOLDER);
        assert_eq!(*length.expose(), SECRET_TITLE.len());
    }

    #[test]
    fn expose_returns_the_original_value() {
        let title = Redacted::new(SECRET_TITLE.to_owned());
        assert_eq!(title.expose(), SECRET_TITLE);
        assert_eq!(title.into_inner(), SECRET_TITLE);
    }
}
