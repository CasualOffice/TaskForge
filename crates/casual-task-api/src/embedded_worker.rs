/// Start the dispatch loop inside the API process, if this deployment wants it.
///
/// Returns the cancellation handle. `None` when the loop is not running, for
/// one of two reasons, and **both are said out loud**: a silent no-op here is
/// how a deployment ends up with notifications and SSE that work in tests and
/// nowhere else, which is exactly the state D-060 described.
async fn start_embedded_worker(
    config: &casual_task_api::Config,
    state: &AppState,
) -> Option<casual_task_worker::dispatcher::CancelOnDrop> {
    use casual_task_worker::consumers::{NotificationFanout, SseFanout};
    use casual_task_worker::dispatcher::{self, CancelOnDrop};

    if !config.worker_embedded {
        tracing::info!(
            "TF_WORKER_EMBEDDED=false: this process serves requests only; \
             a separate worker must run the dispatch loop"
        );
        return None;
    }
    let Some(dsn) = config.dispatcher_database_url.as_deref() else {
        // Not fatal. An API that serves requests is more useful than one that
        // refuses to start, and the operator is told precisely what is off and
        // what it costs them.
        tracing::warn!(
            "DISPATCHER_DATABASE_URL is not set: the outbox dispatcher is NOT \
             running, so notifications and live updates will not be delivered. \
             It needs the taskforge_dispatcher role (migration 0014), not the \
             application role."
        );
        return None;
    };

    // A small pool of its own. The loop is a handful of connections and must
    // not compete with request serving for the API's.
    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(config.pool.acquire_timeout)
        .connect(dsn)
        .await
    {
        Ok(pool) => pool,
        Err(error) => {
            tracing::error!(%error, "the dispatcher cannot connect; the loop is not running");
            return None;
        }
    };

    // Verified once, here, so a misconfigured DSN is a startup message rather
    // than a loop that claims nothing forever: `claim` polls across tenants and
    // needs a role that bypasses row-level security.
    if let Err(error) = casual_task_persistence::dispatch::DispatcherRole::verify(&mut *match pool
        .acquire()
        .await
    {
        Ok(conn) => conn,
        Err(error) => {
            tracing::error!(%error, "the dispatcher cannot acquire a connection");
            return None;
        }
    })
    .await
    {
        tracing::error!(
            %error,
            "DISPATCHER_DATABASE_URL does not connect as a role that bypasses \
             row-level security; the loop is not running"
        );
        return None;
    }

    let (handle, cancel) = CancelOnDrop::new();
    let worker_id = format!("api-{}", std::process::id());

    // One loop per consumer: each claims its own delivery rows, and a consumer
    // that is slow or failing must not hold up the others (`docs/25`).
    let notification = std::sync::Arc::new(NotificationFanout::new(
        state.pool.clone(),
        std::sync::Arc::clone(&state.mailer),
        config.public_url.clone(),
    ));
    let sse = std::sync::Arc::new(SseFanout::new(std::sync::Arc::clone(&state.broadcast)));
    // Its own pool, as `taskforge_app` and NOT the dispatcher's: the projection
    // writes tenant rows, and the dispatcher role bypasses row-level security
    // and is granted on the two outbox tables and nothing else (migration 0014).
    let search = std::sync::Arc::new(casual_task_worker::projection::SearchProjection::new(
        state.pool.clone(),
    ));
    let intervals = std::sync::Arc::new(
        casual_task_worker::state_interval::StateIntervalProjection::new(state.pool.clone()),
    );
    // `docs/28` step 4. Without this loop an upload is stored and stays
    // invisible forever, because `committed_at` is set by the scan alone —
    // which is exactly what the product did until this consumer existed.
    //
    // No `TF_CLAMD_ADDR` means no scanner, and no scanner means the attachment
    // stays `PENDING`: D-062, countersigned, and not something this code may
    // reverse by treating an absent scanner as a pass.
    let scanner: Option<std::sync::Arc<dyn casual_task_infra::Scanner>> =
        match std::env::var("TF_CLAMD_ADDR") {
            Ok(address) if !address.trim().is_empty() => {
                tracing::info!(%address, "scanning attachments with clamd");
                Some(std::sync::Arc::new(casual_task_infra::Clamd::new(address)))
            }
            _ => {
                tracing::warn!(
                    "TF_CLAMD_ADDR is unset: uploaded attachments will never become visible, \
                     because nothing can mark them clean (docs/28 step 4, D-062)"
                );
                None
            }
        };
    let scan = std::sync::Arc::new(casual_task_worker::attachment_scan::AttachmentScan::new(
        state.pool.clone(),
        std::sync::Arc::clone(&state.storage),
        scanner,
    ));

    // Export jobs are not outbox deliveries: one runner claims a queued job,
    // then re-resolves the requester's authority before every page. It still
    // shares this cancellation handle so embedded shutdown drains both kinds
    // of background work.
    {
        let dispatch_pool = pool.clone();
        let app_pool = state.pool.clone();
        let storage = std::sync::Arc::clone(&state.storage);
        let worker_id = worker_id.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if let Err(error) = casual_task_worker::export::runner::run(
                &dispatch_pool,
                &app_pool,
                storage,
                &worker_id,
                cancel,
            )
            .await
            {
                tracing::error!(%error, %worker_id, "export loop stopped unexpectedly");
            }
        });
    }

    for (name, spawn) in [
        (
            casual_task_worker::consumers::notification::NAME,
            Loop::Notification(notification),
        ),
        ("sse_fanout", Loop::Sse(sse)),
        // Without this loop the index is never written and search returns
        // nothing — while every task write succeeds and every gate passes,
        // because the projection's own tests drive the consumer directly.
        (casual_task_worker::projection::NAME, Loop::Search(search)),
        // Without this loop every duration measure reads an empty table and
        // reports zero — which looks like a quiet team rather than a missing
        // consumer.
        (
            casual_task_worker::state_interval::NAME,
            Loop::Intervals(intervals),
        ),
        (casual_task_worker::attachment_scan::NAME, Loop::Scan(scan)),
    ] {
        let pool = pool.clone();
        let cancel = cancel.clone();
        let metrics = std::sync::Arc::clone(&state.metrics);
        let worker_id = worker_id.clone();
        tokio::spawn(async move {
            let outcome = match spawn {
                Loop::Notification(consumer) => {
                    dispatcher::run(
                        &pool,
                        consumer,
                        &worker_id,
                        dispatcher::Config::default(),
                        cancel,
                        metrics,
                    )
                    .await
                }
                Loop::Sse(consumer) => {
                    dispatcher::run(
                        &pool,
                        consumer,
                        &worker_id,
                        dispatcher::Config::default(),
                        cancel,
                        metrics,
                    )
                    .await
                }
                Loop::Search(consumer) => {
                    dispatcher::run(
                        &pool,
                        consumer,
                        &worker_id,
                        dispatcher::Config::default(),
                        cancel,
                        metrics,
                    )
                    .await
                }
                Loop::Intervals(consumer) => {
                    dispatcher::run(
                        &pool,
                        consumer,
                        &worker_id,
                        dispatcher::Config::default(),
                        cancel,
                        metrics,
                    )
                    .await
                }
                Loop::Scan(consumer) => {
                    dispatcher::run(
                        &pool,
                        consumer,
                        &worker_id,
                        dispatcher::Config::default(),
                        cancel,
                        metrics,
                    )
                    .await
                }
            };
            match outcome {
                Ok(stopped) => tracing::info!(consumer = name, ?stopped, "dispatch loop stopped"),
                Err(error) => tracing::error!(%error, consumer = name, "dispatch loop failed"),
            }
        });
    }

    tracing::info!(worker_id, "the embedded outbox dispatcher is running");
    Some(handle)
}

/// The consumers the embedded worker runs. An enum rather than a boxed trait
/// object because `Consumer` takes `self` by reference in an `async fn` and is
/// therefore not dyn-compatible on the pinned toolchain.
enum Loop {
    Notification(std::sync::Arc<casual_task_worker::consumers::NotificationFanout>),
    Sse(std::sync::Arc<casual_task_worker::consumers::SseFanout>),
    Search(std::sync::Arc<casual_task_worker::projection::SearchProjection>),
    Intervals(std::sync::Arc<casual_task_worker::state_interval::StateIntervalProjection>),
    Scan(std::sync::Arc<casual_task_worker::attachment_scan::AttachmentScan>),
}

/// Refuse to serve as a superuser (`docs/48`, migration 0012).
///
/// A superuser bypasses **every** row-level security policy unconditionally and
/// is unaffected by the `REVOKE`s that make audit history append-only. Connected
/// as one, the application still works — every request succeeds, every test
/// passes, and tenant isolation and audit immutability are both silently inert.
/// There is no symptom until a customer sees another customer's tasks.
///
/// That is precisely the failure that has to be impossible rather than
/// documented, so it is checked here and the process exits.
async fn refuse_superuser(pool: &sqlx::PgPool) -> Result<(), String> {
    let is_superuser = casual_task_persistence::health::is_superuser(pool)
        .await
        .map_err(|error| format!("cannot determine the connected role: {error}"))?;

    if is_superuser {
        return Err(
            "refusing to start: DATABASE_URL connects as a SUPERUSER. A superuser bypasses every \
             row-level security policy and is unaffected by the REVOKEs that make audit history \
             append-only, so tenant isolation and audit immutability would both be silently \
             inert. Connect as taskforge_app (migration 0012, docs/52)."
                .to_owned(),
        );
    }
    Ok(())
}
