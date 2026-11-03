//! Icon khay hệ thống (SPEC 5.5).

use std::cell::Cell;

use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use super::{app::Action, win::emit};

const FREEZE_TOGGLE: &str = "Phím tắt chụp đóng băng";

/// Mục menu có phím tắt: (mục, tên gốc). Phím tắt hiện ở cột phải của menu.
struct Shortcut {
    item: MenuItem,
    text: &'static str,
}

pub struct Tray {
    icon: TrayIcon,
    /// Chọn vùng, Chụp lại vùng cuối, Chụp đóng băng.
    shortcuts: [Shortcut; 3],
    /// Tuỳ chọn "Phím tắt chụp đóng băng" (bật/tắt hotkey Freeze).
    freeze_toggle: CheckMenuItem,
    normal: Option<Icon>,
    waiting: Option<Icon>,
    hotkey_errors: Cell<usize>,
    /// Đang đếm ngược hẹn giờ Freeze: giữ tooltip "sắp chụp…" kể cả khi hotkey được đăng ký lại.
    waiting_on: Cell<bool>,
}

/// Icon thường và icon mờ (đang đếm ngược hẹn giờ Freeze).
fn app_icons() -> Option<(Icon, Icon)> {
    let png = include_bytes!("../../../icons/32x32.png");
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let raw = img.into_raw();
    let dim: Vec<u8> = raw.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2], p[3] / 3]).collect();
    Some((Icon::from_rgba(raw, w, h).ok()?, Icon::from_rgba(dim, w, h).ok()?))
}

impl Tray {
    pub fn create() -> Result<Tray, String> {
        let menu = Menu::new();
        let shortcuts = [("select", "Chọn vùng"), ("recapture", "Chụp lại vùng cuối"), ("freeze", "Chụp đóng băng")]
            .map(|(id, text)| Shortcut { item: MenuItem::with_id(id, text, true, None), text });
        let freeze_toggle = CheckMenuItem::with_id("freeze_toggle", FREEZE_TOGGLE, true, true, None);
        // Hai phần: thao tác (bấm là làm ngay) và tuỳ chọn (bật/tắt, mở Cài đặt). Tiêu đề phần là mục mờ không bấm được.
        let items: [&dyn tray_icon::menu::IsMenuItem; 12] = [
            &MenuItem::new("Chụp", false, None),
            &shortcuts[0].item,
            &shortcuts[1].item,
            &shortcuts[2].item,
            &MenuItem::with_id("freeze_delayed", "Chụp đóng băng sau 3 giây", true, None),
            &MenuItem::with_id("open_folder", "Mở folder ảnh", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::new("Tuỳ chọn", false, None),
            &freeze_toggle,
            &MenuItem::with_id("settings", "Cài đặt…", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("quit", "Thoát", true, None),
        ];
        menu.append_items(&items).map_err(|e| e.to_string())?;

        MenuEvent::set_event_handler(Some(|e: MenuEvent| emit(Action::Menu(e.id.0.clone()))));
        TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                emit(Action::StartSelection);
            }
        }));

        let mut b = TrayIconBuilder::new()
            .with_tooltip("Chup")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false);
        let (normal, waiting) = app_icons().unzip();
        if let Some(i) = normal.clone() {
            b = b.with_icon(i);
        }
        let icon = b.build().map_err(|e| format!("Không tạo được icon khay: {e}"))?;
        Ok(Tray {
            icon,
            shortcuts,
            freeze_toggle,
            normal,
            waiting,
            hotkey_errors: Cell::new(0),
            waiting_on: Cell::new(false),
        })
    }

    /// Hiện phím tắt cạnh mục menu, theo thứ tự Chọn vùng, Chụp (cũng là "Chụp lại vùng cuối"), Chụp đóng băng.
    /// Chuỗi rỗng = không hiện (chưa gán, hoặc đăng ký lỗi nên bấm cũng không chạy).
    pub fn set_shortcuts(&self, keys: [&str; 3]) {
        for (s, k) in self.shortcuts.iter().zip(keys) {
            // Windows vẽ phần sau tab ở cột phải của menu, như phím tắt chuẩn.
            let text = if k.is_empty() { s.text.to_string() } else { format!("{}\t{k}", s.text) };
            s.item.set_text(text);
        }
    }

    /// Trạng thái bật/tắt hotkey Freeze, kèm phím đã gán (vd. "Phím tắt chụp đóng băng (F1)").
    pub fn set_freeze_toggle(&self, enabled: bool, key: &str) {
        let key = key.trim();
        let text = if key.is_empty() { FREEZE_TOGGLE.to_string() } else { format!("{FREEZE_TOGGLE} ({key})") };
        self.freeze_toggle.set_text(text);
        self.freeze_toggle.set_checked(enabled && !key.is_empty());
        // Chưa gán phím thì bật/tắt vô nghĩa: gán trong Cài đặt.
        self.freeze_toggle.set_enabled(!key.is_empty());
    }

    /// Đang đếm ngược "Chụp đóng băng sau 3 giây": đổi icon và tooltip, không hiện gì lên màn hình (SPEC 14.4).
    pub fn set_waiting(&self, on: bool) {
        self.waiting_on.set(on);
        let _ = self.icon.set_icon(if on { self.waiting.clone() } else { self.normal.clone() });
        if on {
            let _ = self.icon.set_tooltip(Some("Chup — sắp chụp đóng băng…"));
        } else {
            self.set_hotkey_errors(self.hotkey_errors.get());
        }
    }

    /// "Chup", kèm cảnh báo khi có phím tắt đăng ký lỗi.
    pub fn set_hotkey_errors(&self, n: usize) {
        self.hotkey_errors.set(n);
        if self.waiting_on.get() {
            return;
        }
        let text = if n == 0 { "Chup".to_string() } else { format!("Chup — ⚠ {n} phím tắt lỗi, xem Cài đặt") };
        let _ = self.icon.set_tooltip(Some(text));
    }
}
