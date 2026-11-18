//! Assistant mode over HTTP (spec-assistant-mode): its status and spending, the saved 3D models,
//! and "remember that ..." as a route. The chat itself is `POST /api/chat` with
//! `mode: "assistant"` (see `routes.rs`).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::models3d;
use crate::routes::ApiError;
use crate::state::AppState;

fn assistant(state: &AppState) -> Result<&std::sync::Arc<crate::assistant::Assistant>, ApiError> {
    state.assistant().ok_or_else(|| {
        ApiError::NotFound("Assistant mode is turned off ([assistant] enabled = false)".into())
    })
}

/// `GET /api/assistant/status`: whether Assistant mode can answer, with which model, and this
/// month's spending.
pub async fn status(State(state): State<AppState>) -> Json<Value> {
    let config = &state.config().assistant;
    let Some(a) = state.assistant() else {
        return Json(json!({
            "enabled": false, "available": false,
            "reason": "Assistant mode is turned off ([assistant] enabled = false).",
            "brain": null, "spend": null,
        }));
    };
    let model = state
        .provider(&config.provider)
        .map(|e| e.config.model.clone());
    let (available, reason) = match state.assistant_provider() {
        Ok(_) => (true, None),
        Err(reason) => (false, Some(reason)),
    };
    Json(json!({
        "enabled": true,
        "available": available,
        "reason": reason,
        "brain": { "provider": config.provider, "model": model },
        "spend": a.spend().status(chrono::Utc::now()),
        "notes": state.notes().is_some(),
    }))
}

/// `GET /api/models`: saved 3D models, newest first.
pub async fn models_list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let store = assistant(&state)?.models().clone();
    let list = tokio::task::spawn_blocking(move || store.list())
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(Json(json!({ "models": list })))
}

/// `GET /api/models/{id}`: one model with its code (run only in the browser's sandbox).
pub async fn model_get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<models3d::Model3d>, ApiError> {
    if !models3d::valid_id(&id) {
        return Err(ApiError::NotFound(format!("no model with id {id}")));
    }
    let store = assistant(&state)?.models().clone();
    let found = {
        let id = id.clone();
        tokio::task::spawn_blocking(move || store.get(&id))
            .await
            .map_err(|_| ApiError::Internal)?
    };
    found
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("no model with id {id}")))
}

#[derive(Deserialize)]
struct CaptureBody {
    text: String,
}

/// `POST /api/notes/capture` `{text}`: write a new note to the captures folder, the same door
/// the assistant's `remember_note` uses. Never edits or deletes a note.
pub async fn capture(State(state): State<AppState>, body: Bytes) -> Result<Json<Value>, ApiError> {
    let req: CaptureBody = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))?;
    let notes = state
        .notes()
        .ok_or_else(|| ApiError::NotFound("notes are turned off ([notes] enabled = false)".into()))?
        .clone();
    let sub = state.config().assistant.captures_dir.clone();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let result = tokio::task::spawn_blocking(move || notes.capture(&sub, &req.text, &today))
        .await
        .map_err(|_| ApiError::Internal)?;
    match result {
        Ok(c) => {
            state
                .audit()
                .record(
                    None,
                    crate::audit::AuditKind::NoteCaptured,
                    json!({ "id": c.id, "title": c.title, "path": c.path, "via": "route" }),
                )
                .await;
            Ok(Json(json!(c)))
        }
        Err(
            e @ (jarvis_notes::CaptureError::Empty
            | jarvis_notes::CaptureError::TooLong
            | jarvis_notes::CaptureError::BadFolder
            | jarvis_notes::CaptureError::BadDate),
        ) => Err(ApiError::BadRequest(e.to_string())),
        Err(e) => {
            tracing::error!("capturing a note failed: {e}");
            Err(ApiError::BadRequest(format!("the note was not saved: {e}")))
        }
    }
}

// ---------------------------------------------------------------- uploads (spec-model-uploads)

#[derive(Deserialize)]
pub struct UploadQuery {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

fn upload_error(e: models3d::UploadError) -> ApiError {
    ApiError::BadRequest(e.to_string())
}

/// Store an uploaded file or picture as a model, with its bytes.
async fn store_upload(
    state: &AppState,
    model: models3d::Model3d,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let store = assistant(state)?.models().clone();
    let saved = {
        let model = model.clone();
        tokio::task::spawn_blocking(move || store.save_with_bytes(&model, &bytes))
            .await
            .map_err(|_| ApiError::Internal)?
    };
    saved.map_err(|e| {
        tracing::error!("saving an uploaded model failed: {e}");
        ApiError::Internal
    })?;
    state
        .audit()
        .record(
            None,
            crate::audit::AuditKind::ModelBuilt,
            json!({ "id": model.id, "title": model.title, "kind": model.kind,
                    "format": model.format, "source": "upload" }),
        )
        .await;
    Ok(Json(json!({
        "id": model.id, "title": model.title, "kind": model.kind, "format": model.format,
    })))
}

fn upload_model(
    kind: models3d::ModelKind,
    format: &str,
    name: &str,
    description: String,
) -> models3d::Model3d {
    models3d::Model3d {
        id: models3d::new_id(),
        title: models3d::title_from_name(name),
        description,
        code: String::new(),
        created_at: chrono::Utc::now(),
        previous_id: None,
        kind,
        format: Some(format.to_owned()),
        info: None,
        overlay: None,
        file_id: None,
    }
}

/// `POST /api/models/upload?name=car.glb`: a 3D file, raw body. Checked by its first bytes and
/// stored as it is; never parsed here.
pub async fn upload(
    State(state): State<AppState>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let name = q.name.unwrap_or_default();
    let format = models3d::sniff_model(&name, &body).map_err(upload_error)?;
    let model = upload_model(
        models3d::ModelKind::File,
        format,
        &name,
        format!("Uploaded file {}", models3d::title_from_name(&name)),
    );
    store_upload(&state, model, body).await
}

/// `POST /api/models/relief?name=photo.jpg`: a picture to show as a raised relief, raw body.
pub async fn relief(
    State(state): State<AppState>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let name = q.name.unwrap_or_default();
    let (format, _) = models3d::sniff_image(&body).map_err(upload_error)?;
    let model = upload_model(
        models3d::ModelKind::Relief,
        format,
        &name,
        format!("Relief of {}", models3d::title_from_name(&name)),
    );
    store_upload(&state, model, body).await
}

/// `POST /api/models/from-image?note=...`: Claude builds a 3D model of what the picture shows
/// (paid, through the monthly cap). Always `200`: `model` is null and `message` says why when
/// nothing was built. The picture is not kept.
pub async fn from_image(
    State(state): State<AppState>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let a = assistant(&state)?.clone();
    let (_, media_type) = models3d::sniff_image(&body).map_err(upload_error)?;
    if body.len() > models3d::MAX_CLAUDE_IMAGE_BYTES {
        return Err(ApiError::BadRequest(format!(
            "the picture is too large for Claude ({} MB; the limit is 3.75 MB). The app shrinks \
             pictures before sending them.",
            body.len().div_ceil(1024 * 1024)
        )));
    }
    let provider = match state.assistant_provider() {
        Ok(p) => p,
        Err(reason) => {
            return Ok(Json(json!({
                "model": null,
                "message": format!(
                    "Claude is not set up ({reason}). Set ANTHROPIC_API_KEY and restart Arc MT."
                ),
                "spend": a.spend().status(chrono::Utc::now()),
            })));
        }
    };
    let note = q.note.unwrap_or_default();
    // A task of its own, so the build and its spending record finish even if this request is cut.
    let task = {
        let a = a.clone();
        tokio::spawn(async move {
            a.build_from_image(provider.as_ref(), &body, media_type, &note)
                .await
        })
    };
    let result = task.await.map_err(|_| ApiError::Internal)?;
    let spend = a.spend().status(chrono::Utc::now());
    Ok(Json(match result {
        Ok(model) => json!({
            "model": model,
            "message": format!("Built \"{}\" from your image.", model.title),
            "spend": spend,
        }),
        Err(message) => json!({ "model": null, "message": message, "spend": spend }),
    }))
}

/// `GET /api/models/{id}/file`: the stored bytes of a file or relief model.
pub async fn model_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    if !models3d::valid_id(&id) {
        return Err(ApiError::NotFound(format!("no model with id {id}")));
    }
    let store = assistant(&state)?.models().clone();
    let bytes = {
        let id = id.clone();
        tokio::task::spawn_blocking(move || store.bytes(&id))
            .await
            .map_err(|_| ApiError::Internal)?
    };
    let bytes = bytes.ok_or_else(|| ApiError::NotFound(format!("model {id} has no file")))?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
            (axum::http::header::CONTENT_DISPOSITION, "attachment"),
            (
                axum::http::header::CACHE_CONTROL,
                "private, max-age=86400, immutable",
            ),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
struct InfoBody {
    parts: Vec<String>,
    size: [f64; 3],
}

/// `PUT /api/models/{id}/info` `{parts, size}`: what the sandbox found in an uploaded file, so a
/// later change by Claude knows the part names and size.
pub async fn model_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let info: InfoBody = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))?;
    if !models3d::valid_id(&id) {
        return Err(ApiError::NotFound(format!("no model with id {id}")));
    }
    if info.size.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(ApiError::BadRequest(
            "size must be three positive numbers".into(),
        ));
    }
    let parts: Vec<String> = info
        .parts
        .into_iter()
        .take(200)
        .map(|p| p.chars().filter(|c| !c.is_control()).take(80).collect())
        .collect();
    let value = json!({ "parts": parts, "size": info.size });
    let store = assistant(&state)?.models().clone();
    let found = {
        let id = id.clone();
        tokio::task::spawn_blocking(move || store.set_info(&id, value))
            .await
            .map_err(|_| ApiError::Internal)?
            .map_err(|_| ApiError::Internal)?
    };
    if !found {
        return Err(ApiError::NotFound(format!("no model with id {id}")));
    }
    Ok(Json(json!({ "ok": true })))
}
