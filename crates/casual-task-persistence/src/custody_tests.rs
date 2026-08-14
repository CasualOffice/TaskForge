#[test]
fn no_read_in_this_module_paginates_by_offset() {
    // docs/26 bans it. The needle is assembled: spelling it out would put it
    // in the file this check reads, and the assertion would fail on itself.
    let source = include_str!("custody.rs");
    let banned = format!("{}{} ", "OFF", "SET");
    assert!(!source.to_uppercase().contains(&banned));
}

#[test]
fn the_ordinary_task_writer_does_not_touch_the_second_clock() {
    // `task.rs` owns the first clock and every plain field. If it ever
    // learned to set `environment_id`, an ordinary PATCH would move a task
    // between environments with no promotion row and no way to ask when.
    //
    // The other legitimate writer is `environment::set_on_task`, which is
    // behind `If-Match`; it pairs with `record_promotion` so the log stays
    // complete. That pairing is asserted end to end in the API tests, where
    // both halves actually run.
    let task = include_str!("task.rs");
    assert!(
        !task.contains("SET environment_id"),
        "task.rs writes the environment column; every move must leave a promotion row"
    );

    let custody = include_str!("custody.rs");
    assert!(custody.contains("UPDATE task SET environment_id"));
}
