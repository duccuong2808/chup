//! Đăng ký phím tắt toàn cục; báo lỗi khi bị app khác chiếm (SPEC 7).

use std::collections::BTreeMap;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{actions, state::AppState, tray};

#[derive(Clone, Copy)]
enum Action {
    SelectRegion,
    Capture,
}

fn dispatch<R: Runtime>(app: &AppHandle<R>, action: Action) {
    // Handler chạy trên main thread; đẩy việc sang runtime để không chặn event loop.
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match action {
            Action::SelectRegion => actions::start_selection(&app),
            Action::Capture => actions::capture(app).await,
        }
    });
}

/// Huỷ hết rồi đăng ký lại theo config hiện tại.
pub fn apply<R: Runtime>(app: &AppHandle<R>) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let hk = app.state::<AppState>().lock().config.hotkeys.clone();

    let mut errors = BTreeMap::new();
    for (name, accel, action) in [
        ("selectRegion", hk.select_region, Action::SelectRegion),
        ("capture", hk.capture, Action::Capture),
    ] {
        let accel = accel.trim().to_string();
        if accel.is_empty() {
            continue;
        }
        // Lỗi gốc của crate là chuỗi debug tiếng Anh (vd. "HotKey already registered: HotKey { .. }"):
        // chỉ ghi log, người dùng chỉ thấy câu tiếng Việt.
        let shortcut: Shortcut = match accel.parse() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("parse hotkey {name}={accel:?}: {e}");
                errors.insert(name.to_string(), invalid_message(&accel));
                continue;
            }
        };
        let res = gs.on_shortcut(shortcut, move |app, _sc, ev| {
            if ev.state == ShortcutState::Pressed {
                dispatch(app, action);
            }
        });
        if let Err(e) = res {
            log::warn!("register hotkey {name}={accel}: {e}");
            errors.insert(name.to_string(), taken_message(&accel));
        }
    }

    app.state::<AppState>().lock().hotkey_errors = errors;
    tray::refresh_tooltip(app);
}

/// Trên Windows, RegisterHotKey thất bại gần như luôn vì tổ hợp đã có chủ (app khác hoặc chính Windows).
fn taken_message(accel: &str) -> String {
    format!("Không dùng được {accel} vì app khác (hoặc Windows) đang dùng tổ hợp này. Hãy chọn tổ hợp khác.")
}

fn invalid_message(accel: &str) -> String {
    format!("Phím tắt \"{accel}\" không hợp lệ. Hãy bấm vào ô và nhấn lại tổ hợp phím.")
}

/// Tạm tắt khi màn hình Cài đặt đang ghi phím, để tổ hợp cũ không kích hoạt.
pub fn pause<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.global_shortcut().unregister_all();
}
