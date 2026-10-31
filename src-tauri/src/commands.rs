//! IPC contract (SPEC 6.7). Mọi command đều async để không chạy trên main thread
//! (command sync tạo/điều khiển cửa sổ trên Windows có thể deadlock).

use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::{
    actions::{self, FrameStateDto, StackStateDto},
    capture,
    config::{self, Config},
    hotkeys,
    state::AppState,
    wm::SETTINGS,
};

type CmdResult<T> = Result<T, String>;

// ---------- overlay ----------

#[tauri::command]
pub async fn region_selected<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> CmdResult<()> {
    actions::region_selected(&app, window.label(), x, y, w, h);
    Ok(())
}

#[tauri::command]
pub async fn selection_cancelled<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    actions::cancel_selection(&app);
    Ok(())
}

// ---------- frame / toolbar ----------

#[tauri::command]
pub async fn frame_state<R: Runtime>(app: AppHandle<R>) -> CmdResult<FrameStateDto> {
    Ok(actions::frame_state(&app))
}

#[tauri::command]
pub async fn capture_now<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    actions::capture(app).await;
    Ok(())
}

#[tauri::command]
pub async fn toggle_fade<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    actions::toggle_fade(&app);
    Ok(())
}

#[tauri::command]
pub async fn close_region<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    actions::close_region(&app);
    Ok(())
}

// ---------- stack ----------

#[tauri::command]
pub async fn stack_state<R: Runtime>(app: AppHandle<R>) -> CmdResult<StackStateDto> {
    Ok(actions::stack_state(&app))
}

#[tauri::command]
pub async fn stack_report_size<R: Runtime>(app: AppHandle<R>, width: f64, height: f64) -> CmdResult<()> {
    actions::stack_report_size(&app, width, height);
    Ok(())
}

#[tauri::command]
pub async fn stack_remove<R: Runtime>(app: AppHandle<R>, id: String) -> CmdResult<()> {
    actions::stack_remove(&app, &id);
    Ok(())
}

#[tauri::command]
pub async fn stack_clear<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    actions::stack_clear(&app);
    Ok(())
}

/// Trước khi kéo: file còn không? Mất thì shot tự bị bỏ khỏi stack.
#[tauri::command]
pub async fn shot_exists<R: Runtime>(app: AppHandle<R>, id: String) -> CmdResult<bool> {
    Ok(actions::ensure_shot_file(&app, &id).is_some())
}

#[tauri::command]
pub async fn shot_open<R: Runtime>(app: AppHandle<R>, id: String) -> CmdResult<()> {
    let Some(p) = actions::ensure_shot_file(&app, &id) else { return Ok(()) };
    app.opener()
        .open_path(p.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Không mở được ảnh: {e}"))
}

#[tauri::command]
pub async fn shot_reveal<R: Runtime>(app: AppHandle<R>, id: String) -> CmdResult<()> {
    let Some(p) = actions::ensure_shot_file(&app, &id) else { return Ok(()) };
    app.opener()
        .reveal_item_in_dir(p)
        .map_err(|e| format!("Không mở được thư mục: {e}"))
}

#[tauri::command]
pub async fn shot_copy<R: Runtime>(app: AppHandle<R>, id: String) -> CmdResult<()> {
    let Some(p) = actions::ensure_shot_file(&app, &id) else { return Ok(()) };
    let res = tauri::async_runtime::spawn_blocking(move || capture::copy_file_image(&p))
        .await
        .map_err(|e| e.to_string())?;
    match res {
        Ok(()) => actions::toast(&app, "info", "Đã copy ảnh vào clipboard"),
        Err(e) => actions::toast(&app, "error", e),
    }
    Ok(())
}

// ---------- settings ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub config: Config,
    pub hotkey_errors: BTreeMap<String, String>,
    pub default_save_dir: PathBuf,
    pub file_name_preview: String,
    pub capture_exclusion_ok: bool,
}

fn settings_dto<R: Runtime>(app: &AppHandle<R>) -> SettingsDto {
    let state = app.state::<AppState>();
    let s = state.lock();
    SettingsDto {
        file_name_preview: format!(
            "{}.png",
            capture::render_file_name(&s.config.file_name_pattern, &chrono::Local::now())
        ),
        config: s.config.clone(),
        hotkey_errors: s.hotkey_errors.clone(),
        default_save_dir: state.paths.default_save_dir.clone(),
        capture_exclusion_ok: s.capture_exclusion_ok,
    }
}

#[tauri::command]
pub async fn settings_get<R: Runtime>(app: AppHandle<R>) -> CmdResult<SettingsDto> {
    Ok(settings_dto(&app))
}

/// Áp và lưu config mới, rồi làm những việc kéo theo (hotkey, autostart, stack).
pub fn commit_config<R: Runtime>(app: &AppHandle<R>, next: Config) -> CmdResult<SettingsDto> {
    let state = app.state::<AppState>();
    let prev = state.lock().config.clone();
    config::save(&state.paths.config_file, &next)?;
    {
        let mut s = state.lock();
        s.config = next.clone();
        if prev.stack != next.stack {
            s.stack_monitor_key = None;
        }
    }
    if prev.hotkeys != next.hotkeys {
        hotkeys::apply(app);
    }
    if prev.launch_at_startup != next.launch_at_startup {
        crate::sync_autostart(app)?;
    }
    if prev.stack != next.stack {
        actions::emit_stack(app);
        actions::layout_stack(app);
    }
    let dto = settings_dto(app);
    let _ = app.emit_to(SETTINGS, "config:changed", &dto);
    Ok(dto)
}

#[tauri::command]
pub async fn settings_update<R: Runtime>(app: AppHandle<R>, patch: serde_json::Value) -> CmdResult<SettingsDto> {
    let cur = app.state::<AppState>().lock().config.clone();
    let next = config::apply_patch(&cur, &patch)?;
    commit_config(&app, next)
}

#[tauri::command]
pub async fn pick_save_dir<R: Runtime>(app: AppHandle<R>) -> CmdResult<SettingsDto> {
    let current = app.state::<AppState>().lock().config.save_dir.clone();
    let parent = app.get_webview_window(SETTINGS);
    let dialog_app = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut d = dialog_app.dialog().file().set_title("Chọn folder lưu ảnh");
        if current.is_dir() {
            d = d.set_directory(&current);
        }
        if let Some(p) = &parent {
            d = d.set_parent(p);
        }
        d.blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(fp) = picked else { return Ok(settings_dto(&app)) };
    let dir = fp.into_path().map_err(|e| format!("Đường dẫn không hợp lệ: {e}"))?;
    let mut next = app.state::<AppState>().lock().config.clone();
    next.save_dir = dir;
    commit_config(&app, next)
}

pub fn open_save_dir_impl<R: Runtime>(app: &AppHandle<R>) -> CmdResult<()> {
    let dir = app.state::<AppState>().lock().config.save_dir.clone();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Không tạo được {}: {e}", dir.display()))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Không mở được thư mục: {e}"))
}

#[tauri::command]
pub async fn open_save_dir<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    open_save_dir_impl(&app)
}

#[tauri::command]
pub async fn hotkeys_pause<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    hotkeys::pause(&app);
    Ok(())
}

#[tauri::command]
pub async fn hotkeys_resume<R: Runtime>(app: AppHandle<R>) -> CmdResult<SettingsDto> {
    hotkeys::apply(&app);
    Ok(settings_dto(&app))
}
