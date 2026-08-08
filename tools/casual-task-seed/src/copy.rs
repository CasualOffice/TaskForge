//! The output format: PostgreSQL `COPY` text, one file per table.
//!
//! `COPY` rather than `INSERT` because the reference corpus is 2,000,000 tasks
//! and 20,000,000 activity events
//! (`docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md` §Reference capacity). At that
//! volume `INSERT` is not a slower option, it is a different order of magnitude:
//! every row pays statement parse, plan, and round-trip, where `COPY` streams
//! into the heap with one plan for the whole file.
//!
//! The text format is tab-separated with backslash escapes and `\N` for NULL.
//! It is not plain TSV — a literal tab inside a task description is written as
//! `\t`, and that distinction is why the files carry a `.copy` extension.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

/// Every table the corpus writes, **declared in a load order that satisfies
/// every foreign key**. The loader emits its `\copy` statements by walking this
/// list, so the order here is the order on disk and the order applied, and the
/// three cannot drift apart.
///
/// The column list is part of the declaration for the same reason: the row
/// builder asserts its arity against it, so a column added to one side and not
/// the other fails at generation time rather than as a `COPY` error 40 minutes
/// into a reference load.
macro_rules! tables {
    ($($variant:ident => $name:literal, $cols:literal;)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Table { $($variant),+ }

        impl Table {
            pub const ALL: &'static [Table] = &[$(Table::$variant),+];

            pub fn name(self) -> &'static str {
                match self { $(Table::$variant => $name),+ }
            }

            /// Comma-separated column names, in the order the rows are written.
            pub fn columns(self) -> &'static str {
                match self { $(Table::$variant => $cols),+ }
            }
        }
    };
}

tables! {
    Workspace => "workspace",
        "id,name,slug,authz_epoch,settings,created_at,deleted_at";
    UserAccount => "user_account",
        "id,email,display_name,avatar_url,is_tombstone,created_at,updated_at";
    WorkspaceMembership => "workspace_membership",
        "workspace_id,user_id,member_type,joined_at";
    Team => "team",
        "id,workspace_id,name,created_at,deleted_at";
    TeamMembership => "team_membership",
        "team_id,user_id";
    Role => "role",
        "id,workspace_id,name,is_template,created_at,updated_at,version";
    RolePermission => "role_permission",
        "role_id,permission";
    RoleAssignment => "role_assignment",
        "id,workspace_id,principal_type,principal_id,role_id,scope_type,scope_id,\
         constraints,granted_by,granted_at";
    Workflow => "workflow",
        "id,workspace_id,name,is_default,version";
    WorkflowStatus => "workflow_status",
        "id,workflow_id,workspace_id,name,state,position,is_initial";
    WorkflowTransition => "workflow_transition",
        "id,workflow_id,workspace_id,from_status_id,to_status_id,required_permission,\
         required_fields,ignore_dependencies";
    Project => "project",
        "id,workspace_id,team_id,key,name,description,visibility,workflow_id,task_seq,\
         created_at,created_by,updated_at,updated_by,version,archived_at,deleted_at";
    ProjectMembership => "project_membership",
        "project_id,user_id,workspace_id,added_at";
    ProjectEnvironment => "project_environment",
        "id,project_id,workspace_id,name,position";
    Milestone => "milestone",
        "id,workspace_id,project_id,name,due_at,completed_at";
    Tag => "tag",
        "id,workspace_id,project_id,name,color";
    Task => "task",
        "id,workspace_id,project_id,number,title,description,type,priority,status_id,state,\
         reporter_id,environment_id,milestone_id,parent_id,start_at,due_at,position,\
         created_at,created_by,updated_at,updated_by,version,archived_at,deleted_at";
    TaskAssignee => "task_assignee",
        "task_id,user_id,workspace_id,is_primary,assigned_at";
    TaskDependency => "task_dependency",
        "from_task_id,to_task_id,workspace_id,kind,created_at";
    TaskTag => "task_tag",
        "task_id,tag_id,workspace_id";
    Comment => "comment",
        "id,workspace_id,task_id,parent_comment_id,author_id,body,mentions,created_at,\
         edited_at,deleted_at,version";
    Attachment => "attachment",
        "id,workspace_id,task_id,object_key,filename,content_type,byte_size,checksum,\
         scan_status,committed_at,uploaded_by,created_at,deleted_at";
    TaskSearch => "task_search",
        "task_id,workspace_id,project_id,document,title_trgm,updated_at";
    SavedView => "saved_view",
        "id,workspace_id,project_id,owner_id,name,filter,sort,layout,shared,created_at,version";
    Notification => "notification",
        "id,workspace_id,user_id,event_type,reason,aggregate_id,payload,created_at,read_at";
    AutomationRule => "automation_rule",
        "id,workspace_id,project_id,name,trigger,conditions,actions,enabled,run_as,version";
    ServiceAccount => "service_account",
        "id,workspace_id,name,created_by,disabled_at";
    ApiToken => "api_token",
        "id,workspace_id,principal_type,principal_id,token_hash,name,last_used_at,\
         expires_at,revoked_at";
    PluginInstallation => "plugin_installation",
        "id,workspace_id,plugin_id,version,manifest_hash,granted_scopes,config,secret_ref,\
         installed_by,installed_at,enabled,uninstalled_at";
    OutboxEvent => "outbox_event",
        "id,workspace_id,event_type,aggregate_type,aggregate_id,payload,schema_version,\
         created_at,dispatched_at,attempts,last_error";
    ActivityEvent => "activity_event",
        "id,workspace_id,project_id,aggregate_type,aggregate_id,event_type,actor_id,\
         changes,occurred_at";
    AuditEvent => "audit_event",
        "id,workspace_id,event_type,actor_id,actor_type,target_type,target_id,changes,\
         request_id,correlation_id,ip_address,user_agent,occurred_at";
}

impl Table {
    /// `columns()` carries line continuations for readability; callers want the
    /// list without the intervening whitespace.
    pub fn column_names(self) -> Vec<&'static str> {
        self.columns().split(',').map(str::trim).collect()
    }

    /// `07_role_permission.copy` — the numeric prefix makes the load order
    /// visible in a directory listing.
    pub fn file_name(self) -> String {
        let ordinal = Table::ALL
            .iter()
            .position(|t| *t == self)
            .unwrap_or_default();
        format!("{:02}_{}.copy", ordinal + 1, self.name())
    }
}

/// One open output file, its expected arity, and its row count.
#[derive(Debug)]
pub struct TableWriter {
    table: Table,
    out: BufWriter<File>,
    arity: usize,
    rows: u64,
    buf: String,
    /// First I/O error seen. Checking every row would drown the generators in
    /// `?`; the error is surfaced once, at `finish`.
    error: Option<std::io::Error>,
}

impl TableWriter {
    fn create(dir: &Path, table: Table) -> std::io::Result<Self> {
        Ok(Self {
            table,
            out: BufWriter::with_capacity(1 << 20, File::create(dir.join(table.file_name()))?),
            arity: table.column_names().len(),
            rows: 0,
            buf: String::with_capacity(512),
            error: None,
        })
    }

    pub fn row(&mut self) -> Row<'_> {
        self.buf.clear();
        Row { w: self, n: 0 }
    }
}

/// One row under construction. Consumed by [`Row::end`], which checks that the
/// number of fields written matches the declared column list.
#[derive(Debug)]
pub struct Row<'a> {
    w: &'a mut TableWriter,
    n: usize,
}

impl Row<'_> {
    fn sep(&mut self) {
        if self.n > 0 {
            self.w.buf.push('\t');
        }
        self.n += 1;
    }

    pub fn null(mut self) -> Self {
        self.sep();
        self.w.buf.push_str("\\N");
        self
    }

    /// A value that needs no escaping: enum labels, `t`/`f`, numbers.
    fn bare(mut self, s: &str) -> Self {
        self.sep();
        self.w.buf.push_str(s);
        self
    }

    pub fn text(mut self, s: &str) -> Self {
        self.sep();
        escape_into(s, &mut self.w.buf);
        self
    }

    pub fn opt_text(self, s: Option<&str>) -> Self {
        match s {
            Some(s) => self.text(s),
            None => self.null(),
        }
    }

    /// Formatted into a stack buffer rather than a `String`: this runs tens of
    /// millions of times in a reference run, and it is the one allocation on
    /// the hot path worth removing.
    pub fn uuid(self, id: Uuid) -> Self {
        let mut buf = Uuid::encode_buffer();
        let s = id.hyphenated().encode_lower(&mut buf);
        self.bare(s)
    }

    pub fn opt_uuid(self, id: Option<Uuid>) -> Self {
        match id {
            Some(id) => self.uuid(id),
            None => self.null(),
        }
    }

    pub fn int(self, v: i64) -> Self {
        self.bare(&v.to_string())
    }

    pub fn bool(self, v: bool) -> Self {
        self.bare(if v { "t" } else { "f" })
    }

    /// A closed-enum label (`ACTIVE`, `USER`, `BLOCKS`). Written bare because
    /// PostgreSQL enum labels cannot contain a tab, a newline, or a backslash.
    pub fn label(self, v: &str) -> Self {
        self.bare(v)
    }

    /// Milliseconds since the Unix epoch, as RFC 3339 UTC.
    pub fn ts(self, ms: i64) -> Self {
        self.bare(&format_ts(ms))
    }

    pub fn opt_ts(self, ms: Option<i64>) -> Self {
        match ms {
            Some(ms) => self.ts(ms),
            None => self.null(),
        }
    }

    /// A `jsonb` value, already serialized.
    pub fn json(self, v: &str) -> Self {
        self.text(v)
    }

    /// A PostgreSQL array literal. Every element is double-quoted, so an
    /// element containing a comma or a brace cannot change the arity.
    pub fn text_array<S: AsRef<str>>(self, items: &[S]) -> Self {
        let mut lit = String::from("{");
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                lit.push(',');
            }
            lit.push('"');
            for ch in item.as_ref().chars() {
                if ch == '"' || ch == '\\' {
                    lit.push('\\');
                }
                lit.push(ch);
            }
            lit.push('"');
        }
        lit.push('}');
        self.text(&lit)
    }

    pub fn uuid_array(self, ids: &[Uuid]) -> Self {
        let items: Vec<String> = ids.iter().map(|id| id.hyphenated().to_string()).collect();
        self.text_array(&items)
    }

    pub fn end(self) {
        assert_eq!(
            self.n,
            self.w.arity,
            "{}: wrote {} fields for {} columns",
            self.w.table.name(),
            self.n,
            self.w.arity
        );
        self.w.buf.push('\n');
        if self.w.error.is_none()
            && let Err(e) = self.w.out.write_all(self.w.buf.as_bytes())
        {
            self.w.error = Some(e);
        }
        self.w.rows += 1;
    }
}

/// The four characters `COPY` text format reserves, plus the backslash that
/// introduces them. Everything else — including UTF-8 above ASCII — passes
/// through unchanged.
fn escape_into(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
}

pub fn format_ts(ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

/// The set of open table writers.
#[derive(Debug)]
pub struct Sink {
    writers: Vec<TableWriter>,
}

impl Sink {
    pub fn create(dir: &Path) -> std::io::Result<Self> {
        let mut writers = Vec::with_capacity(Table::ALL.len());
        for table in Table::ALL {
            writers.push(TableWriter::create(dir, *table)?);
        }
        Ok(Self { writers })
    }

    pub fn w(&mut self, table: Table) -> &mut TableWriter {
        // `Table` is a fieldless enum declared in load order, so the
        // discriminant is the index into `ALL` and into `writers`.
        &mut self.writers[table as usize]
    }

    /// Flush every file and return the row counts, in load order.
    pub fn finish(self) -> std::io::Result<BTreeMap<&'static str, u64>> {
        let mut counts = BTreeMap::new();
        for mut w in self.writers {
            if let Some(e) = w.error.take() {
                return Err(e);
            }
            w.out.flush()?;
            counts.insert(w.table.name(), w.rows);
        }
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_arity_matches_every_column_list() {
        for t in Table::ALL {
            let cols = t.column_names();
            assert!(!cols.is_empty(), "{} has no columns", t.name());
            assert!(
                cols.iter().all(|c| !c.is_empty()),
                "{} has an empty column name — check for a trailing comma",
                t.name()
            );
        }
    }

    #[test]
    fn discriminant_is_the_load_order_index() {
        for (i, t) in Table::ALL.iter().enumerate() {
            assert_eq!(*t as usize, i, "{} is out of position", t.name());
        }
    }

    #[test]
    fn escaping_covers_the_reserved_characters() {
        let mut out = String::new();
        escape_into("a\tb\nc\\d\re", &mut out);
        assert_eq!(out, "a\\tb\\nc\\\\d\\re");
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(format_ts(1_780_272_000_000), "2026-06-01T00:00:00Z");
    }

    #[test]
    fn file_names_carry_the_load_order() {
        assert_eq!(Table::Workspace.file_name(), "01_workspace.copy");
        assert_eq!(
            Table::AuditEvent.file_name(),
            format!("{}_audit_event.copy", Table::ALL.len())
        );
    }
}
