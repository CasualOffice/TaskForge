use super::*;
use crate::metrics::{self, PLUGIN_CIRCUIT_STATE, RATE_LIMIT_HITS_TOTAL};

#[test]
fn a_declared_label_is_accepted() {
    let labels = LabelSet::for_metric(metrics::PERMISSION_DENIED_TOTAL)
        .with(keys::PERMISSION, "task.delete")
        .expect("permission is declared on permission_denied_total");
    assert_eq!(labels.pairs(), vec![("permission", "task.delete")]);
}

#[test]
fn a_tenant_bucket_cannot_be_smuggled_onto_another_key() {
    // Both keys are declared on this metric, so the key check passes and
    // only the value check can stop it. Before that check existed, this
    // produced `[("scope_kind", "019fe1ca-…")]` — a raw workspace UUID in a
    // metric label, which is the exact thing docs/46 forbids.
    let workspace = WorkspaceId::new();
    let mut allow = InvestigationAllowList::default();
    allow.admit(workspace).expect("empty list has room");

    let err = LabelSet::for_metric(RATE_LIMIT_HITS_TOTAL)
        .with(keys::SCOPE_KIND, allow.label(workspace))
        .expect_err("an admitted tenant id must not be usable as a scope kind");
    assert!(
        matches!(
            err,
            CardinalityError::MisplacedValue {
                key: "scope_kind",
                minted_for: "workspace_investigation",
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn an_installation_id_cannot_be_used_as_a_statement_name() {
    // `statement` is documented as bounded *because* statement names are
    // `&'static`. An installation id there is unbounded cardinality on a
    // histogram, which is the most expensive shape there is.
    let err = LabelSet::for_metric(metrics::DB_QUERY_DURATION)
        .with(
            keys::STATEMENT,
            LabelValue::plugin_installation(casual_task_model::PluginInstallationId::new()),
        )
        .expect_err("an installation id is not a statement name");
    assert!(
        matches!(
            err,
            CardinalityError::MisplacedValue {
                key: "statement",
                minted_for: "installation",
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn a_bounded_value_still_goes_where_it_belongs() {
    // The counterweight: the guard must not break the two legitimate uses.
    let bucket = LabelValue::workspace_bucket(WorkspaceId::new());
    LabelSet::for_metric(RATE_LIMIT_HITS_TOTAL)
        .with(keys::WORKSPACE_BUCKET, bucket)
        .expect("a bucket belongs on workspace_bucket");

    LabelSet::for_metric(PLUGIN_CIRCUIT_STATE)
        .with(
            keys::INSTALLATION,
            LabelValue::plugin_installation(casual_task_model::PluginInstallationId::new()),
        )
        .expect("an installation id belongs on installation");

    // And a source-text value stays usable on any declared key.
    LabelSet::for_metric(metrics::PERMISSION_DENIED_TOTAL)
        .with(keys::PERMISSION, "task.delete")
        .expect("a static value is safe anywhere");
}

#[test]
fn an_undeclared_label_is_rejected() {
    let err = LabelSet::for_metric(metrics::PERMISSION_DENIED_TOTAL)
        .with(keys::STATEMENT, "select_task")
        .expect_err("statement is not declared on permission_denied_total");
    assert!(matches!(
        err,
        CardinalityError::UndeclaredLabel {
            metric: "permission_denied_total",
            key: "statement",
            ..
        }
    ));
}

#[test]
fn no_metric_declares_a_raw_tenant_label() {
    // The guard from docs/46 §Cardinality discipline, asserted over the
    // whole registry so a new metric cannot reintroduce it.
    let banned = [
        "workspace_id",
        "workspace",
        "tenant",
        "tenant_id",
        "actor_id",
    ];
    for metric in metrics::ALL {
        for key in metric.labels() {
            assert!(
                !banned.contains(&key.as_str()),
                "metric `{}` declares `{}` — docs/46 forbids raw tenant labels",
                metric.name(),
                key
            );
        }
    }
}

#[test]
fn every_metric_stays_under_the_label_cap() {
    for metric in metrics::ALL {
        assert!(
            metric.labels().len() <= MAX_LABELS_PER_METRIC,
            "metric `{}` declares {} labels; cap is {MAX_LABELS_PER_METRIC}",
            metric.name(),
            metric.labels().len()
        );
    }
}

#[test]
fn bucketing_caps_the_series_count_regardless_of_tenant_count() {
    // The whole point: 10,000 tenants must not produce 10,000 series.
    let buckets: BTreeSet<_> = (0..10_000)
        .map(|_| WorkspaceBucket::of(WorkspaceId::new()).index())
        .collect();
    assert!(
        buckets.len() <= usize::from(WORKSPACE_BUCKET_COUNT),
        "bucketing produced {} distinct series",
        buckets.len()
    );
    assert!(
        buckets.iter().all(|b| *b < WORKSPACE_BUCKET_COUNT),
        "a bucket index escaped its range"
    );
    // A degenerate hash that maps everything to one bucket would also pass
    // the cap; assert the spread is real.
    assert!(
        buckets.len() > usize::from(WORKSPACE_BUCKET_COUNT) / 2,
        "bucket distribution is too concentrated to be diagnostic: {} used",
        buckets.len()
    );
}

#[test]
fn bucketing_is_stable_across_calls() {
    // A dashboard series must not move under a rolling deploy.
    let workspace = WorkspaceId::new();
    let first = WorkspaceBucket::of(workspace);
    for _ in 0..100 {
        assert_eq!(WorkspaceBucket::of(workspace), first);
    }
}

#[test]
fn a_bucket_label_never_contains_the_tenant_id() {
    let workspace = WorkspaceId::new();
    let value = LabelValue::workspace_bucket(workspace);
    assert!(
        !value.as_str().contains(&workspace.to_string()),
        "the tenant id leaked into a metric label: {value}"
    );
    let labels = LabelSet::for_metric(RATE_LIMIT_HITS_TOTAL)
        .with(keys::WORKSPACE_BUCKET, value)
        .expect("workspace_bucket is declared on rate_limit_hits_total");
    let rendered = format!("{:?}", labels.pairs());
    assert!(!rendered.contains(&workspace.to_string()));
}

#[test]
fn the_investigation_allow_list_is_bounded() {
    let mut list = InvestigationAllowList::new();
    assert!(list.is_empty(), "the steady state is no investigation");

    let admitted: Vec<_> = (0..InvestigationAllowList::MAX_ENTRIES)
        .map(|_| {
            let workspace = WorkspaceId::new();
            list.admit(workspace).expect("within the cap");
            workspace
        })
        .collect();
    assert_eq!(list.len(), InvestigationAllowList::MAX_ENTRIES);

    // Re-admitting does not consume a slot.
    list.admit(admitted[0]).expect("already admitted");
    assert_eq!(list.len(), InvestigationAllowList::MAX_ENTRIES);

    let err = list
        .admit(WorkspaceId::new())
        .expect_err("the cap is the mechanism");
    assert_eq!(
        err,
        CardinalityError::AllowListFull {
            max: InvestigationAllowList::MAX_ENTRIES
        }
    );

    list.revoke(admitted[0]);
    list.admit(WorkspaceId::new())
        .expect("revoking frees a slot");
}

#[test]
fn a_tenant_off_the_allow_list_collapses_to_other() {
    let mut list = InvestigationAllowList::new();
    let investigated = WorkspaceId::new();
    let ordinary = WorkspaceId::new();
    list.admit(investigated).expect("within the cap");

    assert_eq!(
        list.label(investigated).as_str(),
        investigated.to_string(),
        "an admitted tenant is labelled by id — the documented exception"
    );
    assert_eq!(list.label(ordinary), InvestigationAllowList::OTHER);
    assert!(
        !list
            .label(ordinary)
            .as_str()
            .contains(&ordinary.to_string()),
        "a non-admitted tenant id reached a label"
    );

    list.revoke(investigated);
    assert_eq!(
        list.label(investigated),
        InvestigationAllowList::OTHER,
        "revoking must stop the per-tenant series"
    );
}

#[test]
fn bool_and_static_str_are_the_only_generic_conversions() {
    assert_eq!(LabelValue::from(true).as_str(), "true");
    assert_eq!(LabelValue::from("open").as_str(), "open");
    let labels = LabelSet::for_metric(PLUGIN_CIRCUIT_STATE)
        .with(keys::CIRCUIT_STATE, "open")
        .expect("circuit_state is declared");
    assert_eq!(labels.pairs(), vec![("circuit_state", "open")]);
}

#[test]
fn labels_render_in_deterministic_order() {
    // Two identical series built in opposite insertion order must render
    // identically, or a dashboard sees two series where there is one.
    let forward = LabelSet::for_metric(metrics::HTTP_REQUEST_DURATION_SECONDS)
        .with(keys::METHOD, "GET")
        .and_then(|s| s.with(keys::ROUTE, "/v1/tasks/{id}"))
        .and_then(|s| s.with(keys::STATUS_CLASS, "2xx"))
        .expect("all three are declared on http_request_duration_seconds");
    let reverse = LabelSet::for_metric(metrics::HTTP_REQUEST_DURATION_SECONDS)
        .with(keys::STATUS_CLASS, "2xx")
        .and_then(|s| s.with(keys::ROUTE, "/v1/tasks/{id}"))
        .and_then(|s| s.with(keys::METHOD, "GET"))
        .expect("all three are declared on http_request_duration_seconds");

    assert_eq!(forward.pairs(), reverse.pairs());
    assert_eq!(
        forward.pairs(),
        vec![
            ("method", "GET"),
            ("route", "/v1/tasks/{id}"),
            ("status_class", "2xx"),
        ]
    );
}
