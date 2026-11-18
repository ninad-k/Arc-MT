//! The user's notes over HTTP (ADR-0021): the graph for the galaxy page, one note's text, and
//! a rescan. Read-only; the notes folder is never written.

use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

use crate::routes::ApiError;
use crate::state::AppState;

fn service(state: &AppState) -> Result<&std::sync::Arc<jarvis_notes::NotesService>, ApiError> {
    state
        .notes()
        .ok_or_else(|| ApiError::NotFound("notes are turned off ([notes] enabled = false)".into()))
}

fn graph_json(
    svc: &jarvis_notes::NotesService,
    index: Result<std::sync::Arc<jarvis_notes::NotesIndex>, jarvis_notes::NotesError>,
) -> Value {
    let dir = svc.dir().display().to_string();
    match index {
        Ok(ix) => json!({
            "dir": dir,
            "nodes": ix.notes(),
            "links": ix
                .links()
                .into_iter()
                .map(|(a, b)| json!({ "source": a, "target": b }))
                .collect::<Vec<_>>(),
            "skipped": ix.skipped(),
            "error": null,
        }),
        // A missing folder is not a server error: the page explains how to set one.
        Err(e) => json!({
            "dir": dir,
            "nodes": [],
            "links": [],
            "skipped": 0,
            "error": e.to_string(),
        }),
    }
}

/// `GET /api/notes/graph`
pub async fn graph(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let svc = service(&state)?.clone();
    let body = tokio::task::spawn_blocking(move || graph_json(&svc, svc.index()))
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(Json(body))
}

/// `POST /api/notes/rescan`: rebuild the index now.
pub async fn rescan(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let svc = service(&state)?.clone();
    let body = tokio::task::spawn_blocking(move || graph_json(&svc, svc.rescan()))
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(Json(body))
}

/// `GET /api/notes/{id}`: the whole note as text.
pub async fn note(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    let svc = service(&state)?.clone();
    let found = tokio::task::spawn_blocking(move || {
        // One snapshot for the title and the text.
        let ix = svc.index().ok()?;
        let n = ix.get(id)?.clone();
        let text = svc.text_of(&ix, id)?;
        Some(json!({
            "id": n.id,
            "title": n.title,
            "group": n.group,
            "path": n.path,
            "text": text,
        }))
    })
    .await
    .map_err(|_| ApiError::Internal)?;
    found
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("no note with id {id}")))
}
