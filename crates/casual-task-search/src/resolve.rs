//! Resolving symbolic values (`docs/27` §Symbolic values).
//!
//! A saved view stores `@me`, not a user id. That is what makes one view — "My
//! overdue work" — correct for everybody who opens it; `docs/27`: "A view that
//! hardcoded a user id would be shareable but wrong." So symbols survive
//! storage and validation, and are resolved here, at evaluation.
//!
//! # Timezone is the actor's, and this is the bug the design names
//!
//! `docs/27`: "`due before @today` must mean the same thing to someone in
//! Auckland and someone in Los Angeles. Server-local date boundaries are a
//! classic and extremely confusing bug."
//!
//! So [`Context`] carries a [`UtcOffset`] and there is **no default**. A
//! context cannot be built without one, which means a caller cannot
//! accidentally resolve `@today` against the server's midnight.
//!
//! **What the offset is, and what it is not.** It is the offset in effect for
//! the actor *at evaluation time*, which the API layer derives from their IANA
//! zone. That is correct for the day boundaries these symbols compute, and it
//! is deliberately not an attempt to model a timezone: an offset cannot answer
//! "what will midnight be in three weeks", and nothing here asks it to.

use casual_task_model::{TeamId, UserId};
use time::{Duration, OffsetDateTime, UtcOffset};

use crate::filter::{Clause, Field, Node, Operator, Value};

/// Everything a symbol can resolve against.
#[derive(Debug, Clone)]
pub struct Context {
    actor: UserId,
    teams: Vec<TeamId>,
    now: OffsetDateTime,
    /// The actor's offset at `now`. No default — see the module docs.
    offset: UtcOffset,
}

impl Context {
    pub fn new(actor: UserId, teams: Vec<TeamId>, now: OffsetDateTime, offset: UtcOffset) -> Self {
        Self {
            actor,
            teams,
            now,
            offset,
        }
    }

    /// Midnight today, in the **actor's** offset, as an instant.
    fn start_of_today(&self) -> OffsetDateTime {
        let local = self.now.to_offset(self.offset);
        local.replace_time(time::Time::MIDNIGHT)
    }
}

/// Why a symbol could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// Not a symbol this version understands. Unknown symbols are refused
    /// rather than passed through: a typo'd `@tomorow` reaching the database as
    /// a literal would compare a timestamp against a string and fail there,
    /// which is a worse place to find out.
    UnknownSymbol(String),
    /// `@unassigned` is sugar for `assignee is_empty`, so it is meaningless on
    /// any other field.
    SymbolNotValidHere { symbol: String, field: Field },
}

/// Replace every symbol in the tree with a concrete value.
///
/// # Errors
///
/// [`ResolveError`] on the first symbol that cannot be resolved.
pub fn resolve(node: &Node, ctx: &Context) -> Result<Node, ResolveError> {
    Ok(match node {
        Node::And(children) => Node::And(try_all(children, ctx)?),
        Node::Or(children) => Node::Or(try_all(children, ctx)?),
        Node::Not(inner) => Node::Not(Box::new(resolve(inner, ctx)?)),
        Node::Clause(c) => resolve_clause(c, ctx)?,
    })
}

fn try_all(children: &[Node], ctx: &Context) -> Result<Vec<Node>, ResolveError> {
    children.iter().map(|c| resolve(c, ctx)).collect()
}

fn resolve_clause(c: &Clause, ctx: &Context) -> Result<Node, ResolveError> {
    let Value::Symbol(sym) = &c.value else {
        return Ok(Node::Clause(c.clone()));
    };

    // `@unassigned` rewrites the clause rather than the value — docs/27 calls
    // it "sugar for `assignee is_empty`", and sugar that produced a value
    // instead of the shape it stands for would need a special case downstream.
    if sym == "@unassigned" {
        if c.field != Field::Assignee {
            return Err(ResolveError::SymbolNotValidHere {
                symbol: sym.clone(),
                field: c.field,
            });
        }
        return Ok(Node::Clause(Clause {
            field: Field::Assignee,
            op: Operator::IsEmpty,
            value: Value::None,
        }));
    }

    let value = match sym.as_str() {
        "@me" => Value::Literal(ctx.actor.as_uuid().to_string()),
        "@my_teams" => Value::List(ctx.teams.iter().map(|t| t.as_uuid().to_string()).collect()),
        "@today" => instant(ctx.start_of_today()),
        "@tomorrow" => instant(ctx.start_of_today() + Duration::days(1)),
        "@start_of_week" => {
            let today = ctx.start_of_today();
            // Monday, per ISO 8601. Sunday-start locales exist and this is a
            // product decision rather than a computation — it is stated here so
            // that changing it is a deliberate act.
            let back = today.weekday().number_days_from_monday() as i64;
            instant(today - Duration::days(back))
        }
        other => match relative(other) {
            Some(d) => instant(ctx.now + d),
            None => return Err(ResolveError::UnknownSymbol(other.to_owned())),
        },
    };

    Ok(Node::Clause(Clause {
        field: c.field,
        op: c.op,
        value,
    }))
}

fn instant(t: OffsetDateTime) -> Value {
    // RFC 3339, so the database receives an unambiguous instant rather than a
    // local wall-clock reading whose meaning depends on the server.
    Value::Literal(
        t.format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
    )
}

/// `+7d`, `-30d`, `+1w`, `-3mo`.
///
/// Months are 30 days. That is an approximation, and it is the right one here:
/// the alternative — calendar months — makes `-3mo` land on a different day
/// depending on which months it crosses, which is harder to explain in a filter
/// than "about ninety days". Stated rather than hidden.
fn relative(s: &str) -> Option<Duration> {
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1i64, &s[1..]),
        b'-' => (-1i64, &s[1..]),
        _ => return None,
    };
    let split = rest.find(|c: char| !c.is_ascii_digit())?;
    let (digits, unit) = rest.split_at(split);
    let n: i64 = digits.parse().ok()?;
    let days = match unit {
        "d" => n,
        "w" => n * 7,
        "mo" => n * 30,
        _ => return None,
    };
    Some(Duration::days(sign * days))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn ctx_at(now: OffsetDateTime, hours: i8) -> Context {
        Context::new(
            UserId::new(),
            vec![TeamId::new()],
            now,
            UtcOffset::from_hms(hours, 0, 0).expect("valid offset"),
        )
    }

    fn clause(field: Field, op: Operator, v: Value) -> Node {
        Node::Clause(Clause {
            field,
            op,
            value: v,
        })
    }

    #[test]
    fn at_me_becomes_the_actor() {
        let ctx = ctx_at(datetime!(2026-08-08 12:00 UTC), 0);
        let out = resolve(
            &clause(Field::Assignee, Operator::Eq, Value::Symbol("@me".into())),
            &ctx,
        )
        .expect("resolves");
        let Node::Clause(c) = out else { panic!() };
        assert_eq!(c.value, Value::Literal(ctx.actor.as_uuid().to_string()));
    }

    #[test]
    fn today_is_the_actors_midnight_not_the_servers() {
        // docs/27's named bug. 2026-08-08 12:00 UTC is the 8th in Los Angeles
        // (UTC-7) and already the 9th in Auckland (UTC+12), so `@today` must
        // resolve to two different instants.
        let now = datetime!(2026-08-08 12:00 UTC);
        let la = resolve(
            &clause(
                Field::DueAt,
                Operator::Before,
                Value::Symbol("@today".into()),
            ),
            &ctx_at(now, -7),
        )
        .expect("resolves");
        let akl = resolve(
            &clause(
                Field::DueAt,
                Operator::Before,
                Value::Symbol("@today".into()),
            ),
            &ctx_at(now, 12),
        )
        .expect("resolves");

        let (Node::Clause(a), Node::Clause(b)) = (la, akl) else {
            panic!()
        };
        assert_ne!(
            a.value, b.value,
            "the same instant resolved to the same midnight for two actors on \
             different sides of the date line — this is the server-local \
             boundary bug docs/27 warns about"
        );
        // And each is that actor's own midnight.
        let Value::Literal(la_s) = &a.value else {
            panic!()
        };
        let Value::Literal(akl_s) = &b.value else {
            panic!()
        };
        assert!(la_s.starts_with("2026-08-08T00:00:00"), "{la_s}");
        assert!(akl_s.starts_with("2026-08-09T00:00:00"), "{akl_s}");
    }

    #[test]
    fn relative_offsets_parse_the_documented_forms() {
        assert_eq!(relative("+7d"), Some(Duration::days(7)));
        assert_eq!(relative("-30d"), Some(Duration::days(-30)));
        assert_eq!(relative("+1w"), Some(Duration::days(7)));
        assert_eq!(relative("-3mo"), Some(Duration::days(-90)));
        // Not documented, so not accepted.
        assert_eq!(relative("7d"), None, "a sign is required");
        assert_eq!(relative("+1y"), None);
        assert_eq!(relative("+d"), None);
        assert_eq!(relative(""), None);
    }

    #[test]
    fn an_unknown_symbol_is_refused_rather_than_passed_through() {
        // A typo'd `@tomorow` reaching the database as a literal would compare
        // a timestamp against a string and fail there — a worse place to learn.
        let ctx = ctx_at(datetime!(2026-08-08 12:00 UTC), 0);
        assert_eq!(
            resolve(
                &clause(
                    Field::DueAt,
                    Operator::Before,
                    Value::Symbol("@tomorow".into())
                ),
                &ctx
            ),
            Err(ResolveError::UnknownSymbol("@tomorow".into()))
        );
    }

    #[test]
    fn unassigned_rewrites_the_clause_and_only_fits_assignee() {
        let ctx = ctx_at(datetime!(2026-08-08 12:00 UTC), 0);
        let out = resolve(
            &clause(
                Field::Assignee,
                Operator::Eq,
                Value::Symbol("@unassigned".into()),
            ),
            &ctx,
        )
        .expect("resolves");
        let Node::Clause(c) = out else { panic!() };
        assert_eq!(c.op, Operator::IsEmpty);
        assert_eq!(c.value, Value::None);

        assert!(matches!(
            resolve(
                &clause(
                    Field::Reporter,
                    Operator::Eq,
                    Value::Symbol("@unassigned".into())
                ),
                &ctx
            ),
            Err(ResolveError::SymbolNotValidHere { .. })
        ));
    }

    #[test]
    fn start_of_week_is_monday_in_the_actors_offset() {
        // 2026-08-08 is a Saturday.
        let ctx = ctx_at(datetime!(2026-08-08 12:00 UTC), 0);
        let out = resolve(
            &clause(
                Field::CreatedAt,
                Operator::After,
                Value::Symbol("@start_of_week".into()),
            ),
            &ctx,
        )
        .expect("resolves");
        let Node::Clause(c) = out else { panic!() };
        let Value::Literal(s) = c.value else { panic!() };
        assert!(s.starts_with("2026-08-03T00:00:00"), "{s}");
    }

    #[test]
    fn resolution_reaches_every_branch_of_the_tree() {
        // A symbol nested inside a group must not survive resolution — one that
        // did would reach the compiler and be bound as the literal "@me".
        let ctx = ctx_at(datetime!(2026-08-08 12:00 UTC), 0);
        let tree = Node::And(vec![Node::Not(Box::new(Node::Or(vec![clause(
            Field::Assignee,
            Operator::Eq,
            Value::Symbol("@me".into()),
        )])))]);
        let out = resolve(&tree, &ctx).expect("resolves");
        assert!(
            !format!("{out:?}").contains("Symbol"),
            "a symbol survived resolution: {out:?}"
        );
    }
}
