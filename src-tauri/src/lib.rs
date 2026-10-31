//! Chup — Phase 1 (v1, Windows). Xem SPEC.md.

mod actions;
mod capture;
mod commands;
mod config;
mod coords;
mod hotkeys;
mod platform;
mod stack;
mod state;
mod tray;
mod wm;

use std::time::Duration;

use tauri::{AppHandle, Manager, RunEvent, Runtime};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use crate::{
    platform::PlatformExt,
    state::{AppState, Paths},
};

/// Đồng bộ "Khởi động cùng Windows" với config.
pub(crate) fn sync_autostart<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let want = app.state::<AppState>().lock().config.launch_at_startup;
    let al = app.autolaunch();
    let is = al.is_enabled().unwrap_or(false);
    let res = match (want, is) {
        (true, false) => al.enable(),
        (false, true) => al.disable(),
        _ => Ok(()),
    };
    res.map_err(|e| format!("Không đổi được chế độ khởi động cùng hệ điều hành: {e}"))
}

fn init<R: Runtime>(app: &AppHandle<R>) -> Result<(), Box<dyn std::error::Error>> {
    let path = app.path();
    let pictures = path
        .picture_dir()
        .or_else(|_| path.home_dir().map(|h| h.join("Pictures")))?;
    let default_save_dir = pictures.join("Chup");
    let config_file = path.app_config_dir()?.join("config.json");
    let thumbs_dir = path.app_cache_dir()?.join("thumbs");
    // Stack rỗng khi khởi động (SPEC 2) -> dọn thumbnail cũ.
    let _ = std::fs::remove_dir_all(&thumbs_dir);

    let loaded = config::load(&config_file, &default_save_dir);
    app.manage(AppState::new(
        loaded.config,
        Paths {
            config_file,
            thumbs_dir,
            default_save_dir,
        },
    ));

    tray::create(app)?;
    wm::create_static_windows(app)?;
    wm::sync_overlays(app);

    // SPEC 6.4: loại frame/toolbar/stack khỏi ảnh chụp. Setup chạy trên main thread.
    let excluded = [wm::FRAME, wm::TOOLBAR, wm::STACK]
        .iter()
        .all(|l| app.get_webview_window(l).map(|w| w.exclude_from_capture()).unwrap_or(false));
    app.state::<AppState>().lock().capture_exclusion_ok = excluded;

    hotkeys::apply(app);
    if let Err(e) = sync_autostart(app) {
        log::warn!("{e}");
    }
    actions::spawn_monitor_watcher(app.clone());

    if let Some(w) = loaded.warning {
        // Đợi stack webview tải xong mới báo được.
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            actions::toast(&app, "error", w);
        });
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // single-instance phải đăng ký đầu tiên.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { wm::open_settings(&app) });
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            commands::region_selected,
            commands::selection_cancelled,
            commands::frame_state,
            commands::capture_now,
            commands::toggle_fade,
            commands::close_region,
            commands::stack_state,
            commands::stack_report_size,
            commands::stack_remove,
            commands::stack_clear,
            commands::shot_exists,
            commands::shot_open,
            commands::shot_reveal,
            commands::shot_copy,
            commands::settings_get,
            commands::settings_update,
            commands::pick_save_dir,
            commands::open_save_dir,
            commands::hotkeys_pause,
            commands::hotkeys_resume,
        ])
        .setup(|app| init(app.handle()))
        .build(tauri::generate_context!())
        .expect("không khởi tạo được Chup");

    app.run(|_app, event| {
        // Đóng cửa sổ Cài đặt không được thoát app; chỉ "Thoát" ở tray (exit code có giá trị).
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}
