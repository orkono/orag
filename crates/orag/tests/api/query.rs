//! Query endpoint: JSON and SSE answers, cancellation, the generation slot and shutdown.

use std::time::Duration;

use axum::http::{StatusCode, header};
use http_body_util::BodyExt;
use orag::infer::fake::FakeGenerator;
use orag::server::router;
use serde_json::json;
use tower::ServiceExt;

use crate::support::{
    TestApp, app, app_with, drain_jobs, index_with_previous_model, json_request, send, send_full,
};

async fn seed(app: &TestApp) {
    let doc = json!({"filename": "iade.md", "content": "# Kargo\n\n## İade\n\nÜrünler 14 gün içinde iade edilebilir."});
    let (status, body) = send(
        app,
        json_request("POST", "/v1/collections/1/documents", doc),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    drain_jobs(app);
}

/// Words in the slow fake reply; the generator is allowed to emit all of them,
/// so only cancellation can keep the count below it.
const LONG_REPLY_WORDS: usize = 400;

/// At least 0.8 s for a full answer (several seconds on a loaded CI runner),
/// so an uncancelled run is easy to observe.
fn slow_long_generator() -> FakeGenerator {
    FakeGenerator::new(&"kelime ".repeat(LONG_REPLY_WORDS))
        .with_context(4096, LONG_REPLY_WORDS)
        .with_token_delay(Duration::from_millis(2))
}

/// Generation has stopped: the single generation permit comes back and the
/// token count is below a full answer. Waits for the event, not a fixed time:
/// the permit is dropped when the generation task ends, after its last token
/// was counted, so once it is free the count is final.
async fn assert_generation_stopped(app: &TestApp, why: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let permit = loop {
        if let Ok(permit) = app.state.generation.clone().try_acquire_owned() {
            break permit;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the generation permit was not released: {why}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    drop(permit);
    let settled = app.generator.emitted_tokens();
    assert!(
        settled < LONG_REPLY_WORDS,
        "{why}: all {settled} tokens were generated"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(app.generator.emitted_tokens(), settled, "{why}");
}

#[tokio::test]
async fn query_returns_grounded_answer_json() {
    let app = app_with(FakeGenerator::new("14 gün [1]."), 1024 * 1024);
    seed(&app).await;
    let (status, body) = send(
        &app,
        json_request(
            "POST",
            "/v1/collections/1/query",
            json!({"query": "İade süresi kaç gün?"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["answer"], "14 gün [1].");
    assert_eq!(body["citations"], json!([1]));
    assert_eq!(body["sources"][0]["number"], 1);
    assert_eq!(body["trace"]["retrieval"]["strategy"], "hybrid");
    assert_eq!(body["abstained"], false);
}

#[tokio::test]
async fn query_streams_sources_tokens_done_in_order() {
    let app = app_with(FakeGenerator::new("14 gün [1]."), 1024 * 1024);
    seed(&app).await;
    let request = json_request(
        "POST",
        "/v1/collections/1/query",
        json!({"query": "iade", "stream": true}),
    );
    let response = router(app.state.clone()).oneshot(request).await.unwrap();
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let text = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    let sources = text.find("event: sources").expect("sources event");
    let token = text.find("event: token").expect("token event");
    let done = text.find("event: done").expect("done event");
    assert!(sources < token && token < done, "{text}");
}

#[tokio::test]
async fn query_validation_and_missing_collection() {
    let app = app();
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/collections/1/query", json!({"query": "  "})),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::BAD_REQUEST, json!("invalid_input"))
    );
    let (status, _) = send(
        &app,
        json_request("POST", "/v1/collections/99/query", json!({"query": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn embedding_model_change_requires_reindex() {
    let app = app();
    // A collection indexed by a previous embedding model.
    let new_collection = app.state.store.create_collection("x").unwrap();
    index_with_previous_model(&app, new_collection.id);
    let uri = format!("/v1/collections/{}/query", new_collection.id);
    let (status, body) = send(&app, json_request("POST", &uri, json!({"query": "iade"}))).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::CONFLICT, json!("reindex_required"))
    );
}

#[tokio::test]
async fn dropping_sse_stream_stops_generation() {
    let app = app_with(slow_long_generator(), 1024 * 1024);
    seed(&app).await;
    let request = json_request(
        "POST",
        "/v1/collections/1/query",
        json!({"query": "iade", "stream": true}),
    );
    let response = router(app.state.clone()).oneshot(request).await.unwrap();
    let mut body = response.into_body();
    let _first = body.frame().await.expect("first frame");
    drop(body);
    assert_generation_stopped(&app, "generation kept running after disconnect").await;
    // The server still answers: a new stream starts with its sources. (Waiting
    // for a whole slow answer would only add a timing-dependent wait.)
    let request = json_request(
        "POST",
        "/v1/collections/1/query",
        json!({"query": "iade", "stream": true}),
    );
    let response = router(app.state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first = response
        .into_body()
        .frame()
        .await
        .expect("first frame")
        .unwrap();
    let text = String::from_utf8_lossy(first.data_ref().expect("data frame")).to_string();
    assert!(text.starts_with("event: sources"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn query_wait_for_the_generation_slot_is_bounded() {
    let app = app();
    let _held = app.state.generation.clone().try_acquire_owned().unwrap();
    // Paused time jumps straight to GENERATION_WAIT; the collection is empty, so no seeding.
    let started = tokio::time::Instant::now();
    let (status, headers, body) = send_full(
        &app,
        json_request("POST", "/v1/collections/1/query", json!({"query": "iade"})),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::TOO_MANY_REQUESTS, json!("busy"))
    );
    assert!(
        started.elapsed() >= orag::server::query::GENERATION_WAIT,
        "the query must wait before giving up"
    );
    assert_eq!(headers[header::RETRY_AFTER], "5");
}

#[tokio::test]
async fn query_queued_through_shutdown_does_not_start() {
    let long_reply = "kelime ".repeat(400);
    let app = app_with(FakeGenerator::new(&long_reply), 1024 * 1024);
    seed(&app).await; // retrieval finds chunks, so a query that ran would generate tokens
    let held = app.state.generation.clone().try_acquire_owned().unwrap();
    let request = json_request("POST", "/v1/collections/1/query", json!({"query": "iade"}));
    let pending = tokio::spawn(router(app.state.clone()).oneshot(request));
    // Queued on the slot or still in its pre-checks: both observe the same
    // channel (`wait_for` sees an earlier shutdown too), so the result is 503.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !pending.is_finished(),
        "the query should be waiting for the generation slot"
    );
    app.state.begin_shutdown();
    // Woken by shutdown, not by the slot: `held` is still taken here.
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("shutdown wakes the waiter")
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    drop(held);
    // With the slot free, a query arriving during shutdown still starts no generation.
    let (status, _) = send(
        &app,
        json_request("POST", "/v1/collections/1/query", json!({"query": "iade"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        app.generator.emitted_tokens(),
        0,
        "no generation may start during shutdown"
    );
}

/// Waits (bounded) until the fake generator has emitted a token, so a test can
/// start shutdown mid-generation without depending on machine speed.
async fn wait_until_generating(app: &TestApp) {
    for _ in 0..1000 {
        if app.generator.emitted_tokens() > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("generation never started");
}

#[tokio::test]
async fn shutdown_during_generation_never_returns_a_partial_json_answer() {
    let app = app_with(slow_long_generator(), 1024 * 1024);
    seed(&app).await;
    let request = json_request("POST", "/v1/collections/1/query", json!({"query": "iade"}));
    let pending = tokio::spawn(router(app.state.clone()).oneshot(request));
    wait_until_generating(&app).await;
    app.state.begin_shutdown();
    let response = pending.await.unwrap().unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "a truncated answer must not be a 200"
    );
    assert!(app.generator.emitted_tokens() > 0);
    assert_generation_stopped(&app, "generation kept running after shutdown").await;
}

#[tokio::test]
async fn shutdown_during_sse_ends_with_an_error_event() {
    let app = app_with(slow_long_generator(), 1024 * 1024);
    seed(&app).await;
    let (state, generator) = (app.state.clone(), app.generator.clone());
    tokio::spawn(async move {
        // >= 2: the counter rises just before a token reaches the stream, so wait
        // for a second one to be sure the first was delivered.
        while generator.emitted_tokens() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        state.begin_shutdown();
    });
    let request = json_request(
        "POST",
        "/v1/collections/1/query",
        json!({"query": "iade", "stream": true}),
    );
    let response = router(app.state.clone()).oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("event: token"), "{text}");
    assert!(
        text.trim_end().ends_with("}")
            && text.contains("event: error")
            && text.contains("shutting_down"),
        "{text}"
    );
    assert!(!text.contains("event: done"), "{text}");
    assert_eq!(text.matches("event: error").count(), 1, "{text}");
}

#[tokio::test]
async fn abandoned_json_query_stops_generation() {
    let app = app_with(slow_long_generator(), 1024 * 1024);
    seed(&app).await;
    let request = json_request("POST", "/v1/collections/1/query", json!({"query": "iade"}));
    let pending = tokio::spawn(router(app.state.clone()).oneshot(request));
    // Abandoned mid-generation, however slow the runner: dropping the request
    // future is what a client disconnect does.
    wait_until_generating(&app).await;
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    assert_generation_stopped(&app, "generation kept running for a disconnected client").await;
}

#[tokio::test]
async fn a_collection_deleted_while_queued_fails_the_stream_with_a_status() {
    let app = app();
    let collection = app.state.store.create_collection("gecici").unwrap();
    let held = app.state.generation.clone().try_acquire_owned().unwrap();
    let request = json_request(
        "POST",
        &format!("/v1/collections/{}/query", collection.id),
        json!({"query": "iade", "stream": true}),
    );
    let pending = tokio::spawn(router(app.state.clone()).oneshot(request));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!pending.is_finished(), "the query should be queued");
    app.state.store.delete_collection(collection.id).unwrap();
    drop(held);
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("the queued query runs")
        .unwrap()
        .unwrap();
    // Checked again after the wait: a 404, not a 200 stream with an error event.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
