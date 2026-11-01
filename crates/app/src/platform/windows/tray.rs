//! Icon khay hệ thống (SPEC 5.5).

use std::cell::Cell;

use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use super::{app::Action, win::emit};

pub struct Tray {
    icon: TrayIcon,
    normal: Option<Icon>,
    waiting: Option<Icon>,
    hotkey_errors: Cell<usize>,
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
        let items: [&dyn tray_icon::menu::IsMenuItem; 9] = [
            &MenuItem::with_id("select", "Chọn vùng", true, None),
            &MenuItem::with_id("recapture", "Chụp lại vùng cuối", true, None),
            &MenuItem::with_id("freeze", "Chụp đóng băng", true, None),
            &MenuItem::with_id("freeze_delayed", "Chụp đóng băng sau 3 giây", true, None),
            &MenuItem::with_id("open_folder", "Mở folder ảnh", true, None),
            &PredefinedMenuItem::separator(),
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
        Ok(Tray { icon, normal, waiting, hotkey_errors: Cell::new(0) })
    }

    /// Đang đếm ngược "Chụp đóng băng sau 3 giây": đổi icon và tooltip, không hiện gì lên màn hình (SPEC 14.4).
    pub fn set_waiting(&self, on: bool) {
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
        let text = if n == 0 { "Chup".to_string() } else { format!("Chup — ⚠ {n} phím tắt lỗi, xem Cài đặt") };
        let _ = self.icon.set_tooltip(Some(text));
    }
}
