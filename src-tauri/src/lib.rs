mod ai;
mod database;
mod features;
mod ingest;
mod util;

use std::sync::Mutex;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Local, per-user database. Created + migrated on first launch.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("courland-prospect.db");
            // Fail loud (never auto-discard the file — a buggy future migration
            // must not silently hide real user data), but log why for diagnosis.
            let conn = database::open(&db_path).map_err(|e| {
                eprintln!("Failed to open database at {}: {e}", db_path.display());
                e
            })?;
            app.manage(database::AppState {
                conn: Mutex::new(conn),
            });

            // Ingest: provision the extension + token, then start the loopback
            // server and the Chrome/extension gate. A provisioning failure is
            // reflected in the gate (so the UI can explain it) rather than
            // aborting launch.
            let handle = app.handle().clone();
            app.manage(ingest::gate::GateState::new());
            app.manage(ingest::Heartbeat::new());
            match ingest::config::provision(&handle) {
                Ok(config) => {
                    app.manage(config.clone());
                    // Server receives the extension's heartbeat; the gate reads it.
                    // Only start them once provisioning wrote the token/config the
                    // extension needs — otherwise a provisioning error is the state
                    // to show, not a heartbeat that can never arrive.
                    let gate_handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        ingest::server::serve(handle, config).await;
                    });
                    tauri::async_runtime::spawn(ingest::gate::run(gate_handle));
                }
                Err(e) => {
                    eprintln!("ingest: provisioning failed: {e}");
                    if let Ok(mut status) =
                        app.state::<ingest::gate::GateState>().status.lock()
                    {
                        *status = ingest::gate::GateStatus::Error(e);
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ai::commands::ai_available,
            features::pitches::commands::list_pitches,
            features::pitches::commands::create_pitch,
            features::pitches::commands::update_pitch,
            features::pitches::commands::delete_pitch,
            features::pitches::commands::polish_skill,
            features::profile::commands::get_profile,
            features::profile::commands::update_profile,
            features::profile::commands::polish_who,
            features::profile::commands::polish_building,
            features::prospects::commands::list_prospects,
            features::prospects::commands::delete_prospect,
            features::prospects::commands::set_prospect_stage,
            features::stages::commands::list_stages,
            features::stages::commands::create_stage,
            features::stages::commands::rename_stage,
            features::stages::commands::set_stage_color,
            features::stages::commands::reorder_stages,
            features::stages::commands::delete_stage,
            features::snippets::commands::list_snippets,
            features::snippets::commands::create_snippet,
            features::snippets::commands::update_snippet,
            features::snippets::commands::delete_snippet,
            features::snippets::commands::approve_snippet,
            features::snippets::commands::set_snippet_category,
            features::snippets::commands::copy_snippet,
            features::snippets::commands::reclassify_snippets,
            features::watchlist::commands::list_watched_profiles,
            features::watchlist::commands::add_watched_profile,
            features::watchlist::commands::delete_watched_profile,
            features::comments::commands::list_comment_drafts,
            features::comments::commands::get_comment_run,
            features::comments::commands::request_comment_scrape,
            features::comments::commands::update_comment_draft,
            features::comments::commands::delete_comment_draft,
            features::comments::commands::queue_comment_drafts,
            ingest::gate::gate_status,
            ingest::gate::open_chrome_profile,
            ingest::gate::list_chrome_profiles,
            ingest::gate::extension_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
