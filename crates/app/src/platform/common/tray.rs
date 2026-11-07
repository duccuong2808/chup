//! Icon khay hệ thống (SPEC 5.5).
//! Dùng chung cho Windows và macOS (`#[path]`): cần `super::app::Action` và `super::win::emit` của vỏ OS.

use std::cell::Cell;

use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use chup_core::i18n::{fill, t, Strings};

use super::{app::Action, win::emit};

/// Chữ của một mục, lấy theo ngôn ngữ giao diện hiện hành.
type Text = fn(&Strings) -> &'static str;

/// Mục menu có phím tắt: (mục, tên). Phím tắt hiện ở cột phải của menu.
struct Shortcut {
    item: MenuItem,
    text: Text,
    key: std::cell::RefCell<String>,
}

pub struct Tray {
    icon: TrayIcon,
    /// Chọn vùng, Chụp lại vùng cuối, Chụp đóng băng.
    shortcuts: [Shortcut; 3],
    /// Tuỳ chọn "Phím tắt chụp đóng băng" (bật/tắt hotkey Freeze).
    freeze_toggle: CheckMenuItem,
    freeze_key: std::cell::RefCell<String>,
    /// Mục chữ cố định, để gán lại khi đổi ngôn ngữ.
    plain: Vec<(MenuItem, Text)>,
    normal: Option<Icon>,
    waiting: Option<Icon>,
    hotkey_errors: Cell<usize>,
    /// Đang đếm ngược hẹn giờ Freeze: giữ tooltip "sắp chụp…" kể cả khi hotkey được đăng ký lại.
    waiting_on: Cell<bool>,
}

/// macOS: icon menu bar là template image (chỉ dùng alpha, macOS tự tô màu theo menu bar sáng/tối và lúc được
/// chọn), không dùng icon app có màu. Windows: icon app.
const TEMPLATE: bool = cfg!(target_os = "macos");

/// Icon thường và icon mờ (đang đếm ngược hẹn giờ Freeze).
fn app_icons() -> Option<(Icon, Icon)> {
    #[cfg(target_os = "macos")]
    let png = include_bytes!("../../../icons/tray-template.png");
    #[cfg(not(target_os = "macos"))]
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
        let s = t();
        let shortcut = |id: &str, text: Text| Shortcut {
            item: MenuItem::with_id(id, text(s), true, None),
            text,
            key: Default::default(),
        };
        let shortcuts = [
            shortcut("select", |s| s.tray_select),
            shortcut("recapture", |s| s.tray_recapture),
            shortcut("freeze", |s| s.tray_freeze),
        ];
        let freeze_toggle = CheckMenuItem::with_id("freeze_toggle", s.tray_freeze_toggle, true, true, None);
        let item = |id: Option<&str>, text: Text| {
            let m = match id {
                Some(id) => MenuItem::with_id(id, text(s), true, None),
                // Tiêu đề phần: mục mờ không bấm được.
                None => MenuItem::new(text(s), false, None),
            };
            (m, text)
        };
        let plain = vec![
            item(None, |s| s.tray_section_capture),
            item(Some("freeze_delayed"), |s| s.tray_freeze_delayed),
            item(Some("open_folder"), |s| s.tray_open_folder),
            item(None, |s| s.tray_section_options),
            item(Some("settings"), |s| s.tray_settings),
            item(Some("quit"), |s| s.tray_quit),
        ];
        // Hai phần: thao tác (bấm là làm ngay) và tuỳ chọn (bật/tắt, mở Cài đặt).
        let items: [&dyn tray_icon::menu::IsMenuItem; 12] = [
            &plain[0].0,
            &shortcuts[0].item,
            &shortcuts[1].item,
            &shortcuts[2].item,
            &plain[1].0,
            &plain[2].0,
            &PredefinedMenuItem::separator(),
            &plain[3].0,
            &freeze_toggle,
            &plain[4].0,
            &PredefinedMenuItem::separator(),
            &plain[5].0,
        ];
        menu.append_items(&items).map_err(|e| e.to_string())?;

        MenuEvent::set_event_handler(Some(|e: MenuEvent| emit(Action::Menu(e.id.0.clone()))));
        // Windows: click trái icon tray = chọn vùng. macOS: click là mở menu (không chọn vùng cùng lúc).
        TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                if cfg!(windows) {
                    emit(Action::StartSelection);
                }
            }
        }));

        let mut b = TrayIconBuilder::new()
            .with_tooltip("Chup")
            .with_menu(Box::new(menu))
            // Windows: bấm trái = chọn vùng, phải = menu. macOS: bấm icon trên menu bar là mở menu (thói quen của Mac).
            .with_menu_on_left_click(cfg!(target_os = "macos"));
        let (normal, waiting) = app_icons().unzip();
        if let Some(i) = normal.clone() {
            b = b.with_icon(i).with_icon_as_template(TEMPLATE);
        }
        let icon = b.build().map_err(|e| fill(t().err_tray, &[&e]))?;
        Ok(Tray {
            icon,
            shortcuts,
            freeze_toggle,
            freeze_key: Default::default(),
            plain,
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
            *s.key.borrow_mut() = k.to_string();
        }
        self.retext_shortcuts();
    }

    fn retext_shortcuts(&self) {
        for s in &self.shortcuts {
            let (name, k) = ((s.text)(t()), s.key.borrow());
            // Windows vẽ phần sau tab ở cột phải của menu, như phím tắt chuẩn. macOS không có cột đó.
            let sep = if cfg!(target_os = "macos") { "   " } else { "\t" };
            let text = if k.is_empty() { name.to_string() } else { format!("{name}{sep}{}", show_key(&k)) };
            s.item.set_text(text);
        }
    }

    /// Ngôn ngữ giao diện đổi: gán lại chữ của mọi mục và tooltip.
    pub fn retext(&self) {
        for (m, text) in &self.plain {
            m.set_text(text(t()));
        }
        self.retext_shortcuts();
        self.retext_freeze_toggle();
        self.set_waiting(self.waiting_on.get());
    }

    /// Trạng thái bật/tắt hotkey Freeze, kèm phím đã gán (vd. "Phím tắt chụp đóng băng (F1)").
    pub fn set_freeze_toggle(&self, enabled: bool, key: &str) {
        let key = key.trim();
        *self.freeze_key.borrow_mut() = key.to_string();
        self.retext_freeze_toggle();
        self.freeze_toggle.set_checked(enabled && !key.is_empty());
        // Chưa gán phím thì bật/tắt vô nghĩa: gán trong Cài đặt.
        self.freeze_toggle.set_enabled(!key.is_empty());
    }

    fn retext_freeze_toggle(&self) {
        let (name, key) = (t().tray_freeze_toggle, self.freeze_key.borrow());
        self.freeze_toggle.set_text(if key.is_empty() { name.to_string() } else { format!("{name} ({})", show_key(&key)) });
    }

    /// Đang đếm ngược "Chụp đóng băng sau 3 giây": đổi icon và tooltip, không hiện gì lên màn hình (SPEC 14.4).
    pub fn set_waiting(&self, on: bool) {
        self.waiting_on.set(on);
        let icon = if on { self.waiting.clone() } else { self.normal.clone() };
        let _ = self.icon.set_icon_with_as_template(icon, TEMPLATE);
        if on {
            let _ = self.icon.set_tooltip(Some(t().tray_waiting));
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
        let text = if n == 0 { "Chup".to_string() } else { fill(t().tray_hotkey_errors, &[&n]) };
        let _ = self.icon.set_tooltip(Some(text));
    }
}

/// Phím tắt để hiện trên menu: macOS dùng ký hiệu quen thuộc (`⌃⌘A`), Windows giữ nguyên (`Ctrl+Alt+A`).
fn show_key(accel: &str) -> String {
    if !cfg!(target_os = "macos") {
        return accel.to_string();
    }
    let mut mods = String::new();
    let mut key = "";
    for part in accel.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.push('⌃'),
            "alt" | "option" => mods.push('⌥'),
            "shift" => mods.push('⇧'),
            "super" | "cmd" | "command" => mods.push('⌘'),
            _ => key = part,
        }
    }
    format!("{mods}{key}")
}
