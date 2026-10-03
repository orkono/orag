//! Question answering: JSON or Server-Sent Events.

use std::convert::Infallible;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::retrieval::answer::{
    AnswerEvent, AnswerSummary, QueryTrace, SourceRef, validate_question,
};
use crate::server::errors::{ApiError, ApiPath, ApiResult};
use crate::server::{AppState, blocking};

const SSE_BUFFER: usize = 64;
/// A client that reads nothing for this long loses its stream (and the permit).
const SLOW_CLIENT_TIMEOUT: Duration = Duration::from_secs(30);
/// After shutdown starts, a full SSE buffer is retried only this long.
const SHUTDOWN_SEND_GRACE: Duration = Duration::from_secs(2);
/// How long a query waits for the single generation slot before `429 busy` (D-019).
/// Generous for one answer on the reference machines; Task 24 measures generation
/// speed and v0.2 revisits this value with data (D-018).
pub const GENERATION_WAIT: Duration = Duration::from_secs(120);

/// Flags cancellation when the request future is dropped (client disconnected).
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryBody {
    pub query: String,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub answer: String,
    pub sources: Vec<SourceRef>,
    pub citations: Vec<usize>,
    pub invalid_citations: Vec<usize>,
    pub abstained: bool,
    pub trace: QueryTrace,
}

pub async fn query(
    State(state): State<AppState>,
    ApiPath(collection_id): ApiPath<i64>,
    body: Result<Json<QueryBody>, JsonRejection>,
) -> ApiResult<Response> {
    let Json(body) = body.map_err(|e| ApiError::json_rejection(e.status(), e.body_text()))?;
    let question = validate_question(&body.query)?.to_string();
    check_queryable(&state, collection_id).await?;
    // Bounded wait for the single generation slot. `until_shutdown` also covers
    // a shutdown that started earlier, and wakes queued queries at once.
    let permit = state
        .until_shutdown(async {
            tokio::time::timeout(GENERATION_WAIT, state.generation.clone().acquire_owned())
                .await
                .map_err(|_| ApiError::busy("another answer is being generated; retry shortly"))?
                .map_err(|_| ApiError::internal())
        })
        .await?;
    if body.stream {
        // The wait can be long: check again, so a collection deleted or a model
        // changed meanwhile is still a status code, not an error event in a 200.
        check_queryable(&state, collection_id).await?;
        Ok(stream_answer(state, collection_id, question, permit).into_response())
    } else {
        Ok(Json(collect_answer(state, collection_id, question, permit).await?).into_response())
    }
}

/// Fails before any answer starts: unknown collection or embedding-space mismatch.
async fn check_queryable(state: &AppState, collection_id: i64) -> ApiResult<()> {
    let (store, engine) = (state.store.clone(), state.engine.clone());
    blocking(move || {
        store.get_collection(collection_id)?;
        store
            .query_space(collection_id, engine.retriever.embedder.descriptor())
            .map(|_| ())
    })
    .await
}

async fn collect_answer(
    state: AppState,
    collection_id: i64,
    question: String,
    permit: OwnedSemaphorePermit,
) -> ApiResult<QueryResponse> {
    let (engine, stopping) = (state.engine.clone(), state.subscribe_shutdown());
    let disconnected = Arc::new(AtomicBool::new(false));
    // The guard lives in this async frame, NOT in the blocking closure: when the
    // client disconnects, hyper drops this future, the guard sets the flag, and
    // the token callback below breaks at the next token. Only the flag moves in.
    let _guard = CancelOnDrop(disconnected.clone());
    blocking(move || {
        let _permit = permit;
        let mut sources = Vec::new();
        let mut summary: Option<AnswerSummary> = None;
        engine.answer(collection_id, &question, &mut |event| {
            match event {
                AnswerEvent::Sources(found) => sources = found,
                AnswerEvent::Token(_) => {}
                AnswerEvent::Done(done) => summary = Some(*done),
            }
            if disconnected.load(Ordering::SeqCst) || *stopping.borrow() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })?;
        // A Break is final (no Done follows), so a cut answer has no summary.
        // Stopped by shutdown: 503, never a truncated 200.
        let Some(summary) = summary else {
            if *stopping.borrow() {
                return Ok(None);
            }
            return Err(crate::error::OragError::Internal(
                "answer finished without summary".into(),
            ));
        };
        Ok(Some(QueryResponse {
            answer: summary.answer,
            sources,
            citations: summary.citations,
            invalid_citations: summary.invalid_citations,
            abstained: summary.abstained,
            trace: summary.trace,
        }))
    })
    .await?
    .ok_or_else(ApiError::shutting_down)
}

fn stream_answer(
    state: AppState,
    collection_id: i64,
    question: String,
    permit: OwnedSemaphorePermit,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(SSE_BUFFER);
    let (engine, stopping) = (state.engine.clone(), state.subscribe_shutdown());
    tokio::task::spawn_blocking(move || {
        let _permit = permit; // released when generation ends or the client disconnects
        let send = |event| send_or_give_up(&tx, event, &stopping);
        let result = engine.answer(collection_id, &question, &mut |event| {
            // A finished answer is delivered even if shutdown just started.
            if *stopping.borrow() && !matches!(event, AnswerEvent::Done(_)) {
                // Stream ends with one `error` event (a Break is final: no Done follows).
                let _ = send(error_event(&ApiError::shutting_down()));
                return ControlFlow::Break(());
            }
            send(to_event(event))
        });
        if let Err(err) = result {
            let _ = send(error_event(&ApiError::from(err)));
        }
    });
    Sse::new(ReceiverStream::new(rx).map(Ok)).keep_alive(KeepAlive::default())
}

/// The `event: error` frame for an API error; every SSE error goes through here.
fn error_event(error: &ApiError) -> Event {
    Event::default()
        .event("error")
        .json_data(error.body())
        .unwrap_or_else(|_| {
            Event::default()
                .event("error")
                .data("{\"error\":{\"code\":\"internal\",\"message\":\"serialization failed\"}}")
        })
}

/// Delivers an event, giving up if the client disconnected or stopped reading.
/// A full buffer is retried for `SLOW_CLIENT_TIMEOUT`, or only for
/// `SHUTDOWN_SEND_GRACE` once shutdown has started: a slow but reading client
/// still gets its final `done`/`error`, a client that stopped reading cannot
/// hold the generation task. (`serve` bounds the socket drain separately.)
/// Polls with a std sleep on purpose: a tokio timer driven from this blocking
/// thread panics if the runtime shuts down underneath it.
fn send_or_give_up(
    tx: &mpsc::Sender<Event>,
    event: Event,
    stopping: &watch::Receiver<bool>,
) -> ControlFlow<()> {
    let deadline = Instant::now() + SLOW_CLIENT_TIMEOUT;
    let mut shutdown_deadline: Option<Instant> = None;
    let mut pending = event;
    loop {
        match tx.try_send(pending) {
            Ok(()) => return ControlFlow::Continue(()),
            Err(TrySendError::Closed(_)) => return ControlFlow::Break(()),
            Err(TrySendError::Full(back)) => {
                let limit = if *stopping.borrow() {
                    deadline.min(
                        *shutdown_deadline
                            .get_or_insert_with(|| Instant::now() + SHUTDOWN_SEND_GRACE),
                    )
                } else {
                    deadline
                };
                if Instant::now() >= limit {
                    return ControlFlow::Break(());
                }
                pending = back;
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn to_event(event: AnswerEvent) -> Event {
    let built = match event {
        AnswerEvent::Sources(sources) => Event::default()
            .event("sources")
            .json_data(serde_json::json!({ "sources": sources })),
        AnswerEvent::Token(text) => Event::default()
            .event("token")
            .json_data(serde_json::json!({ "text": text })),
        AnswerEvent::Done(summary) => Event::default().event("done").json_data(&*summary),
    };
    built.unwrap_or_else(|_| error_event(&ApiError::internal()))
}
