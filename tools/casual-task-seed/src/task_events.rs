/// Write the weighted search projection from the controlled stem vocabulary.
///
/// A recorded PostgreSQL 16 stem snapshot avoids two million function calls;
/// its cost is that vocabulary changes must update the snapshot test.
fn write_search(sink: &mut Sink, world: &World, task: &TaskCtx<'_>) {
    let mut lexemes: Vec<(String, usize, char)> = Vec::with_capacity(12);
    let mut push = |lex: String, position: usize, weight: char| {
        if !lexemes.iter().any(|(l, _, _)| *l == lex) {
            lexemes.push((lex, position, weight));
        }
    };

    // Positions count every token, including the dropped ones — that is how
    // the text search parser numbers them, and `ts_rank_cd` reads proximity
    // out of those numbers.
    let mut position = 0;
    for word in task.title.split_whitespace() {
        position += 1;
        let w: String = word
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_lowercase();
        if let Some(stem) = vocab::stem(&w) {
            push(stem.to_string(), position, 'A');
        }
    }
    // A key that picked up a numeric suffix (`CORE2`) is not in the stem table;
    // PostgreSQL treats such a token as a `numword` and leaves it alone, so the
    // raw form is the correct lexeme.
    let key = task.project.key.to_lowercase();
    let key_lex = vocab::stem(&key).map(str::to_string).unwrap_or(key);
    push(key_lex, position + 1, 'B');
    if let Some(stem) = vocab::stem(&labels::task_type(task.task_type).to_lowercase()) {
        push(stem.to_string(), position + 2, 'C');
    }

    let mut document = String::with_capacity(lexemes.len() * 14);
    for (i, (lex, position, weight)) in lexemes.iter().enumerate() {
        if i > 0 {
            document.push(' ');
        }
        document.push('\'');
        for ch in lex.chars() {
            if ch == '\'' {
                document.push('\''); // doubled, as tsvector input requires
            }
            document.push(ch);
        }
        document.push('\'');
        document.push(':');
        document.push_str(&position.to_string());
        document.push(*weight);
    }

    sink.w(Table::TaskSearch)
        .row()
        .uuid(task.id)
        .uuid(world.workspace_id)
        .uuid(task.project.id)
        .text(&document)
        .text(&task.title.to_lowercase())
        .ts(task.updated)
        .end();
}

/// The activity stream: one row per thing that happened, in the order it
/// happened. `changes` holds display **values**, not ids, because the stream is
/// rendered years later and must still read correctly after a status has been
/// renamed (`docs/25-EVENTS-OUTBOX-AND-AUDIT.md` §Activity).
fn write_activity(
    sink: &mut Sink,
    det: &mut Det,
    world: &World,
    task: &TaskCtx<'_>,
    shape: ActivityShape,
) {
    let workflow = &world.workflows[task.project.workflow];
    let window = (task.updated - task.created).max(1);
    let mut clock = task.created;
    let mut tick = |det: &mut Det| {
        clock += det.below((window / 4).max(1) as u64) as i64;
        clock.min(task.updated)
    };
    let mut emit = |det: &mut Det, event: &str, changes: &str, at: i64| {
        let actor = world.users[*det.pick(&task.project.members)].id;
        sink.w(Table::ActivityEvent)
            .row()
            .uuid(det.uuid_at(at))
            .uuid(world.workspace_id)
            .opt_uuid(Some(task.project.id))
            .text("task")
            .uuid(task.id)
            .text(event)
            // A minority of events are system-authored: automation and the
            // retention worker both write history with no user behind them,
            // which is why `actor_id` is nullable.
            .opt_uuid(if det.chance(970) { Some(actor) } else { None })
            .json(changes)
            .ts(at)
            .end();
    };

    emit(det, "task.created", "{}", task.created);

    for _ in 0..shape.assignees {
        let at = tick(det);
        emit(det, "task.assigned", "{}", at);
    }

    // How many status hops it took to reach the state the task is in. The last
    // hop lands on the task's actual status, so the history and the row agree.
    let hops = match task.state {
        TaskState::Backlog => 0,
        TaskState::Planned => 1,
        TaskState::Active | TaskState::Canceled => 2,
        TaskState::Completed => 3,
    };
    let mut from = 0usize;
    for h in 0..hops {
        let to = if h + 1 == hops {
            task.status
        } else {
            (h + 1).min(task.status)
        };
        if to == from {
            continue;
        }
        let at = tick(det);
        let changes = format!(
            r#"{{"status":{{"from":{},"to":{}}},"state":{{"from":"{}","to":"{}"}}}}"#,
            json_string(workflow.statuses[from].name),
            json_string(workflow.statuses[to].name),
            labels::state(workflow.statuses[from].state),
            labels::state(workflow.statuses[to].state)
        );
        emit(det, "task.status.changed", &changes, at);
        from = to;
    }
    // `task.closed` is emitted in addition to `task.status.changed`, not
    // instead of it (docs/25 §Event types).
    if task.state == TaskState::Completed {
        let at = tick(det);
        emit(det, "task.closed", "{}", at);
    }

    for _ in 0..shape.edits {
        let at = tick(det);
        let changes = match det.weighted(&[300, 250, 250, 200]) {
            0 => format!(
                r#"{{"priority":{{"from":"MEDIUM","to":"{}"}}}}"#,
                labels::priority(labels::PRIORITIES[det.weighted(&PRIORITY_WEIGHTS)])
            ),
            1 => format!(r#"{{"title":{{"to":{}}}}}"#, json_string(task.title)),
            2 => r#"{"due_at":{"from":null,"to":"changed"}}"#.to_string(),
            _ => r#"{"description":{"changed":true}}"#.to_string(),
        };
        emit(det, "task.updated", &changes, at);
    }

    for _ in 0..shape.comments {
        let at = tick(det);
        emit(det, "comment.created", "{}", at);
    }
}

/// Minimal JSON string escaping. The vocabulary contains no control characters,
/// but `changes` is a `jsonb` column and a malformed value fails the whole load
/// rather than degrading quietly, so the escape is not optional.
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn title(det: &mut Det) -> String {
    match det.weighted(&[350, 250, 200, 200]) {
        0 => format!(
            "{} {} in the {}",
            det.pick(vocab::VERBS),
            det.pick(vocab::NOUNS),
            det.pick(vocab::COMPONENTS)
        ),
        1 => format!(
            "{} the {} {}",
            det.pick(vocab::VERBS),
            det.pick(vocab::COMPONENTS),
            det.pick(vocab::QUALIFIERS)
        ),
        2 => format!(
            "{}: {} {}",
            det.pick(vocab::COMPONENTS),
            det.pick(vocab::NOUNS),
            det.pick(vocab::QUALIFIERS)
        ),
        _ => format!("{} {}", det.pick(vocab::VERBS), det.pick(vocab::NOUNS)),
    }
}

/// Multi-paragraph, with real newlines — which is also the cheapest way to keep
/// the `COPY` escaping honest, since an unescaped newline would end the row.
fn description(det: &mut Det) -> String {
    format!(
        "Observed in the {} {}.\n\nExpected: the {} stays within its budget.\nActual: {}\n\n\
         Notes:\n- {}\n- {}\n",
        det.pick(vocab::COMPONENTS),
        det.pick(vocab::QUALIFIERS),
        det.pick(vocab::NOUNS),
        det.pick(vocab::COMMENT_BODIES),
        det.pick(vocab::COMMENT_OPENERS),
        det.pick(vocab::COMMENT_BODIES),
    )
}
