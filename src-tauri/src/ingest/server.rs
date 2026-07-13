//! The loopback HTTP server the Chrome extension talks to. Runs on Tauri's
//! existing tokio runtime (no second runtime). Three routes, all behind a
//! layered guard:
//!
//!   GET  /health    — extension check-in / liveness
//!   GET  /pitches   — feeds the dropdown (reuses the pitches repository)
//!   POST /prospects — captures a prospect (upsert; reuses the prospects repo)
//!   POST /messages  — records captured messages, both directions (messages repo)
//!
//! Defenses (see `super::security`): bound to 127.0.0.1, exact `Host` check
//! (anti DNS-rebinding), a shared token header, and a CORS allowlist pinned to
//! the extension's origin. DB work runs in `spawn_blocking` (rusqlite blocks;
//! the `std::sync::Mutex` guard is `!Send` so it can't cross an `.await`).

use std::net::SocketAddr;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderName, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager};
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::security::{self, TOKEN_HEADER};
use super::{gate, Heartbeat, IngestConfig};
use crate::ai::{self, DraftContext, DraftMessage, Prompt};
use crate::database::AppState;
use crate::features::{messages, pitches, profile, prospects, snippets};
use crate::util::{MAX_NAME_LEN, MAX_TEXT_LEN};

/// Ceiling on a single ingest request body. The largest legitimate payload is a
/// batch of captured messages; this bounds a runaway/hostile body well above
/// that while staying small enough that a bad request can't exhaust memory.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
struct ServerState {
    app: AppHandle,
    config: IngestConfig,
}

/// Emitted whenever the extension changes prospects (a fresh capture, or messages
/// bumping a derived count) so an open desktop view can re-fetch and reflect it
/// live instead of only on its next mount.
const PROSPECTS_CHANGED: &str = "prospects://changed";

/// Bind and serve the ingest API. On a startup failure it drives the gate to
/// `Error` (rather than returning silently) so the UI shows the *real* fault
/// instead of misdiagnosing it as "Chrome closed / extension missing" — which is
/// what happens when the server never comes up and no heartbeat can ever arrive.
pub async fn serve(app: AppHandle, config: IngestConfig) {
    let state = ServerState {
        app: app.clone(),
        config: config.clone(),
    };

    let cors = match format!("chrome-extension://{}", config.extension_id).parse() {
        Ok(origin) => CorsLayer::new()
            .allow_origin(AllowOrigin::exact(origin))
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([header::CONTENT_TYPE, HeaderName::from_static(TOKEN_HEADER)]),
        Err(e) => {
            eprintln!("ingest: bad extension origin, server not started: {e}");
            gate::set_error(&app, "The capture server couldn't start (bad extension origin).");
            return;
        }
    };

    let router = Router::new()
        .route("/health", get(health))
        .route("/pitches", get(list_pitches))
        .route("/prospects", post(create_prospect))
        .route("/messages", post(create_messages))
        .route("/draft", post(draft_reply))
        // Guard is inner (runs after CORS), so CORS preflight is answered without
        // a token; real requests still pass the Host/token/Origin checks.
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(cors)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], config.app_port));
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            if let Err(e) = axum::serve(listener, router).await {
                eprintln!("ingest: server error: {e}");
                gate::set_error(&app, "The capture server stopped unexpectedly. Restart the app.");
            }
        }
        Err(e) => {
            eprintln!("ingest: failed to bind {addr}: {e}");
            gate::set_error(
                &app,
                "The capture server couldn't start (port unavailable). Restart the app.",
            );
        }
    }
}

/// Host + token (+ Origin when present) gate for every non-preflight request.
/// Returns opaque 401/403 without revealing which check failed.
async fn guard(State(state): State<ServerState>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let token = headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok());

    if !security::host_ok(host, state.config.app_port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    // A DNS-rebound page would send a foreign Origin; reject it. Non-browser
    // clients send none — they still must present the token below.
    if origin.is_some() && !security::origin_ok(origin, &state.config.extension_id) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !security::token_ok(token, &state.config.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // Auth passed → this is genuinely our extension talking. Record the check-in
    // so the gate knows Chrome + the extension are alive (the extension's
    // periodic GET /health is the pulse; any authed call also counts).
    if let Some(hb) = state.app.try_state::<Heartbeat>() {
        hb.beat();
    }
    next.run(req).await
}

async fn health() -> Response {
    Json(serde_json::json!({ "ok": true, "app": "courland-prospect" })).into_response()
}

async fn list_pitches(State(state): State<ServerState>) -> Response {
    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        pitches::repository::list(&conn).map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(list)) => Json(list).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Body the extension POSTs. `headline`, `pitch_id`, and `note` are optional.
#[derive(Deserialize)]
struct NewProspect {
    name: String,
    linkedin_url: String,
    #[serde(default)]
    headline: String,
    #[serde(default)]
    pitch_id: Option<i64>,
    #[serde(default)]
    note: String,
}

async fn create_prospect(
    State(state): State<ServerState>,
    Json(body): Json<NewProspect>,
) -> Response {
    let name = body.name.trim().to_string();
    let url = body.linkedin_url.trim().to_string();
    if name.is_empty() || url.is_empty() {
        return (StatusCode::BAD_REQUEST, "name and linkedin_url are required").into_response();
    }
    // Bound untrusted browser input before it reaches the DB.
    if name.chars().count() > MAX_NAME_LEN
        || url.chars().count() > MAX_TEXT_LEN
        || body.headline.trim().chars().count() > MAX_NAME_LEN
        || body.note.trim().chars().count() > MAX_TEXT_LEN
    {
        return (StatusCode::BAD_REQUEST, "field too long").into_response();
    }

    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        // Existence-before-upsert so the extension can distinguish a fresh
        // capture from a dedup update in its feedback toast.
        let existed = prospects::repository::exists(&conn, &url).map_err(|e| e.to_string())?;
        let prospect = prospects::repository::upsert(
            &conn,
            &name,
            &url,
            body.headline.trim(),
            body.pitch_id,
            body.note.trim(),
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>((existed, prospect))
    })
    .await;

    match result {
        Ok(Ok((existed, prospect))) => {
            let _ = state.app.emit(PROSPECTS_CHANGED, ());
            Json(serde_json::json!({
                "existed": existed,
                "prospect": serde_json::to_value(&prospect).unwrap_or(serde_json::Value::Null),
            }))
            .into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// One prior message in the thread, as the extension scraped it. `direction` is
/// `"incoming"` (from the prospect) or anything else (treated as yours), matching
/// the messages capture vocabulary.
#[derive(Deserialize)]
struct DraftMsgIn {
    #[serde(default)]
    direction: String,
    #[serde(default)]
    body: String,
}

/// Body the extension POSTs to draft one reply. The conversation is scraped live
/// from the open thread and sent here (not read from the DB), so this works for
/// any thread — including people who aren't prospects yet. `prospect_name` is
/// best-effort; `pitch_id` chooses which snippet library to compose from. Drafting
/// is stateless (no prospect lookup), so no `linkedin_url` is needed here.
#[derive(Deserialize)]
struct DraftRequest {
    #[serde(default)]
    prospect_name: String,
    pitch_id: i64,
    #[serde(default)]
    messages: Vec<DraftMsgIn>,
}

/// Draft the next reply for one open LinkedIn thread. Gathers the chosen pitch's
/// snippets plus the global profile's snippets + profile text, builds a strict
/// "compose only from this material" prompt, and runs it through the local Claude
/// Code CLI (concurrency-capped in `crate::ai::client`). Returns `{ "draft": .. }`
/// — either a composed reply or the model's ALL-CAPS reason it couldn't build one.
/// Reads only; writes nothing to the DB.
async fn draft_reply(State(state): State<ServerState>, Json(body): Json<DraftRequest>) -> Response {
    let app = state.app.clone();
    let pitch_id = body.pitch_id;

    // Bound untrusted browser input before it becomes a CLI argument.
    if body.prospect_name.chars().count() > MAX_NAME_LEN
        || body.messages.iter().any(|m| m.body.chars().count() > MAX_TEXT_LEN)
    {
        return (StatusCode::BAD_REQUEST, "field too long").into_response();
    }

    // Gather the compositional material with one lock (rusqlite blocks; the
    // `!Send` guard can't cross an `.await`, so this stays in spawn_blocking).
    let gathered = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        let pitch = pitches::repository::get(&conn, pitch_id).map_err(|e| e.to_string())?;
        let profile = profile::repository::get(&conn).map_err(|e| e.to_string())?;
        let mut snippets =
            snippets::repository::list(&conn, Some(pitch_id)).map_err(|e| e.to_string())?;
        snippets.extend(snippets::repository::list(&conn, None).map_err(|e| e.to_string())?);
        Ok::<_, String>((pitch, profile, snippets))
    })
    .await;

    let (pitch, profile, snippets) = match gathered {
        Ok(Ok(v)) => v,
        // A missing pitch (deleted mid-batch) surfaces here as a DB error → 500.
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    // Only content-bearing snippets are worth sending; drop the blank cards the
    // editor leaves behind.
    let snippet_pairs: Vec<(String, String)> = snippets
        .into_iter()
        .filter(|s| !s.content.trim().is_empty())
        .map(|s| (s.name, s.content))
        .collect();

    // Nothing to compose from → don't burn a CLI call. Return the same shape of
    // ALL-CAPS refusal the model would, so the composer treatment is consistent.
    let has_material = !pitch.skill.trim().is_empty()
        || !profile.who_are_you.trim().is_empty()
        || !profile.what_building.trim().is_empty()
        || !snippet_pairs.is_empty();
    if !has_material {
        return Json(serde_json::json!({
            "draft": "NO SNIPPETS OR PROFILE CONFIGURED YET — ADD MATERIAL IN COURLAND TO DRAFT A REPLY."
        }))
        .into_response();
    }

    let conversation: Vec<DraftMessage> = body
        .messages
        .iter()
        .filter(|m| !m.body.trim().is_empty())
        .map(|m| DraftMessage {
            incoming: m.direction.trim() == messages::repository::INCOMING,
            body: m.body.trim().to_string(),
        })
        .collect();

    let ctx = DraftContext {
        prospect_name: body.prospect_name.trim(),
        pitch_name: &pitch.name,
        pitch_skill: &pitch.skill,
        profile_who: &profile.who_are_you,
        profile_building: &profile.what_building,
        snippets: &snippet_pairs,
        conversation: &conversation,
    };

    match ai::client::run_capped(Prompt::draft_reply(&ctx)).await {
        Ok(draft) => Json(serde_json::json!({ "draft": draft })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// One captured message the extension POSTs. `sent_at` is optional (LinkedIn
/// doesn't always expose a scrapeable timestamp). `direction` distinguishes an
/// outgoing message you sent from an incoming reply; it defaults to outgoing so
/// an older extension (which sent no direction) keeps its prior behavior.
#[derive(Deserialize)]
struct NewMessage {
    linkedin_url: String,
    li_key: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    sent_at: Option<String>,
    #[serde(default)]
    direction: String,
}

/// Record a batch of captured messages (both directions). An outgoing message
/// is stored only if its person is a prospect in their pitch's messaging stage;
/// an incoming reply is stored for any existing prospect at any stage. Anything
/// else is silently skipped (not an error — the extension still clears it from
/// its outbox). Idempotent via the messages repo's dedup, so replaying an
/// offline-cached batch never double-counts. The whole batch commits atomically:
/// a DB error rolls back and returns 500, so the extension safely retries.
async fn create_messages(
    State(state): State<ServerState>,
    Json(items): Json<Vec<NewMessage>>,
) -> Response {
    if items.is_empty() {
        return Json(serde_json::json!({ "stored": 0, "skipped": 0 })).into_response();
    }
    // Bound untrusted browser input before it reaches the DB.
    if items.iter().any(|m| {
        m.body.chars().count() > MAX_TEXT_LEN
            || m.linkedin_url.chars().count() > MAX_TEXT_LEN
            || m.li_key.chars().count() > MAX_TEXT_LEN
    }) {
        return (StatusCode::BAD_REQUEST, "message field too long").into_response();
    }

    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        // Trim into borrowed repository input; serde stays in this transport layer.
        let batch: Vec<_> = items
            .iter()
            .map(|item| messages::repository::CapturedMessage {
                linkedin_url: item.linkedin_url.trim(),
                li_key: item.li_key.trim(),
                body: item.body.trim(),
                sent_at: item.sent_at.as_deref(),
                direction: item.direction.trim(),
            })
            .collect();

        let st = app.state::<AppState>();
        let mut conn = st.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let counts = messages::repository::store_batch(&tx, &batch).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok::<_, String>(counts)
    })
    .await;

    match result {
        Ok(Ok((stored, skipped))) => {
            // Only nudge the UI when something actually landed.
            if stored > 0 {
                let _ = state.app.emit(PROSPECTS_CHANGED, ());
            }
            Json(serde_json::json!({ "stored": stored, "skipped": skipped })).into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
