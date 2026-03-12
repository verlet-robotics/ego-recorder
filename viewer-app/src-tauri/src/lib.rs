pub mod commands;
mod config;
mod h264_annex_b;
mod mp4_mux;
pub mod pipeline;
pub mod recent;
mod state;
mod video_server;

use state::AppState;
use std::sync::Arc;
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_state = Arc::new(AppState::new());

            // Check for CLI args: --dir or --workspace
            if let Some(dir) = std::env::args()
                .position(|a| a == "--dir")
                .and_then(|i| std::env::args().nth(i + 1))
            {
                let path = std::path::PathBuf::from(&dir);
                if path.is_dir() {
                    *app_state.recordings_dir.write() = Some(path);
                    log::info!("Recordings dir from CLI: {}", dir);
                }
            }

            // If no CLI --dir, restore from persisted config
            if app_state.recordings_dir.read().is_none() {
                if let Ok(data_dir) = app.path().app_data_dir() {
                    if let Some(path) = config::load_recordings_dir(&data_dir) {
                        if path.is_dir() {
                            *app_state.recordings_dir.write() = Some(path.clone());
                            log::info!(
                                "Restored recordings dir from config: {}",
                                path.display()
                            );
                        }
                    }
                }
            }

            if let Some(workspace) = std::env::args()
                .position(|a| a == "--workspace")
                .and_then(|i| std::env::args().nth(i + 1))
            {
                let path = std::path::PathBuf::from(&workspace);
                if path.is_dir() {
                    let has_curation = path.join("curation/v1").is_dir()
                        || path.join("staging/v1").is_dir();
                    if has_curation {
                        *app_state.curation_workspace.write() = Some(path.clone());
                        if let Some(parent) = path.parent() {
                            *app_state.curation_root.write() = Some(parent.to_path_buf());
                        }
                        log::info!("Curation workspace from CLI: {}", workspace);
                    } else {
                        *app_state.curation_root.write() = Some(path);
                        log::info!("Curation root from CLI: {}", workspace);
                    }

                    if let Ok(data_dir) = app.path().app_data_dir() {
                        let _ = recent::touch(&data_dir, &workspace);
                    }
                }
            }

            if let Some(qc) = std::env::args()
                .position(|a| a == "--qc")
                .and_then(|i| std::env::args().nth(i + 1))
            {
                *app_state.qc_binary.write() = qc;
            }

            if let Some(python) = std::env::args()
                .position(|a| a == "--python")
                .and_then(|i| std::env::args().nth(i + 1))
            {
                *app_state.python_binary.write() = python;
            }

            // Spawn video stream server
            let state_for_video = Arc::clone(&app_state);
            let state_for_port = Arc::clone(&app_state);
            tauri::async_runtime::spawn(async move {
                match video_server::spawn_video_server(state_for_video).await {
                    Ok(port) => {
                        *state_for_port.video_server_port.write() = Some(port);
                        log::info!("Video server started on port {}", port);
                    }
                    Err(e) => {
                        log::error!("Failed to start video server: {}", e);
                    }
                }
            });

            let open_dir = MenuItemBuilder::with_id("open_dir", "Open Directory...")
                .accelerator("CmdOrCtrl+O")
                .build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit")
                .accelerator("CmdOrCtrl+Q")
                .build(app)?;
            let file_menu = SubmenuBuilder::new(app, "File")
                .item(&open_dir)
                .separator()
                .item(&quit)
                .build()?;

            let menu = MenuBuilder::new(app).item(&file_menu).build()?;
            app.set_menu(menu)?;

            app.on_menu_event(move |app, event| {
                match event.id().as_ref() {
                    "open_dir" => {
                        let _ = app.emit("menu:open_directory", ());
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                }
            });

            app.manage(app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Dialogs
            commands::dialogs::open_directory,
            commands::dialogs::get_recordings_dir,
            commands::dialogs::set_recordings_dir,
            // Files
            commands::files::discover_files,
            commands::files::list_files,
            commands::files::get_file_metadata,
            // Analysis
            commands::analysis::run_analysis,
            commands::analysis::get_analysis,
            // Operations
            commands::operations::prune_file,
            commands::operations::splice_file,
            commands::operations::restore_file,
            commands::operations::list_pruned,
            // Video
            commands::video::get_video_server_port,
            commands::video::get_stream_url,
            commands::video::get_curation_stream_url,
            // Curation
            commands::curation::get_curation_workspace,
            commands::curation::set_curation_root,
            commands::curation::list_workspaces,
            commands::curation::set_active_workspace,
            commands::curation::set_curation_workspace,
            commands::curation::run_curation_job,
            commands::curation::read_curation_data,
            commands::curation::write_curation_override,
            // Recent workspaces
            commands::curation::get_recent_workspaces,
            commands::curation::remove_recent_workspace,
            commands::curation::update_recent_workspace_alias,
        ])
        .run(tauri::generate_context!())
        .expect("error while running egorec-viewer");
}
