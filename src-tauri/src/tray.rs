//! Icon khay hệ thống (SPEC 5.5).

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

use crate::{actions, commands, state::AppState, wm};

const TRAY_ID: &str = "main";

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "select", "Chọn vùng", true, None::<&str>)?,
            &MenuItem::with_id(app, "recapture", "Chụp lại vùng cuối", true, None::<&str>)?,
            &MenuItem::with_id(app, "open_folder", "Mở folder ảnh", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "Cài đặt…", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Thoát", true, None::<&str>)?,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Chup")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| {
            let app = app.clone();
            let id = ev.id().as_ref().to_string();
            tauri::async_runtime::spawn(async move {
                match id.as_str() {
                    "select" => actions::start_selection(&app),
                    "recapture" => actions::capture(app).await,
                    "open_folder" => {
                        if let Err(e) = commands::open_save_dir_impl(&app) {
                            actions::toast(&app, "error", e);
                        }
                    }
                    "settings" => wm::open_settings(&app),
                    "quit" => app.exit(0),
                    _ => {}
                }
            });
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                let app = tray.app_handle().clone();
                tauri::async_runtime::spawn(async move { actions::start_selection(&app) });
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Tooltip "Chup", kèm cảnh báo nếu có phím tắt đăng ký lỗi.
pub fn refresh_tooltip<R: Runtime>(app: &AppHandle<R>) {
    let errors = app.state::<AppState>().lock().hotkey_errors.clone();
    let text = if errors.is_empty() {
        "Chup".to_string()
    } else {
        format!("Chup — ⚠ {} phím tắt lỗi, xem Cài đặt", errors.len())
    };
    if let Some(t) = app.tray_by_id(TRAY_ID) {
        let _ = t.set_tooltip(Some(text));
    }
}
