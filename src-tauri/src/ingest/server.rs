//! The loopback HTTP server the Chrome extension talks to. Runs on Tauri's
//! existing tokio runtime (no second runtime). All routes sit behind a layered
//! guard. Capture + drafting:
//!
//!   GET  /health    — extension check-in / liveness
//!   GET  /customers — feeds the dropdown (reuses the customers repository)
//!   GET  /prospect  — is this person a prospect, and which customer (by URL)
//!   POST /prospects — captures a prospect (upsert; reuses the prospects repo)
//!   POST /messages  — records captured messages, both directions (messages repo)
//!   POST /draft     — drafts one reply for an open thread
//!   GET/POST /selectors, /heal-selectors — self-healing LinkedIn selectors
//!
//! The commenter (the extension is a headless worker polling these; the app is
//! the cockpit — see `crate::features::comments`):
//!
//!   GET  /watched-profiles        — the watchlist to visit before the feed
//!   POST /comment-run/claim       — claim a requested scrape (idle→scraping)
//!   POST /comment-run/status      — report run progress (scraping / idle)
//!   POST /comment-drafts          — draft + persist a comment for one post
//!   POST /comment-drafts/claim    — claim queued drafts to post (queued→posting)
//!   POST /comment-draft-status    — report a post outcome (posted / failed)
//!
//! Defenses (see `super::security`): bound to 127.0.0.1, exact `Host` check
//! (anti DNS-rebinding), a shared token header, and a CORS allowlist pinned to
//! the extension's origin. DB work runs in `spawn_blocking` (rusqlite blocks;
//! the `std::sync::Mutex` guard is `!Send` so it can't cross an `.await`).

use std::collections::HashSet;
use std::net::SocketAddr;

use axum::extract::{DefaultBodyLimit, Query, Request, State};
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
use crate::ai::{
    self, comment_is_skip, BrokenSelector, CommentContext, DraftContext, DraftCustomer,
    DraftMessage, DraftSnippet, DraftStage, Prompt,
};
use crate::database::AppState;
use crate::features::{
    comments, customers, messages, product, profile, prospects, selectors, snippets, stages,
    watchlist,
};
use crate::util::{MAX_NAME_LEN, MAX_TEXT_LEN};

/// Ceiling on a single ingest request body. The largest legitimate payload is a
/// batch of captured messages; this bounds a runaway/hostile body well above
/// that while staying small enough that a bad request can't exhaust memory.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Cap on the page HTML forwarded to a heal request. The prompt is fed to `claude`
/// on stdin (not argv), so this isn't an OS command-line limit — it just bounds the
/// prompt size (and the generation cost) to a sane ceiling; the extension also
/// trims before sending.
const MAX_HEAL_HTML: usize = 180_000;

/// Cap on how many broken selectors one heal request may ask about — bounds the
/// prompt size and a runaway/hostile body.
const MAX_BROKEN: usize = 40;

#[derive(Clone)]
struct ServerState {
    app: AppHandle,
    config: IngestConfig,
}

/// Emitted whenever the extension changes prospects (a fresh capture, or messages
/// bumping a derived count) so an open desktop view can re-fetch and reflect it
/// live instead of only on its next mount.
///
/// `pub(crate)` because the advance analyzer
/// (`features::prospects::advance`) writes suggestions from a background task
/// long after the ingest response has been sent, and has to nudge the board the
/// same way. One event name, one meaning: "prospect rows changed underneath you".
pub(crate) const PROSPECTS_CHANGED: &str = "prospects://changed";

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
        .route("/customers", get(list_customers))
        .route("/prospect", get(lookup_prospect))
        .route("/prospects", post(create_prospect))
        .route("/messages", post(create_messages))
        .route("/draft", post(draft_reply))
        .route("/watched-profiles", get(list_watched_profiles))
        .route("/comment-run/claim", post(claim_comment_run))
        .route("/comment-run/status", post(set_comment_run_status))
        .route("/comment-drafts", post(create_comment_draft))
        .route("/comment-drafts/claim", post(claim_comment_drafts))
        .route("/comment-draft-status", post(record_comment_status))
        .route("/selectors", get(get_selectors))
        .route("/heal-selectors", post(heal_selectors))
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

/// Serve the customer profiles for the extension's capture dropdown — which kind
/// of buyer this person is, so their drafts get steered toward that profile's
/// goal. Reads only.
///
/// Projected down to `{id, name}`: that's all the dropdown renders, and the rest
/// of a profile is the user's private positioning work (who they are, their pain,
/// the goal). The draft prompt reads that server-side, so there is no reason to
/// hand it to a content script running inside a LinkedIn page.
async fn list_customers(State(state): State<ServerState>) -> Response {
    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        let list = customers::repository::list(&conn).map_err(|e| e.to_string())?;
        Ok::<_, String>(
            list.into_iter()
                .map(|c| serde_json::json!({ "id": c.id, "name": c.name }))
                .collect::<Vec<_>>(),
        )
    })
    .await;

    match result {
        Ok(Ok(list)) => Json(list).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Query for `GET /prospect` — the canonical LinkedIn URL of the open thread's
/// person (the extension's `normalizeProfileUrl` is the sole producer, matching
/// how capture resolves prospects).
#[derive(Deserialize)]
struct ProspectQuery {
    // `default` so a missing `url` hits the domain-specific "url is required" 400
    // below rather than a generic extractor-level 400 with no message.
    #[serde(default)]
    url: String,
}

/// Look up whether the open thread's person is already a prospect, and which
/// customer profile they match. Returns
/// `{ "exists": bool, "customer_id": number | null }` — the extension uses it to
/// show "Prospect · <customer>" instead of the add control, and to steer each
/// draft toward that profile's goal. `customer_id` is null when the prospect is
/// unassigned (captured without one, or their profile was deleted since), which
/// is a valid state: they stay in the pipeline and their drafts simply get no
/// customer block. Reads only.
async fn lookup_prospect(
    State(state): State<ServerState>,
    Query(q): Query<ProspectQuery>,
) -> Response {
    let url = q.url.trim().to_string();
    if url.is_empty() {
        return (StatusCode::BAD_REQUEST, "url is required").into_response();
    }

    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        prospects::repository::find_by_url(&conn, &url).map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(found)) => Json(serde_json::json!({
            "exists": found.is_some(),
            "customer_id": found.and_then(|p| p.customer_id),
        }))
        .into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Body the extension POSTs. `headline`, `customer_id`, and `note` are optional.
///
/// `deny_unknown_fields` so a stale extension build fails loudly instead of
/// having its unrecognized field dropped on the floor: a pre-rework build sends
/// `pitch_id`, which would otherwise be ignored and the capture silently filed
/// unassigned with a success toast.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewProspect {
    name: String,
    linkedin_url: String,
    #[serde(default)]
    headline: String,
    #[serde(default)]
    customer_id: Option<i64>,
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
        // A profile deleted between the widget populating its dropdown and this
        // request landing is not an error — the same race `/draft` already
        // tolerates. Drop the dangling id rather than tripping the foreign key: a
        // 500 here loses the capture entirely and keeps failing until the LinkedIn
        // tab is reloaded, over a tag the user can set in one click afterwards.
        // (`upsert` COALESCEs, so on a re-capture this leaves the existing tag.)
        let customer_id = match body.customer_id {
            Some(id) => customers::repository::find(&conn, id)
                .map_err(|e| e.to_string())?
                .map(|c| c.id),
            None => None,
        };
        let prospect = prospects::repository::upsert(
            &conn,
            &name,
            &url,
            body.headline.trim(),
            customer_id,
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
/// best-effort; `customer_id` is the profile to steer toward, and is optional —
/// omitted (or naming a since-deleted profile) simply drafts without a goal.
///
/// `linkedin_url` is the person's PROFILE url (`/in/<slug>/`), used for one
/// lookup: which cycle stage this prospect sits in, so the reply can aim at that
/// step's goal rather than the whole relationship's. Optional throughout —
/// omitted, unknown, or naming a non-prospect all drop the stage block and draft
/// exactly as before. Note this is the ONLY stateful thing the route reads about
/// the person; the conversation still comes from the live scrape.
///
/// `deny_unknown_fields` for the same reason as `NewProspect`: a stale build's
/// `pitch_id` should be a visible error, not a silently unsteered draft. Which
/// is also why `linkedin_url` had to land here before the extension started
/// sending it — an unknown field is a 422, not a degraded draft.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftRequest {
    #[serde(default)]
    prospect_name: String,
    #[serde(default)]
    customer_id: Option<i64>,
    #[serde(default)]
    linkedin_url: String,
    #[serde(default)]
    messages: Vec<DraftMsgIn>,
}

/// Draft the next reply for one open LinkedIn thread. Gathers the product, the
/// founder's profile, the whole snippet library, and — when the prospect is
/// matched — the customer profile that steers the reply toward a goal, then builds
/// a strict "compose only from this material" prompt and runs it through the local
/// Claude Code CLI (concurrency-capped in `crate::ai::client`). Returns
/// `{ "draft": .. }` — either a composed reply or the model's ALL-CAPS reason it
/// couldn't build one. Reads only; writes nothing to the DB.
async fn draft_reply(State(state): State<ServerState>, Json(body): Json<DraftRequest>) -> Response {
    let app = state.app.clone();
    let customer_id = body.customer_id;
    let linkedin_url = body.linkedin_url.trim().to_string();

    // Bound untrusted browser input before it becomes a CLI argument.
    if body.prospect_name.chars().count() > MAX_NAME_LEN
        || linkedin_url.chars().count() > MAX_TEXT_LEN
        || body.messages.iter().any(|m| m.body.chars().count() > MAX_TEXT_LEN)
    {
        return (StatusCode::BAD_REQUEST, "field too long").into_response();
    }

    // Gather the compositional material with one lock (rusqlite blocks; the
    // `!Send` guard can't cross an `.await`, so this stays in spawn_blocking).
    let gathered = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        let product = product::repository::get(&conn).map_err(|e| e.to_string())?;
        let profile = profile::repository::get(&conn).map_err(|e| e.to_string())?;
        // A profile deleted between the extension reading the dropdown and this
        // request landing is not an error — it just means no steering, same as an
        // unassigned prospect.
        let customer = match customer_id {
            Some(id) => customers::repository::find(&conn, id).map_err(|e| e.to_string())?,
            None => None,
        };
        // The whole library, approved only — an unreviewed proposal must never leak
        // into a drafted reply. Already in conversation-arc order (openers →
        // closers), which is the order the model composes in; picking which of these
        // serve this customer's goal is the model's job, not a query's.
        let snippets = snippets::repository::list_approved(&conn).map_err(|e| e.to_string())?;
        // Where this thread sits in the cycle, when we can tell. Every step is
        // allowed to come up empty — a blank url, a person who isn't a prospect,
        // a prospect with no stage — and each simply drafts without the block,
        // exactly as before this lookup existed. Never an error: a reply the user
        // is waiting on must not fail because the board couldn't be consulted.
        let stage = match prospects::repository::find_by_url(&conn, &linkedin_url) {
            Ok(Some(p)) => p.stage_id.and_then(|id| {
                stages::repository::find(&conn, id).ok().flatten()
            }),
            Ok(None) => None,
            Err(e) => {
                eprintln!("draft: stage lookup failed, drafting without it: {e}");
                None
            }
        };
        Ok::<_, String>((product, profile, customer, snippets, stage))
    })
    .await;

    let (product, profile, customer, snippets, stage) = match gathered {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    // Only content-bearing snippets are worth sending; drop the blank cards the
    // editor leaves behind. Each carries its conversation stage (`category`) so the
    // composer can prefer stage-appropriate lines for where the thread sits.
    let draft_snippets: Vec<DraftSnippet> = fit_to_budget(
        snippets
            .into_iter()
            .filter(|s| !s.content.trim().is_empty())
            .map(|s| DraftSnippet {
                stage: s.category,
                topic: s.topic,
                name: s.name,
                content: s.content,
            })
            .collect(),
    );

    // Nothing to compose from → don't burn a CLI call. Return the same shape of
    // ALL-CAPS refusal the model would, so the composer treatment is consistent.
    // A customer profile alone isn't material: it says who to write to and toward
    // what, but carries nothing you're allowed to actually say.
    let has_material = !product.description.trim().is_empty()
        || !profile.who_are_you.trim().is_empty()
        || !draft_snippets.is_empty();
    if !has_material {
        // `blocked` marks this as a setup problem rather than a composed reply, so
        // the batch runner can stop and say so once instead of writing this
        // sentence into every open composer. The sentinel text stays for a client
        // that predates the flag (and for a single manual draft, where seeing it in
        // the composer is the clearest possible answer).
        return Json(serde_json::json!({
            "draft": "NO SNIPPETS OR PROFILE CONFIGURED YET — ADD MATERIAL IN COURLAND TO DRAFT A REPLY.",
            "blocked": true,
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
        product_name: &product.name,
        product_description: &product.description,
        profile_who: &profile.who_are_you,
        customer: customer.as_ref().map(|c| DraftCustomer {
            name: &c.name,
            who_they_are: &c.who_they_are,
            pain: &c.pain,
            goal: &c.goal,
        }),
        // A stage with no goal steers nothing, so it's dropped here rather than
        // rendered as an empty heading (the renderer guards this too; keeping the
        // rule visible at the call site is cheap).
        stage: stage
            .as_ref()
            .filter(|s| !s.goal.trim().is_empty())
            .map(|s| DraftStage { name: &s.name, goal: &s.goal }),
        snippets: &draft_snippets,
        conversation: &conversation,
    };

    // Enforce the no-literal-bracket rule in code, not just in the instruction.
    //
    // `DRAFT_INSTRUCTION` states that the reply must never contain a literal `[` or `]`,
    // and nothing checked it. Worse, the instruction can talk itself into breaking the
    // rule: its PLACEHOLDERS clause offers "reword the sentence" as the fallback for an
    // unfillable blank, while a later clause insists snippets stay verbatim — and a model
    // resolving that conflict toward verbatim emits the bracket. It only takes an approved
    // snippet with a blank whose only source is the customer profile, plus a thread for
    // someone who isn't a prospect yet (no profile, no stage), for that to be the live
    // case. The draft then lands in the LinkedIn composer with `[their kind of team]` in
    // it. The commenter already guards this exact thing via `sample_voice`; the drafter
    // did not.
    //
    // One retry, then fail closed. Returning an error is honest — the extension shows it
    // and leaves the composer untouched — whereas stripping the bracket would silently
    // ship a sentence with a hole in it.
    for attempt in 0..2 {
        match ai::client::run_capped(Prompt::draft_reply(&ctx)).await {
            Ok(draft) if !draft.contains(['[', ']']) => {
                return Json(serde_json::json!({ "draft": draft })).into_response();
            }
            Ok(_) => {
                eprintln!(
                    "ingest: draft came back with a literal placeholder bracket (attempt {})",
                    attempt + 1
                );
            }
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        }
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "The draft kept coming back with an unfilled [placeholder]. Check that the \
         snippets it needs have a customer profile to fill them from."
            .to_string(),
    )
        .into_response()
}

/// Serve the hand-curated watchlist — the LinkedIn profiles a comment run visits
/// (before the feed) to check for new posts. The extension fetches this at the
/// start of a run. Reads only.
async fn list_watched_profiles(State(state): State<ServerState>) -> Response {
    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        watchlist::repository::list(&conn).map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(list)) => Json(list).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Claim a requested scrape. If the app has queued a run (`status = requested`),
/// atomically flip it to `scraping` and return its budget + watchlist choice;
/// otherwise `{ "run": null }`. The extension polls this and, on a non-null run,
/// does the scraping. The flip is the claim, so two polling workers can't both
/// start the same run.
async fn claim_comment_run(State(state): State<ServerState>) -> Response {
    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        comments::repository::take_requested(&conn).map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(Some(run))) => {
            let _ = state.app.emit(comments::COMMENTS_CHANGED, ());
            Json(serde_json::json!({
                "run": { "count": run.count, "include_watchlist": run.include_watchlist }
            }))
            .into_response()
        }
        Ok(Ok(None)) => Json(serde_json::json!({ "run": null })).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Body for `POST /comment-run/status` — the extension reporting run progress.
#[derive(Deserialize)]
struct RunStatusIn {
    #[serde(default)]
    status: String,
}

/// Report the extension finished a scrape (`status: "idle"`). The claim already put
/// the run into `scraping`, so `idle` is the only status the extension ever reports
/// here — and it's applied CONDITIONALLY (only from `scraping`, see `finish_scrape`)
/// so it can't clobber a re-request that arrived mid-run. Any other value is
/// rejected so a bad body can't wedge the run.
async fn set_comment_run_status(
    State(state): State<ServerState>,
    Json(body): Json<RunStatusIn>,
) -> Response {
    if body.status.trim() != "idle" {
        return (StatusCode::BAD_REQUEST, "status must be idle").into_response();
    }
    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        comments::repository::finish_scrape(&conn).map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(_)) => {
            let _ = state.app.emit(comments::COMMENTS_CHANGED, ());
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Character ceiling on the snippet material in one draft prompt. Deliberately a
/// size budget rather than a count: which snippets serve a given buyer is the
/// model's job (see the caller), so the whole library goes through untouched in the
/// normal case. This only exists so that a library large enough to blow the context
/// window degrades into a slightly thinner prompt instead of a 60s CLI timeout
/// reported as an installation problem. ~30k tokens, which leaves the conversation,
/// profile and instructions comfortable room.
const DRAFT_SNIPPET_BUDGET: usize = 120_000;

/// Keep the snippet set under [`DRAFT_SNIPPET_BUDGET`], preserving conversation-arc
/// coverage. Under budget — the overwhelmingly common case — the set is returned
/// untouched. Over it, an evenly-spaced stride thins the set across the whole arc,
/// so the model still sees openers through closers rather than losing the tail to a
/// truncation. Trimming is logged: a silently shortened prompt would look like the
/// model ignoring material the user can plainly see in their library.
fn fit_to_budget(snippets: Vec<DraftSnippet>) -> Vec<DraftSnippet> {
    let weigh = |s: &DraftSnippet| s.content.len() + s.name.len() + s.stage.len();
    let total: usize = snippets.iter().map(weigh).sum();
    if total <= DRAFT_SNIPPET_BUDGET {
        return snippets;
    }

    let n = snippets.len();
    let mut k = n;
    let (picked, size) = loop {
        // Evenly spaced over the arc; strictly increasing while k <= n.
        let pick: Vec<usize> = (0..k).map(|i| i * n / k).collect();
        let size: usize = pick.iter().map(|&i| weigh(&snippets[i])).sum();
        if size <= DRAFT_SNIPPET_BUDGET || k == 1 {
            break (pick, size);
        }
        // Proportional shrink, with a guaranteed decrement so this always converges.
        let next = k.saturating_mul(DRAFT_SNIPPET_BUDGET) / size.max(1);
        k = if next >= k { k - 1 } else { next.max(1) };
    };

    eprintln!(
        "ingest: snippet library over the draft budget ({total} chars, {n} snippets) — \
         sending an evenly-spaced {} ({size} chars)",
        picked.len()
    );

    let mut src: Vec<Option<DraftSnippet>> = snippets.into_iter().map(Some).collect();
    picked.iter().filter_map(|&i| src[i].take()).collect()
}

/// How many snippets to feed the commenter as voice samples. A handful of varied
/// examples conveys the founder's style; more only adds tokens and tempts the model
/// to lift wording, which the style-only rule forbids (a comment must never reuse a
/// snippet's content).
const COMMENT_VOICE_CAP: usize = 12;

/// Reduce the founder's approved snippet contents to a bounded voice/style corpus
/// for the commenter: drop any snippet carrying a blank, drop duplicate lines
/// (case/space-insensitive, keeping the first), then, if still over the cap, take an
/// evenly-spaced stride across the set so the sample spreads over the whole library
/// instead of clustering on the newest additions.
///
/// Blanks are ELIDED rather than the whole line dropped. A comment is published
/// verbatim under the founder's name, and these samples sit in the prompt as prose to
/// imitate, so a literal `[first name]` must never reach it — but dropping every
/// blanked line threw the voice out with the bracket: a founder whose library is
/// mostly blanked lines got two samples, or none at all, and their comments were then
/// written with no style reference while the templating feature looked like it was
/// working. `elide_blanks` keeps the sentence shape and word choice (which is all a
/// style sample is for) and guarantees no bracket survives; a line that elides to
/// nothing is then dropped as having no prose left.
fn sample_voice(contents: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let kept: Vec<String> = contents
        .into_iter()
        .map(|c| snippets::placeholder::elide_blanks(&c))
        .filter(|c| !c.trim().is_empty())
        .filter(|c| seen.insert(c.trim().to_lowercase()))
        .collect();
    if kept.len() <= COMMENT_VOICE_CAP {
        return kept;
    }
    let n = kept.len();
    (0..COMMENT_VOICE_CAP)
        .map(|i| kept[i * n / COMMENT_VOICE_CAP].clone())
        .collect()
}

/// Body the extension POSTs per scraped post to draft + persist a comment.
#[derive(Deserialize)]
struct CommentDraftIn {
    #[serde(default)]
    permalink: String,
    #[serde(default)]
    author_name: String,
    #[serde(default)]
    post_text: String,
}

/// Draft a comment for one scraped post and persist it to the inbox. Dedup +
/// budget are driven by the response the extension counts against the run's
/// `count`:
///  - `exists`  — already in the inbox (any status); no CLI call, no insert.
///  - `skipped` — the model judged the post not worth engaging; no insert, so a
///                later scrape can resurface it once the post has evolved.
///  - `created` — a new draft row (returned). The extension counts these toward
///                the placed-draft budget.
/// The comment composes from the founder's profile and product as PERSONA (who is
/// speaking, never something to promote) plus their snippets used purely as a
/// voice/style corpus (never their content) — and, unlike a draft, no customer
/// profile and no goal, so nothing steers it toward an outcome. It reads as a peer
/// who sounds like the founder, not pitch copy.
async fn create_comment_draft(
    State(state): State<ServerState>,
    Json(body): Json<CommentDraftIn>,
) -> Response {
    let permalink = body.permalink.trim().to_string();
    if permalink.is_empty() {
        return (StatusCode::BAD_REQUEST, "permalink is required").into_response();
    }
    if permalink.chars().count() > MAX_TEXT_LEN
        || body.author_name.chars().count() > MAX_NAME_LEN
        || body.post_text.chars().count() > MAX_TEXT_LEN
    {
        return (StatusCode::BAD_REQUEST, "field too long").into_response();
    }

    // Already handled → skip without burning a CLI call.
    let app = state.app.clone();
    let plink = permalink.clone();
    let known = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        comments::repository::exists(&conn, &plink).map_err(|e| e.to_string())
    })
    .await;
    match known {
        Ok(Ok(true)) => return Json(serde_json::json!({ "result": "exists" })).into_response(),
        Ok(Ok(false)) => {}
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }

    // An empty post has nothing to react to — treat as a skip (no CLI).
    if body.post_text.trim().is_empty() {
        return Json(serde_json::json!({ "result": "skipped" })).into_response();
    }

    // Gather the persona (who is speaking + what they work on) plus the founder's
    // approved snippets. The profile and product are persona to write FROM; the
    // snippets are a VOICE/STYLE corpus only — the model mimics how they're written,
    // never reuses their content, so no sales material leaks into a public comment.
    let app = state.app.clone();
    let gathered = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        let profile = profile::repository::get(&conn).map_err(|e| e.to_string())?;
        let product = product::repository::get(&conn).map_err(|e| e.to_string())?;
        let snippets = snippets::repository::list_all_approved(&conn).map_err(|e| e.to_string())?;
        Ok::<_, String>((profile, product, snippets))
    })
    .await;
    let (profile, product, snippets) = match gathered {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let voice_samples = sample_voice(snippets.into_iter().map(|s| s.content).collect());

    let ctx = CommentContext {
        author_name: body.author_name.trim(),
        post_text: body.post_text.trim(),
        profile_who: &profile.who_are_you,
        product_name: &product.name,
        product_description: &product.description,
        voice_samples: &voice_samples,
    };
    let comment = match ai::client::run_capped(Prompt::draft_comment(&ctx)).await {
        Ok(out) if comment_is_skip(&out) => {
            return Json(serde_json::json!({ "result": "skipped" })).into_response()
        }
        Ok(out) => out,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    // Persist the draft. A lost insert race (a concurrent scrape beat us to this
    // permalink) comes back None → report `exists`, same as the pre-check.
    let author = body.author_name.trim().to_string();
    let post_text = body.post_text.trim().to_string();
    let app = state.app.clone();
    let stored = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        comments::repository::insert_generated(&conn, &permalink, &author, &post_text, &comment)
            .map_err(|e| e.to_string())
    })
    .await;

    match stored {
        Ok(Ok(Some(draft))) => {
            let _ = state.app.emit(comments::COMMENTS_CHANGED, ());
            Json(serde_json::json!({
                "result": "created",
                "draft": serde_json::to_value(&draft).unwrap_or(serde_json::Value::Null),
            }))
            .into_response()
        }
        Ok(Ok(None)) => Json(serde_json::json!({ "result": "exists" })).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Body for `POST /comment-drafts/claim`.
#[derive(Deserialize)]
struct ClaimIn {
    #[serde(default)]
    limit: Option<u32>,
}

/// Cap on how many queued drafts one claim hands out — small so posting drains in
/// paced batches across polls (an evicted service worker resumes on the next
/// alarm) rather than one long run that could be killed mid-way.
const MAX_CLAIM: u32 = 5;

/// Claim a batch of queued drafts to post: flips them `queued` → `posting` and
/// returns the id + permalink + comment for each. The extension opens each post,
/// submits, then reports the outcome to `/comment-draft-status`.
async fn claim_comment_drafts(
    State(state): State<ServerState>,
    Json(body): Json<ClaimIn>,
) -> Response {
    let limit = body.limit.unwrap_or(MAX_CLAIM).clamp(1, MAX_CLAIM);
    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        comments::repository::claim_queued(&conn, limit).map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(drafts)) => {
            if !drafts.is_empty() {
                let _ = state.app.emit(comments::COMMENTS_CHANGED, ());
            }
            let out: Vec<_> = drafts
                .iter()
                .map(|d| {
                    serde_json::json!({ "id": d.id, "permalink": d.permalink, "comment": d.comment })
                })
                .collect();
            Json(serde_json::json!({ "drafts": out })).into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Body for `POST /comment-draft-status` — the extension reporting a post outcome.
#[derive(Deserialize)]
struct DraftStatusIn {
    // `default` (0) so a missing `id` hits the explicit "id is required" 400 below
    // rather than a generic extractor 400 — and 0 never silently matches a real row.
    #[serde(default)]
    id: i64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    error: String,
}

/// Record the outcome of posting one claimed draft: `posted` (done) or `failed`
/// (retryable — the next "Post all" re-queues it). Any other status is rejected.
async fn record_comment_status(
    State(state): State<ServerState>,
    Json(body): Json<DraftStatusIn>,
) -> Response {
    if body.id <= 0 {
        return (StatusCode::BAD_REQUEST, "id is required").into_response();
    }
    let status = body.status.trim().to_string();
    if status != comments::repository::STATUS_POSTED
        && status != comments::repository::STATUS_FAILED
    {
        return (StatusCode::BAD_REQUEST, "status must be posted or failed").into_response();
    }
    let error: String = body.error.trim().chars().take(MAX_TEXT_LEN).collect();
    let id = body.id;
    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        comments::repository::set_status(&conn, id, &status, &error).map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(_)) => {
            let _ = state.app.emit(comments::COMMENTS_CHANGED, ());
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Serve the stored LinkedIn-selector overrides — a JSON object of `{key: value}`
/// the extension merges over its compiled defaults at startup. `{}` when none
/// have been healed yet. Reads only.
async fn get_selectors(State(state): State<ServerState>) -> Response {
    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        selectors::repository::get_overrides(&conn).map_err(|e| e.to_string())
    })
    .await;

    match result {
        // The stored blob is already a JSON object; parse it so we always emit
        // valid JSON (fall back to `{}` if it were ever somehow malformed).
        Ok(Ok(json)) => {
            let value: serde_json::Value =
                serde_json::from_str(&json).unwrap_or_else(|_| serde_json::json!({}));
            Json(value).into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// One selector the extension reports as broken.
#[derive(Deserialize)]
struct BrokenIn {
    #[serde(default)]
    key: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    current: String,
}

/// Body the extension POSTs when a selector stops matching: the live page HTML and
/// the broken selectors to repair. (The extension also sends a `url` for context;
/// serde ignores unread fields, so it isn't declared here.)
#[derive(Deserialize)]
struct HealRequest {
    #[serde(default)]
    html: String,
    #[serde(default)]
    broken: Vec<BrokenIn>,
}

/// Repair stale LinkedIn selectors. Sends the live page HTML + the broken keys
/// to the local Claude Code CLI (`crate::ai`), keeps only valid replacements for
/// the requested keys, merges them into the stored overrides, and returns ONLY the
/// keys healed this round (not the full merged map — see the response below). The
/// extension applies them live. Writes the overrides; nothing else.
async fn heal_selectors(State(state): State<ServerState>, Json(body): Json<HealRequest>) -> Response {
    let broken_in = body.broken;
    if broken_in.is_empty() {
        return (StatusCode::BAD_REQUEST, "no broken selectors given").into_response();
    }
    if broken_in.len() > MAX_BROKEN {
        return (StatusCode::BAD_REQUEST, "too many selectors").into_response();
    }
    // Bound untrusted browser input before it goes into the heal prompt.
    if broken_in.iter().any(|b| {
        b.key.trim().is_empty()
            || b.key.chars().count() > MAX_NAME_LEN
            || b.description.chars().count() > MAX_TEXT_LEN
            || b.current.chars().count() > MAX_TEXT_LEN
    }) {
        return (StatusCode::BAD_REQUEST, "selector field invalid or too long").into_response();
    }

    // Bound the HTML that goes into the heal prompt (fed to `claude` on stdin).
    let html: String = body.html.chars().take(MAX_HEAL_HTML).collect();

    let requested: HashSet<String> = broken_in.iter().map(|b| b.key.clone()).collect();
    let broken: Vec<BrokenSelector> = broken_in
        .into_iter()
        .map(|b| BrokenSelector {
            key: b.key,
            description: b.description,
            current: b.current,
        })
        .collect();

    let raw = match ai::client::run_capped(Prompt::heal_selectors(&html, &broken)).await {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    let healed = parse_healed(&raw, &requested);
    if healed.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Claude Code returned no usable selectors.",
        )
            .into_response();
    }

    // Respond with ONLY the keys healed this round. The extension gauges success by
    // how many of the returned keys actually apply client-side; returning the full
    // merged map (including unrelated pre-existing overrides) would let a heal that
    // fixed nothing report success — and mark the still-broken key as done, blocking
    // its retry. The merged map is still what we persist and serve at bootstrap.
    let delta = serde_json::Value::Object(healed.clone());

    // Merge the healed keys into the stored overrides and persist (one lock).
    let app = state.app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        let current = selectors::repository::get_overrides(&conn).map_err(|e| e.to_string())?;
        // Fail loud on a non-object blob rather than silently reducing it: writing a
        // fresh map here would discard every prior override. (Unreachable in normal
        // operation — every write serializes an object and the migration seeds `{}` —
        // but the migrations contract is "never discard the user's data".)
        let mut obj = match serde_json::from_str::<serde_json::Value>(&current) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => {
                return Err(
                    "stored selector overrides are not a JSON object; refusing to overwrite"
                        .to_string(),
                )
            }
        };
        for (k, v) in healed {
            obj.insert(k, v);
        }
        let serialized = serde_json::to_string(&obj).map_err(|e| e.to_string())?;
        selectors::repository::set_overrides(&conn, &serialized).map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "selectors": delta })).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Upper bound on a single healed selector value's length. A real CSS selector or
/// class token is short; anything longer is model junk (prose, a pasted blob) and
/// is rejected before it can be persisted.
const MAX_SELECTOR_LEN: usize = 400;

/// A conservative, syntax-agnostic sanity check on one healed selector value.
/// Full CSS-validity checking needs a browser parser and stays client-side (the
/// extension's `isValidCss`, the real oracle); this only rejects the obvious junk a
/// confused model emits — empty, over-long, or containing characters (`{`, `}`,
/// newlines) that never appear in a selector or class token — so garbage isn't
/// persisted into the overrides blob. Values that pass here but aren't valid CSS
/// are still filtered by the extension when it applies them.
fn looks_like_selector(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().count() <= MAX_SELECTOR_LEN && !t.contains(['{', '}', '\n', '\r'])
}

/// Extract the healed selector map from Claude's reply, keeping only requested
/// keys whose value is a usable selector string, or a non-empty array of usable
/// selector strings (see [`looks_like_selector`]). Locating the `{...}` object past
/// any prose or ```` ```json ```` fences is [`ai::parse::json_object`]'s job.
fn parse_healed(
    raw: &str,
    requested: &HashSet<String>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    let Some(obj) = ai::parse::json_object(raw) else {
        return out;
    };
    for (k, v) in obj {
        if !requested.contains(&k) {
            continue;
        }
        let usable = match &v {
            serde_json::Value::String(s) => looks_like_selector(s),
            serde_json::Value::Array(items) => {
                !items.is_empty()
                    && items.iter().all(
                        |it| matches!(it, serde_json::Value::String(s) if looks_like_selector(s)),
                    )
            }
            _ => false,
        };
        if usable {
            out.insert(k, v);
        }
    }
    out
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

/// Record a batch of captured messages (both directions). A message of either
/// direction is stored if its person is an existing prospect, at any stage;
/// anything else (unknown person, blank identity) is silently skipped (not an
/// error — the extension still clears it from its outbox). Idempotent via the
/// messages repo's dedup, so replaying an offline-cached batch never
/// double-counts. The whole batch commits atomically: a DB error rolls back and
/// returns 500, so the extension safely retries.
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
        let outcome = messages::repository::store_batch(&tx, &batch).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok::<_, String>(outcome)
    })
    .await;

    match result {
        Ok(Ok(outcome)) => {
            // Only nudge the UI when something actually landed.
            if outcome.stored > 0 {
                let _ = state.app.emit(PROSPECTS_CHANGED, ());
            }
            // Two background passes over what just landed, both fire-and-forget so
            // the extension's outbox clears now rather than behind a CLI call:
            // reusable snippets out of what WE said, and — over threads that moved
            // in either direction — whether the current stage's goal is now met.
            snippets::proposals::spawn(state.app.clone(), outcome.new_outgoing);
            prospects::advance::spawn(state.app.clone(), outcome.touched_prospects);
            Json(serde_json::json!({ "stored": outcome.stored, "skipped": outcome.skipped }))
                .into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{fit_to_budget, parse_healed, DraftSnippet, DRAFT_SNIPPET_BUDGET};
    use std::collections::HashSet;

    fn snippet(name: &str, chars: usize) -> DraftSnippet {
        DraftSnippet {
            stage: "Opener".into(),
            topic: "Workflow".into(),
            name: name.into(),
            content: "x".repeat(chars),
        }
    }

    #[test]
    fn budget_leaves_a_normal_library_untouched() {
        let library: Vec<DraftSnippet> =
            (0..80).map(|i| snippet(&format!("s{i}"), 500)).collect();
        let kept = fit_to_budget(library);
        assert_eq!(kept.len(), 80, "a 40k-char library is well under budget");
    }

    #[test]
    fn budget_thins_an_oversized_library_and_spans_the_whole_arc() {
        // 400 × 1000 chars = 400k, comfortably over the budget.
        let library: Vec<DraftSnippet> =
            (0..400).map(|i| snippet(&format!("s{i:03}"), 1000)).collect();
        let kept = fit_to_budget(library);

        let size: usize = kept.iter().map(|s| s.content.len() + s.name.len() + s.stage.len()).sum();
        assert!(size <= DRAFT_SNIPPET_BUDGET, "trimmed set must fit the budget, got {size}");
        assert!(!kept.is_empty(), "must never trim to nothing");

        // Evenly spaced, not truncated: the last snippet in the arc survives, and the
        // sample reaches into the back half of the library.
        assert_eq!(kept[0].name, "s000");
        let last: usize = kept.last().unwrap().name[1..].parse().unwrap();
        assert!(last > 200, "sample must span the arc, deepest kept was s{last:03}");
    }

    #[test]
    fn budget_survives_a_single_snippet_larger_than_the_whole_budget() {
        let kept = fit_to_budget(vec![snippet("huge", DRAFT_SNIPPET_BUDGET * 2)]);
        assert_eq!(kept.len(), 1, "one over-budget snippet is still better than none");
    }

    fn requested(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn keeps_requested_string_and_array_values() {
        let req = requested(&["composeRoot", "identityHeaders"]);
        let raw = r#"{"composeRoot": ".foo", "identityHeaders": ["a", "b"]}"#;
        let out = parse_healed(raw, &req);
        assert_eq!(out.len(), 2);
        assert_eq!(out["composeRoot"], serde_json::json!(".foo"));
        assert_eq!(out["identityHeaders"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn tolerates_prose_and_code_fences_around_the_object() {
        let req = requested(&["composeRoot"]);
        let raw = "Sure! Here you go:\n```json\n{\"composeRoot\": \".foo\"}\n```\nHope that helps.";
        let out = parse_healed(raw, &req);
        assert_eq!(out["composeRoot"], serde_json::json!(".foo"));
    }

    #[test]
    fn drops_unrequested_keys_and_invalid_values() {
        let req = requested(&["composeRoot", "messageItem"]);
        // sneaky: an unrequested key, an empty string, an array with a non-string,
        // an empty array, and a nested object — all must be rejected.
        let raw = r#"{
            "composeRoot": ".ok",
            "evilKey": ".nope",
            "messageItem": "",
            "identityHeaders": ["a", 3],
            "x": [],
            "y": {"nested": true}
        }"#;
        let out = parse_healed(raw, &req);
        assert_eq!(out.len(), 1, "only the one valid, requested key survives");
        assert_eq!(out["composeRoot"], serde_json::json!(".ok"));
    }

    #[test]
    fn empty_on_garbage_or_non_object() {
        let req = requested(&["composeRoot"]);
        assert!(parse_healed("not json at all", &req).is_empty());
        assert!(parse_healed("[1, 2, 3]", &req).is_empty());
        assert!(parse_healed("", &req).is_empty());
    }

    #[test]
    fn rejects_structurally_junk_selector_values() {
        let req = requested(&["composeRoot", "sendButtonClasses"]);
        // A value carrying braces (a pasted rule / prose blob) and one with a newline
        // are not selectors — both must be dropped, even though they're non-empty.
        let raw = r#"{
            "composeRoot": ".msg-form { color: red }",
            "sendButtonClasses": ["ok-button", "line one\nline two"]
        }"#;
        let out = parse_healed(raw, &req);
        assert!(out.is_empty(), "brace/newline values are not usable selectors");

        // A real attribute selector (with brackets and quotes) is fine.
        let ok = parse_healed(r#"{"composeRoot": "[class~=\"msg-form\"]"}"#, &req);
        assert_eq!(ok["composeRoot"], serde_json::json!("[class~=\"msg-form\"]"));
    }
}
