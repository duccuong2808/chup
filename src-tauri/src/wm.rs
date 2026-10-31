//! Tạo và đặt vị trí cửa sổ (SPEC 6.2). Mọi vị trí/kích thước dùng physical px.

use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

use crate::{
    actions,
    coords::{MonitorInfo, Rect},
    state::AppState,
};

pub const FRAME: &str = "frame";
pub const TOOLBAR: &str = "toolbar";
pub const STACK: &str = "stack";
pub const SETTINGS: &str = "settings";
pub const OVERLAY_PREFIX: &str = "overlay-";

/// Bề rộng cố định của cửa sổ stack khi mở rộng (CSS px). Frontend dùng cùng giá trị.
pub const STACK_WIDTH_CSS: f64 = 184.0;
pub const TOOLBAR_NORMAL_CSS: (f64, f64) = (220.0, 44.0);
pub const TOOLBAR_FADED_CSS: (f64, f64) = (116.0, 40.0);

/// Tham số cho WebView2, dùng chung cho **mọi** cửa sổ: WebView2 từ chối tạo webview nếu tham số khác
/// với webview đã có trong cùng data directory.
/// - Giữ nguyên 3 feature mà wry tắt mặc định (set tham số là ghi đè mặc định của wry).
/// - `--renderer-process-limit=1`: các cửa sổ cùng origin dùng chung renderer thay vì mỗi cửa sổ một
///   process. Đo trên Windows 11 lúc vừa khởi động: 125,6 MB -> 101,3 MB (private working set).
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --renderer-process-limit=1";

fn page(name: &str) -> WebviewUrl {
    WebviewUrl::App(format!("windows/{name}/index.html").into())
}

/// Cửa sổ phụ: không viền, trong suốt, luôn trên cùng, không lên taskbar, tạo ở trạng thái ẩn.
fn tool_window<'a, R: Runtime>(app: &'a AppHandle<R>, label: &'a str, page_name: &str) -> WebviewWindowBuilder<'a, R, AppHandle<R>> {
    WebviewWindowBuilder::new(app, label, page(page_name))
        .title("Chup")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .focused(false)
        .additional_browser_args(BROWSER_ARGS)
}

pub fn read_monitors<R: Runtime>(app: &AppHandle<R>) -> Vec<MonitorInfo> {
    let mons = app.available_monitors().unwrap_or_default();
    mons.iter()
        .map(|m| {
            let pos = m.position();
            let size = m.size();
            let wa = m.work_area();
            MonitorInfo {
                key: format!("{}@{},{}", m.name().map(String::as_str).unwrap_or("?"), pos.x, pos.y),
                rect: Rect::new(pos.x, pos.y, size.width, size.height),
                work: Rect::new(wa.position.x, wa.position.y, wa.size.width, wa.size.height),
                scale: m.scale_factor(),
            }
        })
        .collect()
}

pub fn primary_monitor_key<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let m = app.primary_monitor().ok()??;
    let pos = m.position();
    Some(format!("{}@{},{}", m.name().map(String::as_str).unwrap_or("?"), pos.x, pos.y))
}

/// Đặt cửa sổ đúng rect physical. Gọi vị trí -> kích thước -> vị trí: khi cửa sổ sang
/// màn hình khác DPI, Windows tự đổi kích thước theo WM_DPICHANGED; lần set sau sửa lại.
pub fn place<R: Runtime>(w: &WebviewWindow<R>, r: Rect) {
    let pos = PhysicalPosition::new(r.x, r.y);
    let _ = w.set_position(pos);
    let _ = w.set_size(PhysicalSize::new(r.w.max(1), r.h.max(1)));
    let _ = w.set_position(pos);
}

pub fn create_static_windows<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // Frame: cần nhận chuột để kéo/resize ở Normal mode, nên focusable.
    let frame = tool_window(app, FRAME, "frame").resizable(true).build()?;
    let handle = app.clone();
    frame.on_window_event(move |ev| match ev {
        WindowEvent::Moved(p) => actions::frame_moved(&handle, p.x, p.y),
        WindowEvent::Resized(s) => actions::frame_resized(&handle, s.width, s.height),
        _ => {}
    });

    // Toolbar và stack: không bao giờ lấy focus (tao gắn WS_EX_NOACTIVATE).
    tool_window(app, TOOLBAR, "toolbar").focusable(false).inner_size(220.0, 44.0).build()?;
    tool_window(app, STACK, "stack")
        .focusable(false)
        .inner_size(STACK_WIDTH_CSS, 120.0)
        .build()?;
    Ok(())
}

/// Tạo lại overlay khi danh sách màn hình đổi (SPEC 7). Không làm gì nếu không đổi.
pub fn sync_overlays<R: Runtime>(app: &AppHandle<R>) {
    let mons = read_monitors(app);
    let st = app.state::<AppState>();
    let same = {
        let s = st.lock();
        s.overlays.len() == mons.len() && s.overlays.values().zip(mons.iter()).all(|(a, b)| a == b)
    };
    if same {
        return;
    }

    for (label, w) in app.webview_windows() {
        if label.starts_with(OVERLAY_PREFIX) {
            let _ = w.destroy();
        }
    }

    let mut overlays = std::collections::BTreeMap::new();
    for (i, m) in mons.iter().enumerate() {
        let label = format!("{OVERLAY_PREFIX}{i}");
        match tool_window(app, &label, "overlay").build() {
            Ok(w) => {
                place(&w, m.rect);
                overlays.insert(label, m.clone());
            }
            Err(e) => log::error!("Không tạo được overlay cho {}: {e}", m.key),
        }
    }
    let mut s = st.lock();
    s.overlays = overlays;
    s.monitors = mons;
}

pub fn open_settings<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, SETTINGS, page("settings"))
        .title("Chup — Cài đặt")
        .inner_size(560.0, 680.0)
        .min_inner_size(480.0, 520.0)
        .center()
        .focused(true)
        .additional_browser_args(BROWSER_ARGS)
        .build();
    if let Err(e) = built {
        log::error!("Không mở được cửa sổ cài đặt: {e}");
    }
}
