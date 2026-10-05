//! Document upload (JSON text or multipart file), listing, retrieval, deletion.

use axum::Json;
use axum::extract::{FromRequest, Multipart, Request, State};
use axum::http::{StatusCode, header};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ingest::format::SourceFormat;
use crate::server::errors::{ApiError, ApiPath, ApiQuery, ApiResult};
use crate::server::{AppState, blocking};
use crate::store::documents::{DocumentRecord, NewDocument, Page};

const DEFAULT_PAGE_SIZE: u32 = 50;
/// How long an upload may take to send its body; the upload permit is held meanwhile.
const UPLOAD_BODY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextDocument {
    pub content: String,
    pub filename: Option<String>,
    /// `text` or `markdown`; detected from `filename` when absent, else `text`.
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub after_id: Option<i64>,
    pub limit: Option<u32>,
}

pub async fn upload(
    State(state): State<AppState>,
    ApiPath(collection_id): ApiPath<i64>,
    request: Request,
) -> ApiResult<(StatusCode, Json<Value>)> {
    // Cheap rejections first: a bad Content-Type gets 415 even when busy.
    // (Filename-based 415s need the body, so they come after the permit.)
    let kind = UploadKind::of(&request)?;
    // Unknown or incompatible collections are refused before a slot is taken (D-009).
    // Trade-off: one cheap SQLite read even for requests that then get 429.
    let (store, engine) = (state.store.clone(), state.engine.clone());
    blocking(move || store.query_space(collection_id, engine.retriever.embedder.descriptor()))
        .await?;
    // Admission control before the body is read (D-019).
    let _upload_permit = state
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::busy("too many uploads in progress; retry shortly"))?;
    // Total deadline for the body (not an inactivity timer): a stalled client
    // cannot hold a permit forever. 60 s is ample for ≤ 60 MB over loopback.
    // Shutdown (the same single signal as queries) wins over a body in transit.
    let document = state
        .until_shutdown(async {
            tokio::time::timeout(UPLOAD_BODY_TIMEOUT, read_document(&state, kind, request))
                .await
                .map_err(|_| {
                    ApiError::new(
                        StatusCode::REQUEST_TIMEOUT,
                        "upload_timeout",
                        "the upload body was not received in time",
                    )
                })?
        })
        .await?;
    if document.bytes.len() > state.max_upload_bytes {
        return Err(too_large());
    }
    let (store, engine) = (state.store.clone(), state.engine.clone());
    let enqueued = blocking(move || {
        // On a blocking thread: a DOCX signature reads the ZIP directory.
        document.format.check_signature(&document.bytes)?;
        // Re-checked after the body arrived (a separate read, not part of the
        // insert's transaction): the model or collection may have changed.
        store.query_space(collection_id, engine.retriever.embedder.descriptor())?;
        store.enqueue_document(collection_id, document)
    })
    .await?;
    state.ingest_wake.notify_one();
    let status = if enqueued.duplicate {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((status, Json(json!(enqueued))))
}

/// The two accepted upload encodings, decided from `Content-Type` once.
#[derive(Clone, Copy)]
enum UploadKind {
    Multipart,
    Json,
}

impl UploadKind {
    fn of(request: &Request) -> ApiResult<UploadKind> {
        let content_type = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if content_type.starts_with("multipart/form-data") {
            Ok(UploadKind::Multipart)
        } else if content_type.starts_with("application/json") {
            Ok(UploadKind::Json)
        } else {
            Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "send application/json or multipart/form-data",
            ))
        }
    }
}

/// Reads the request body as a multipart file or a JSON text document.
async fn read_document(
    state: &AppState,
    kind: UploadKind,
    request: Request,
) -> ApiResult<NewDocument> {
    match kind {
        UploadKind::Multipart => {
            let multipart = Multipart::from_request(request, state)
                .await
                .map_err(|e| body_error(e.status(), "invalid_multipart", e.body_text()))?;
            read_multipart(multipart, state.max_upload_bytes).await
        }
        UploadKind::Json => {
            let Json(body) = Json::<TextDocument>::from_request(request, state)
                .await
                .map_err(|e| match e.status() {
                    StatusCode::PAYLOAD_TOO_LARGE => too_large(),
                    status => ApiError::json_rejection(status, e.body_text()),
                })?;
            text_document(body)
        }
    }
}

/// Format of JSON text content (D-010): see `SourceFormat::detect_text`.
fn text_document(body: TextDocument) -> ApiResult<NewDocument> {
    let declared = match body
        .format
        .as_deref()
        .map(SourceFormat::from_name)
        .transpose()
    {
        Ok(declared) => declared,
        Err(err) => {
            // Filename rules come first (D-010): `a.pdf` is "use multipart" even with a bad `format`.
            SourceFormat::detect_text(body.filename.as_deref(), None)?;
            return Err(err.into());
        }
    };
    let format = SourceFormat::detect_text(body.filename.as_deref(), declared)?;
    Ok(NewDocument {
        filename: body.filename,
        format,
        bytes: body.content.into_bytes(),
    })
}

fn too_large() -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "too_large",
        "document exceeds max_document_mb in config.toml",
    )
}

/// Body-limit rejections from extractors become the same `too_large` error as the explicit check.
fn body_error(status: StatusCode, code: &'static str, message: String) -> ApiError {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        too_large()
    } else {
        ApiError::new(status, code, message)
    }
}

/// Streams the `file` field, stopping as soon as it passes `max_bytes`, so an
/// oversized file is never held in memory whole.
async fn read_multipart(mut multipart: Multipart, max_bytes: usize) -> ApiResult<NewDocument> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| body_error(e.status(), "invalid_multipart", e.body_text()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().map(str::to_string);
        let format = SourceFormat::detect(filename.as_deref(), field.content_type())?;
        let mut field = field;
        let mut bytes = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| body_error(e.status(), "invalid_multipart", e.body_text()))?
        {
            if bytes.len() + chunk.len() > max_bytes {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(NewDocument {
            filename,
            format,
            bytes,
        });
    }
    Err(ApiError::invalid(
        "multipart body must contain a `file` field",
    ))
}

pub async fn list(
    State(state): State<AppState>,
    ApiPath(collection_id): ApiPath<i64>,
    ApiQuery(params): ApiQuery<ListParams>,
) -> ApiResult<Json<Value>> {
    let limit = params.limit.unwrap_or(DEFAULT_PAGE_SIZE);
    let store = state.store.clone();
    let documents: Vec<DocumentRecord> = blocking(move || {
        // list_documents returns NotFound for an unknown collection.
        store.list_documents(
            collection_id,
            Page {
                after_id: params.after_id,
                limit,
            },
        )
    })
    .await?;
    // A limit outside 1..=MAX_PAGE_SIZE was already rejected by the store (400).
    let next_after_id = (documents.len() as u32 == limit)
        .then(|| documents.last().map(|d| d.id))
        .flatten();
    Ok(Json(
        json!({ "documents": documents, "next_after_id": next_after_id }),
    ))
}

pub async fn get_one(
    State(state): State<AppState>,
    ApiPath((collection_id, document_id)): ApiPath<(i64, i64)>,
) -> ApiResult<Json<DocumentRecord>> {
    let store = state.store.clone();
    Ok(Json(
        blocking(move || store.get_document(collection_id, document_id)).await?,
    ))
}

pub async fn remove(
    State(state): State<AppState>,
    ApiPath((collection_id, document_id)): ApiPath<(i64, i64)>,
) -> ApiResult<StatusCode> {
    let store = state.store.clone();
    blocking(move || store.delete_document(collection_id, document_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
