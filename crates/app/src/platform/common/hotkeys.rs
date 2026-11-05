//! Phím tắt toàn cục (SPEC 5.1, 7). Báo lỗi bằng câu tiếng Việt khi tổ hợp đã có chủ.
//! Dùng chung cho Windows và macOS (`#[path]`): cần `super::app::Action` và `super::win::emit` của vỏ OS.

use chup_core::i18n::{fill, t};
use std::{collections::BTreeMap, str::FromStr};

use chup_core::config::Hotkeys as HotkeyConfig;
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use super::{app::Action, win::emit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    SelectRegion,
    Capture,
    Freeze,
    /// "Chụp text" (SPEC 15.2).
    Ocr,
}

pub struct Hotkeys {
    mgr: GlobalHotKeyManager,
    registered: Vec<(HotKey, Kind)>,
}

/// RegisterHotKey thất bại gần như luôn vì tổ hợp đã có chủ (app khác hoặc chính Windows).
fn taken_message(accel: &str) -> String {
    fill(t().hotkey_taken, &[&accel])
}

fn invalid_message(accel: &str) -> String {
    fill(t().hotkey_invalid, &[&accel])
}

impl Hotkeys {
    pub fn new() -> Result<Hotkeys, String> {
        let mgr = GlobalHotKeyManager::new().map_err(|e| fill(t().err_hotkey_init, &[&e]))?;
        GlobalHotKeyEvent::set_event_handler(Some(|e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                emit(Action::Hotkey(e.id));
            }
        }));
        Ok(Hotkeys { mgr, registered: vec![] })
    }

    /// Huỷ hết rồi đăng ký lại; trả về lỗi theo tên (`selectRegion`, `capture`, `freeze`, `ocr`).
    pub fn apply(&mut self, cfg: &HotkeyConfig) -> BTreeMap<String, String> {
        self.pause();
        let mut errors = BTreeMap::new();
        for (name, accel, kind) in [
            ("selectRegion", cfg.select_region.trim(), Kind::SelectRegion),
            ("capture", cfg.capture.trim(), Kind::Capture),
            // Tắt trong Cài đặt / menu tray: không giữ phím (F1 trả lại cho app khác), vẫn nhớ phím đã gán.
            ("freeze", if cfg.freeze_enabled { cfg.freeze.trim() } else { "" }, Kind::Freeze),
            ("ocr", cfg.ocr.trim(), Kind::Ocr),
        ] {
            if accel.is_empty() {
                continue;
            }
            let hk = match HotKey::from_str(accel) {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("parse hotkey {name}={accel:?}: {e}");
                    errors.insert(name.to_string(), invalid_message(accel));
                    continue;
                }
            };
            match self.mgr.register(hk) {
                Ok(()) => self.registered.push((hk, kind)),
                Err(e) => {
                    log::warn!("register hotkey {name}={accel}: {e}");
                    errors.insert(name.to_string(), taken_message(accel));
                }
            }
        }
        errors
    }

    /// Tạm tắt khi màn hình Cài đặt đang ghi phím.
    pub fn pause(&mut self) {
        for (hk, _) in self.registered.drain(..) {
            let _ = self.mgr.unregister(hk);
        }
    }

    pub fn kind_of(&self, id: u32) -> Option<Kind> {
        self.registered.iter().find(|(h, _)| h.id() == id).map(|(_, k)| *k)
    }
}
