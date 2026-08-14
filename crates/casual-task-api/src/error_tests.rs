use super::*;
use axum::body::to_bytes;

async fn body_of(error: ApiError) -> serde_json::Value {
    let response = error.into_response();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test]
async fn the_envelope_matches_the_documented_shape() {
    let error = ApiError::not_found("018f2c").with_details(serde_json::json!({"a": 1}));
    let json = body_of(error).await;

    assert_eq!(json["error"]["code"], "TF-AZN-0008");
    assert_eq!(json["error"]["request_id"], "018f2c");
    assert_eq!(json["error"]["details"]["a"], 1);
    assert_eq!(
        json["error"]["docs"],
        "https://docs.taskforge.dev/errors/TF-AZN-0008"
    );
}

#[tokio::test]
async fn details_are_absent_rather_than_null_when_there_are_none() {
    // `null` and "not applicable" are different things to a client that
    // switches on presence.
    let json = body_of(ApiError::unauthenticated("r")).await;
    assert!(
        json["error"].get("details").is_none(),
        "details rendered as null: {json}"
    );
}

#[tokio::test]
async fn an_internal_error_reveals_nothing() {
    // The detail belongs in the log, correlated by request_id. In the
    // response it is reconnaissance.
    let json = body_of(ApiError::internal("r")).await;
    assert_eq!(json["error"]["message"], "Something went wrong");
    assert!(json["error"].get("details").is_none());
}

#[test]
fn a_rate_limit_refusal_always_carries_retry_after() {
    // docs/05: "429 | rate limited (`Retry-After` always present)". The
    // constructor takes the value, so there is no path to a 429 without it —
    // a client told to back off with no idea for how long retries
    // immediately, which is the flood the limiter was added to stop.
    let response = ApiError::too_many_requests("r", 6).into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("6")
    );
}

#[tokio::test]
async fn a_rate_limit_refusal_names_no_limit_and_no_address() {
    // The body is reconnaissance if it says what was exceeded. The numbers a
    // legitimate client needs are the RateLimit-* headers.
    let json = body_of(ApiError::too_many_requests("r", 6)).await;
    assert_eq!(json["error"]["code"], "TF-LIM-0001");
    assert_eq!(json["error"]["message"], "Too many requests");
    assert!(json["error"].get("details").is_none());
}

#[test]
fn service_unavailable_always_carries_retry_after() {
    // docs/05 says "always present" for 429 and 503. The constructor sets
    // it, so a call site cannot omit it.
    let response = ApiError::unavailable("r", 5).into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("5")
    );
}

#[test]
fn the_documented_status_codes_are_used() {
    assert_eq!(
        ApiError::unauthenticated("r").status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ApiError::forbidden(codes::CSRF, "r").status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(ApiError::not_found("r").status(), StatusCode::NOT_FOUND);
}

#[test]
fn every_code_this_binary_emits_is_in_the_registry() {
    // docs/20 is what the `docs` URL in every error body points at. A code
    // that is not there is a link to a 404 in the exact moment a user is
    // trying to understand a failure.
    //
    // There is deliberately NO exception list. C-002 shipped this gate with
    // four — TF-REQ-0001, TF-REQ-0004, TF-SRV-0001, TF-SRV-0003, in two
    // areas the registry does not define — and opened D-055 rather than
    // resolving it. D-055 is now resolved: the four were retired in favour
    // of registry codes, which was safe for a reason that will not be true
    // again — none of them had ever been released. An exception list is how
    // a gate stops holding, one entry at a time, so this one has nowhere to
    // put the next one.
    //
    // (This test also went missing: it exists on `feat/c002-workspaces` and
    // was dropped by the merge into `feat/phase-1`. Restored here.)
    let registry = include_str!("../../../docs/20-ERROR-CODE-REGISTRY.md");
    for code in codes::ALL {
        assert!(
            registry.contains(code.as_str()),
            "{code:?} is emitted by this binary and absent from docs/20"
        );
    }
}

#[test]
fn the_registry_gate_can_fail() {
    // A gate nobody has watched fail is a gate nobody knows works. The
    // retired codes are the values the check above would have to reject.
    let registry = include_str!("../../../docs/20-ERROR-CODE-REGISTRY.md");
    for retired in ["TF-REQ-0001", "TF-REQ-0004", "TF-SRV-0001", "TF-SRV-0003"] {
        assert!(
            !registry.contains(retired),
            "{retired} is back in the registry, so the gate above would \
                 pass for a code that should not exist"
        );
        assert!(
            !codes::ALL.iter().any(|c| c.as_str() == retired),
            "{retired} is emitted again"
        );
    }
}

#[test]
fn the_area_of_every_code_is_one_the_registry_declares() {
    // Stronger than containment: `TF-XYZ-0001` would pass the test above if
    // the string happened to appear anywhere in the prose. The registry
    // declares its areas in one table, and a code outside them is a code in
    // an area nobody defined — which is exactly what TF-REQ-* and TF-SRV-*
    // were.
    let areas = [
        "AUT", "AZN", "VAL", "QRY", "WFL", "TSK", "PRJ", "CNC", "IDM", "ATT", "PLG", "AUM", "LIM",
        "SYS",
    ];
    for code in codes::ALL {
        let area = code.as_str().split('-').nth(1).unwrap_or_default();
        assert!(
            areas.contains(&area),
            "{code:?} is in area {area}, which docs/20 does not declare"
        );
    }
}

#[test]
fn every_code_this_crate_emits_is_in_the_registry() {
    // docs/20 is what the `docs` URL in every error body points at. A code
    // that is not there is a link to a 404 in the exact moment a user is
    // trying to understand a failure.
    //
    // There is deliberately no exception list. One existed while four codes
    // were drifting from the registry; D-055 corrected the codes instead,
    // and an exception list that outlives its exceptions is a gate with a
    // hole in it.
    //
    // This test was dropped once already, by a merge that resolved two
    // versions of the enclosing module by keeping matching lines. Losing it
    // is silent — the codes keep working, and only their documentation
    // links rot.
    let registry = include_str!("../../../docs/20-ERROR-CODE-REGISTRY.md");
    for code in codes::ALL {
        assert!(
            registry.contains(code.as_str()),
            "{code:?} is emitted by this crate and absent from docs/20"
        );
    }
}

#[test]
fn every_code_follows_the_registry_format() {
    // docs/20: TF-XXX-NNNN. A code that does not match is one no client can
    // look up, and the URL in the envelope would 404.
    for code in codes::ALL {
        let text = code.as_str();
        let parts: Vec<_> = text.split('-').collect();
        assert_eq!(parts.len(), 3, "{text}");
        assert_eq!(parts[0], "TF", "{text}");
        assert_eq!(parts[1].len(), 3, "{text}");
        assert!(parts[1].bytes().all(|b| b.is_ascii_uppercase()), "{text}");
        assert_eq!(parts[2].len(), 4, "{text}");
        assert!(parts[2].bytes().all(|b| b.is_ascii_digit()), "{text}");
    }
}
