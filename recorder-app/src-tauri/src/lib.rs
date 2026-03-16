pub mod camera_watcher;
pub mod commands;
pub mod config;
mod disk;
pub mod preview;
pub mod recorder;
pub mod state;
pub mod video;
pub mod library;
pub mod upload;
pub mod dataset;

use state::AppState;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let first_run = config::is_first_run();
            let app_config = config::load_config();

            let app_state = Arc::new(AppState::new(app_config, first_run));

            // Spawn video stream server
            let state_for_video = Arc::clone(&app_state);
            let state_for_port = Arc::clone(&app_state);
            tauri::async_runtime::spawn(async move {
                match video::video_server::spawn_video_server(state_for_video).await {
                    Ok(port) => {
                        *state_for_port.video_server_port.write() = Some(port);
                        log::info!("Video server started on port {}", port);
                    }
                    Err(e) => {
                        log::error!("Failed to start video server: {}", e);
                    }
                }
            });

            // Spawn upload background loop (always runs, checks upload_enabled internally)
            let state_for_upload = Arc::clone(&app_state);
            let app_handle_upload = app.handle().clone();
            upload::upload_loop::spawn_upload_loop(app_handle_upload, state_for_upload);

            // Spawn camera hotplug watcher
            let state_for_camera = Arc::clone(&app_state);
            let app_handle_camera = app.handle().clone();
            camera_watcher::spawn_camera_watcher(app_handle_camera, state_for_camera);

            // Spawn file watcher for auto-discovering new recordings
            let state_for_watcher = Arc::clone(&app_state);
            let app_handle_watcher = app.handle().clone();
            let watcher_tx =
                library::spawn_file_watcher(app_handle_watcher, Arc::clone(&state_for_watcher));
            {
                let rt = tauri::async_runtime::handle();
                rt.block_on(async {
                    *state_for_watcher.watcher_cmd_tx.lock().await = Some(watcher_tx);
                });
            }

            app.manage(app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Preview (unified preview + recording via subprocess stdin)
            commands::preview_commands::start_preview,
            commands::preview_commands::stop_preview,
            commands::preview_commands::start_recording,
            commands::preview_commands::stop_recording,
            commands::preview_commands::get_preview_state,
            commands::preview_commands::get_camera_info,
            commands::preview_commands::get_preview_url,
            commands::preview_commands::discard_last_recording,
            commands::preview_commands::check_camera,
            // Recorder (legacy commands still needed for status/stats/lid-safe)
            commands::recorder_commands::get_recorder_status,
            commands::recorder_commands::get_recorder_stats,
            commands::recorder_commands::toggle_lid_safe,
            // Library
            commands::library_commands::discover_files,
            commands::library_commands::get_file_metadata,
            commands::library_commands::get_video_server_port,
            commands::library_commands::get_stream_url,
            commands::library_commands::watch_directory,
            commands::library_commands::get_watched_dir,
            // Settings
            commands::settings_commands::get_config,
            commands::settings_commands::save_config,
            commands::settings_commands::is_first_run,
            commands::settings_commands::complete_first_run,
            commands::settings_commands::locate_binary,
            commands::settings_commands::test_camera,
            commands::settings_commands::get_disk_info,
            commands::settings_commands::get_system_info,
            // Dialogs
            commands::dialog_commands::open_directory,
            commands::dialog_commands::select_file,
            // Upload
            commands::upload_commands::queue_upload,
            commands::upload_commands::get_upload_queue,
            commands::upload_commands::get_upload_manifest,
            commands::upload_commands::retry_failed,
            commands::upload_commands::cancel_upload,
            commands::upload_commands::test_upload_connection,
            commands::upload_commands::toggle_auto_upload,
            // Datasets
            commands::dataset_commands::list_datasets,
            commands::dataset_commands::create_dataset,
            commands::dataset_commands::update_dataset,
            commands::dataset_commands::delete_dataset,
            commands::dataset_commands::get_dataset_files,
            commands::dataset_commands::upload_dataset,
            commands::dataset_commands::convert_dataset,
            commands::dataset_commands::get_conversion_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ego-recorder-app");
}
