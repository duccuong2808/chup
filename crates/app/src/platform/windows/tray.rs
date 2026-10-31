//! Icon khay hệ thống (SPEC 5.5).

use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use super::{app::Action, win::emit};

pub struct Tray {
    icon: TrayIcon,
}

fn app_icon() -> Option<Icon> {
    let png = include_bytes!("../../../icons/32x32.png");
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).ok()
}

impl Tray {
    pub fn create() -> Result<Tray, String> {
        let menu = Menu::new();
        let items: [&dyn tray_icon::menu::IsMenuItem; 7] = [
            &MenuItem::with_id("select", "Chọn vùng", true, None),
            &MenuItem::with_id("recapture", "Chụp lại vùng cuối", true, None),
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
        if let Some(i) = app_icon() {
            b = b.with_icon(i);
        }
        let icon = b.build().map_err(|e| format!("Không tạo được icon khay: {e}"))?;
        Ok(Tray { icon })
    }

    /// "Chup", kèm cảnh báo khi có phím tắt đăng ký lỗi.
    pub fn set_hotkey_errors(&self, n: usize) {
        let text = if n == 0 { "Chup".to_string() } else { format!("Chup — ⚠ {n} phím tắt lỗi, xem Cài đặt") };
        let _ = self.icon.set_tooltip(Some(text));
    }
}
