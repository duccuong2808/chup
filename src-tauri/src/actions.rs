//! Các luồng chính (SPEC 5, 6.8). Được gọi từ command, hotkey, tray và watcher.
//! Không giữ khoá state qua `.await` hay khi gọi API cửa sổ.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalSize, Runtime};

use crate::{
    capture::{self, PersistOptions, Shot},
    config::{AfterCapture, Edge, StackMonitor},
    coords::{self, css_to_physical, frame_margin, monitor_at, MonitorInfo, Rect},
    platform::PlatformExt,
    stack,
    state::{AppState, FrameMode, Region},
    wm::{self, FRAME, OVERLAY_PREFIX, STACK, TOOLBAR, TOOLBAR_FADED_CSS, TOOLBAR_NORMAL_CSS},
};

// ---------- DTO gửi xuống frontend ----------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStateDto {
    pub mode: FrameMode,
    pub margin_css: f64,
    pub capture_hotkey: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackStateDto {
    pub shots: Vec<Shot>,
    pub edge: Edge,
}

#[derive(Clone, Serialize)]
pub struct ToastDto {
    pub kind: &'static str,
    pub message: String,
}

fn st<R: Runtime>(app: &AppHandle<R>) -> &AppState {
    app.state::<AppState>().inner()
}

pub fn toast<R: Runtime>(app: &AppHandle<R>, kind: &'static str, message: impl Into<String>) {
    let _ = app.emit_to(STACK, "toast", ToastDto { kind, message: message.into() });
}

// ---------- Chọn vùng (SPEC 5.1) ----------

pub fn start_selection<R: Runtime>(app: &AppHandle<R>) {
    let (selecting, mode) = {
        let s = st(app).lock();
        (s.selecting, s.mode)
    };
    if selecting {
        // Đang chọn thì bỏ qua (SPEC 7). Nhưng nếu overlay không còn hiện (webview lỗi,
        // bị đóng ngoài ý muốn) thì gỡ trạng thái kẹt và cho chọn lại.
        let visible = app
            .webview_windows()
            .into_iter()
            .any(|(l, w)| l.starts_with(OVERLAY_PREFIX) && w.is_visible().unwrap_or(false));
        if visible {
            return;
        }
        st(app).lock().selecting = false;
    }
    if mode != FrameMode::Hidden {
        close_region(app);
    }
    wm::sync_overlays(app);
    let overlays = {
        let mut s = st(app).lock();
        s.selecting = true;
        s.overlays.clone()
    };

    for (label, mon) in &overlays {
        if let Some(w) = app.get_webview_window(label) {
            wm::place(&w, mon.rect);
            let _ = app.emit_to(label.as_str(), "selection:start", serde_json::json!({ "mode": "live" }));
            let _ = w.show();
        }
    }

    // Focus overlay dưới con trỏ để nhận phím Esc.
    let cursor = app.cursor_position().ok();
    let target = cursor
        .and_then(|c| overlays.iter().find(|(_, m)| m.rect.contains_point(c.x as i32, c.y as i32)))
        .or_else(|| overlays.iter().next())
        .map(|(l, _)| l.clone());
    if let Some(w) = target.and_then(|l| app.get_webview_window(&l)) {
        let _ = w.set_focus();
    }
}

pub fn cancel_selection<R: Runtime>(app: &AppHandle<R>) {
    st(app).lock().selecting = false;
    hide_overlays(app);
}

fn hide_overlays<R: Runtime>(app: &AppHandle<R>) {
    for (label, w) in app.webview_windows() {
        if label.starts_with(OVERLAY_PREFIX) {
            let _ = w.hide();
        }
    }
}

/// Overlay `label` báo vùng đã chọn, đơn vị CSS px tương đối cửa sổ.
pub fn region_selected<R: Runtime>(app: &AppHandle<R>, label: &str, x: f64, y: f64, w: f64, h: f64) {
    let mon = {
        let mut s = st(app).lock();
        s.selecting = false;
        s.overlays.get(label).cloned()
    };
    hide_overlays(app);
    let Some(mon) = mon else { return };
    match css_to_physical(&mon, x, y, w, h) {
        Some(r) if r.w >= 4 && r.h >= 4 => open_region(app, r, &mon),
        _ => {}
    }
}

fn frame_state_of<R: Runtime>(app: &AppHandle<R>) -> FrameStateDto {
    let s = st(app).lock();
    let scale = s
        .region
        .as_ref()
        .and_then(|r| {
            let (cx, cy) = r.rect.center();
            monitor_at(&s.monitors, cx, cy)
        })
        .map(|m| m.scale)
        .unwrap_or(1.0);
    FrameStateDto {
        mode: s.mode,
        margin_css: s.frame_margin as f64 / scale,
        capture_hotkey: s.config.hotkeys.capture.clone(),
    }
}

pub fn frame_state<R: Runtime>(app: &AppHandle<R>) -> FrameStateDto {
    frame_state_of(app)
}

fn emit_frame_state<R: Runtime>(app: &AppHandle<R>) {
    let dto = frame_state_of(app);
    let _ = app.emit_to(FRAME, "frame:state", dto.clone());
    let _ = app.emit_to(TOOLBAR, "frame:state", dto);
}

fn open_region<R: Runtime>(app: &AppHandle<R>, rect: Rect, mon: &MonitorInfo) {
    let m = frame_margin(mon.scale);
    let outer = rect.expand(m);
    {
        let mut s = st(app).lock();
        s.region = Some(Region { rect });
        s.last_region = Some(Region { rect });
        s.mode = FrameMode::Normal;
        s.frame_margin = m;
        s.frame_pos = (outer.x, outer.y);
        s.frame_size = (outer.w, outer.h);
    }
    if let Some(f) = app.get_webview_window(FRAME) {
        let _ = f.set_ignore_cursor_events(false);
        let _ = f.set_resizable(true);
        let min = (2 * m + 8) as u32;
        let _ = f.set_min_size(Some(PhysicalSize::new(min, min)));
        wm::place(&f, outer);
        emit_frame_state(app);
        let _ = f.show();
        let _ = f.set_focus();
    }
    layout_toolbar(app);
    if let Some(t) = app.get_webview_window(TOOLBAR) {
        t.show_inactive();
    }
}

pub fn close_region<R: Runtime>(app: &AppHandle<R>) {
    {
        let mut s = st(app).lock();
        s.region = None;
        s.mode = FrameMode::Hidden;
    }
    if let Some(f) = app.get_webview_window(FRAME) {
        let _ = f.hide();
        let _ = f.set_ignore_cursor_events(false);
    }
    if let Some(t) = app.get_webview_window(TOOLBAR) {
        t.hide_inactive();
    }
    emit_frame_state(app);
}

pub fn toggle_fade<R: Runtime>(app: &AppHandle<R>) {
    let next = {
        let mut s = st(app).lock();
        let next = match s.mode {
            FrameMode::Normal => FrameMode::Faded,
            FrameMode::Faded => FrameMode::Normal,
            FrameMode::Hidden => return,
        };
        s.mode = next;
        next
    };
    if let Some(f) = app.get_webview_window(FRAME) {
        let faded = next == FrameMode::Faded;
        let _ = f.set_ignore_cursor_events(faded);
        // Tauri gắn 1 cửa sổ con làm viền resize cho cửa sổ không viền; tháo nó ở Faded
        // để chắc chắn không còn chỗ nào chặn chuột.
        let _ = f.set_resizable(!faded);
    }
    emit_frame_state(app);
    layout_toolbar(app);
}

/// Đặt toolbar cạnh region theo mode hiện tại (SPEC 5.2).
pub fn layout_toolbar<R: Runtime>(app: &AppHandle<R>) {
    let (rect, mode, margin, monitors) = {
        let s = st(app).lock();
        let Some(r) = s.region.as_ref() else { return };
        (r.rect, s.mode, s.frame_margin, s.monitors.clone())
    };
    let (cx, cy) = rect.center();
    let Some(mon) = monitor_at(&monitors, cx, cy).or(monitors.first()) else { return };
    let css = if mode == FrameMode::Faded { TOOLBAR_FADED_CSS } else { TOOLBAR_NORMAL_CSS };
    let size = ((css.0 * mon.scale).ceil() as u32, (css.1 * mon.scale).ceil() as u32);
    let gap = margin + (6.0 * mon.scale).round() as i32;
    let (x, y) = coords::place_toolbar(&rect, size, &mon.work, gap);
    if let Some(t) = app.get_webview_window(TOOLBAR) {
        wm::place(&t, Rect::new(x, y, size.0, size.1));
    }
}

// ---------- Frame bị kéo/resize bằng chuột (SPEC 5.3) ----------

pub fn frame_moved<R: Runtime>(app: &AppHandle<R>, x: i32, y: i32) {
    if update_region_from_frame(app, |s| s.frame_pos = (x, y)) {
        layout_toolbar(app);
    }
}

pub fn frame_resized<R: Runtime>(app: &AppHandle<R>, w: u32, h: u32) {
    if update_region_from_frame(app, |s| s.frame_size = (w, h)) {
        layout_toolbar(app);
    }
}

fn update_region_from_frame<R: Runtime>(app: &AppHandle<R>, f: impl FnOnce(&mut crate::state::Inner)) -> bool {
    let mut s = st(app).lock();
    if s.mode == FrameMode::Hidden {
        return false;
    }
    f(&mut s);
    let outer = Rect::new(s.frame_pos.0, s.frame_pos.1, s.frame_size.0, s.frame_size.1);
    let inner = outer.expand(-s.frame_margin);
    if inner.w == 0 || inner.h == 0 {
        return false;
    }
    s.region = Some(Region { rect: inner });
    s.last_region = Some(Region { rect: inner });
    true
}

// ---------- Chụp (SPEC 6.8) ----------

pub async fn capture<R: Runtime>(app: AppHandle<R>) {
    let (rect, mode, hide_ui, after) = {
        let s = st(&app).lock();
        let Some(r) = s.region.clone().or_else(|| s.last_region.clone()) else { return };
        (
            r.rect,
            s.mode,
            s.config.hide_ui_during_capture || !s.capture_exclusion_ok,
            s.config.after_capture_normal,
        )
    };

    // Màn hình chứa vùng còn tồn tại không?
    let mons = wm::read_monitors(&app);
    let (cx, cy) = rect.center();
    if monitor_at(&mons, cx, cy).is_none() {
        close_region(&app);
        st(&app).lock().last_region = None;
        toast(&app, "error", "Màn hình chứa vùng chụp không còn. Hãy chọn vùng mới.");
        return;
    }

    // Fallback SPEC 6.4: ẩn toolbar/stack nếu chúng có thể lọt vào ảnh.
    // Frame không cần ẩn: viền nằm ngoài region, phần trong region trong suốt.
    let hidden = if hide_ui { hide_ui_for_capture(&app) } else { vec![] };
    let wait = !hidden.is_empty();
    let grabbed = tauri::async_runtime::spawn_blocking(move || {
        if wait {
            std::thread::sleep(Duration::from_millis(80));
        }
        capture::grab_region(rect)
    })
    .await;
    for label in hidden {
        if let Some(w) = app.get_webview_window(label) {
            w.show_inactive();
        }
    }

    let img = match grabbed {
        Ok(Ok(img)) => img,
        Ok(Err(e)) => return toast(&app, "error", e),
        Err(e) => return toast(&app, "error", format!("Chụp thất bại: {e}")),
    };

    if mode == FrameMode::Normal && after == AfterCapture::Close {
        close_region(&app);
    }

    let opts = persist_options(&app);
    let persisted = tauri::async_runtime::spawn_blocking(move || capture::persist_shot(&img, &opts)).await;
    match persisted {
        Ok(Ok(out)) => {
            let evicted = st(&app).lock().stack.push(out.shot);
            stack::drop_thumbnails(&evicted);
            emit_stack(&app);
            for w in out.warnings {
                toast(&app, "error", w);
            }
        }
        Ok(Err(e)) => toast(&app, "error", e),
        Err(e) => toast(&app, "error", format!("Lưu ảnh thất bại: {e}")),
    }
}

fn hide_ui_for_capture<R: Runtime>(app: &AppHandle<R>) -> Vec<&'static str> {
    let (toolbar, stack) = {
        let s = st(app).lock();
        (s.mode != FrameMode::Hidden, s.stack_visible)
    };
    let mut hidden = vec![];
    for (label, visible) in [(TOOLBAR, toolbar), (STACK, stack)] {
        if let (true, Some(w)) = (visible, app.get_webview_window(label)) {
            w.hide_inactive();
            hidden.push(label);
        }
    }
    hidden
}

fn persist_options<R: Runtime>(app: &AppHandle<R>) -> PersistOptions {
    let state = st(app);
    let s = state.lock();
    PersistOptions {
        save_dir: s.config.save_dir.clone(),
        fallback_dir: state.paths.default_save_dir.clone(),
        pattern: s.config.file_name_pattern.clone(),
        copy_to_clipboard: s.config.copy_to_clipboard,
        thumbs_dir: state.paths.thumbs_dir.clone(),
    }
}

// ---------- Stack (SPEC 5.4) ----------

pub fn stack_state<R: Runtime>(app: &AppHandle<R>) -> StackStateDto {
    let s = st(app).lock();
    StackStateDto {
        shots: s.stack.list(),
        edge: s.config.stack.edge,
    }
}

pub fn emit_stack<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit_to(STACK, "stack:changed", stack_state(app));
}

/// Frontend báo kích thước nội dung (CSS px). 0 chiều cao = ẩn cửa sổ.
pub fn stack_report_size<R: Runtime>(app: &AppHandle<R>, width: f64, height: f64) {
    st(app).lock().stack_css = (width, height);
    layout_stack(app);
}

pub fn layout_stack<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window(STACK) else { return };
    let (css, visible, edge, which, key, monitors) = {
        let s = st(app).lock();
        (
            s.stack_css,
            s.stack_visible,
            s.config.stack.edge,
            s.config.stack.monitor,
            s.stack_monitor_key.clone(),
            s.monitors.clone(),
        )
    };

    if css.1 < 1.0 || css.0 < 1.0 {
        if visible {
            win.hide_inactive();
            let mut s = st(app).lock();
            s.stack_visible = false;
            s.stack_monitor_key = None;
        }
        return;
    }

    // Chốt màn hình lúc stack bắt đầu hiện, để stack không nhảy theo chuột.
    let key = key.filter(|k| monitors.iter().any(|m| &m.key == k)).or_else(|| match which {
        StackMonitor::Primary => wm::primary_monitor_key(app),
        StackMonitor::Cursor => app
            .cursor_position()
            .ok()
            .and_then(|c| monitor_at(&monitors, c.x as i32, c.y as i32))
            .map(|m| m.key.clone()),
    });
    let Some(mon) = key
        .as_ref()
        .and_then(|k| monitors.iter().find(|m| &m.key == k))
        .or(monitors.first())
        .cloned()
    else {
        return;
    };

    let size = ((css.0 * mon.scale).ceil() as u32, (css.1 * mon.scale).ceil() as u32);
    let margin = (12.0 * mon.scale).round() as i32;
    let (x, y) = coords::place_stack(size, &mon.work, edge == Edge::Right, margin);
    wm::place(&win, Rect::new(x, y, size.0, size.1));
    if !visible {
        win.show_inactive();
    }
    let mut s = st(app).lock();
    s.stack_visible = true;
    s.stack_monitor_key = Some(mon.key);
}

pub fn stack_remove<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let removed = st(app).lock().stack.remove(id);
    if let Some(s) = removed {
        stack::drop_thumbnails(&[s]);
    }
    emit_stack(app);
}

pub fn stack_clear<R: Runtime>(app: &AppHandle<R>) {
    let removed = st(app).lock().stack.clear();
    stack::drop_thumbnails(&removed);
    emit_stack(app);
}

pub fn shot_path<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<std::path::PathBuf> {
    st(app).lock().stack.get(id).map(|s| s.path.clone())
}

/// Kiểm tra file của shot còn không; mất thì bỏ khỏi stack (SPEC 7).
pub fn ensure_shot_file<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<std::path::PathBuf> {
    let path = shot_path(app, id)?;
    if path.is_file() {
        return Some(path);
    }
    stack_remove(app, id);
    toast(app, "error", format!("File {} không còn, đã bỏ khỏi stack", path.display()));
    None
}

// ---------- Màn hình thay đổi (SPEC 7) ----------

pub fn on_monitors_changed<R: Runtime>(app: &AppHandle<R>, mons: Vec<MonitorInfo>) {
    let (selecting, region) = {
        let mut s = st(app).lock();
        s.monitors = mons.clone();
        s.stack_monitor_key = None;
        (s.selecting, s.region.clone())
    };
    if selecting {
        cancel_selection(app);
    }
    wm::sync_overlays(app);
    if let Some(r) = region {
        let (cx, cy) = r.rect.center();
        if monitor_at(&mons, cx, cy).is_none() {
            close_region(app);
            st(app).lock().last_region = None;
            toast(app, "info", "Màn hình chứa vùng chọn đã bị rút, vùng chọn đã đóng.");
        } else {
            layout_toolbar(app);
        }
    }
    layout_stack(app);
}

pub fn spawn_monitor_watcher<R: Runtime>(app: AppHandle<R>) {
    std::thread::Builder::new()
        .name("chup-monitor-watch".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            let mons = wm::read_monitors(&app);
            if mons.is_empty() {
                continue;
            }
            let changed = st(&app).lock().monitors != mons;
            if changed {
                on_monitors_changed(&app, mons);
            }
        })
        .expect("spawn monitor watcher");
}
