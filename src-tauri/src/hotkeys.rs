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
        let shortcut: Shortcut = match accel.parse() {
            Ok(s) => s,
            Err(e) => {
                errors.insert(name.to_string(), format!("Phím tắt \"{accel}\" không hợp lệ: {e}"));
                continue;
            }
        };
        let res = gs.on_shortcut(shortcut, move |app, _sc, ev| {
            if ev.state == ShortcutState::Pressed {
                dispatch(app, action);
            }
        });
        if let Err(e) = res {
            errors.insert(
                name.to_string(),
                format!("Không đăng ký được {accel}, có thể app khác đang dùng ({e})"),
            );
        }
    }

    app.state::<AppState>().lock().hotkey_errors = errors;
    tray::refresh_tooltip(app);
}

/// Tạm tắt khi màn hình Cài đặt đang ghi phím, để tổ hợp cũ không kích hoạt.
pub fn pause<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.global_shortcut().unregister_all();
}
