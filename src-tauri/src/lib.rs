mod database;
mod features;

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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            features::pitches::commands::list_pitches,
            features::pitches::commands::create_pitch,
            features::pitches::commands::update_pitch,
            features::pitches::commands::delete_pitch,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
