//! Phím tắt toàn cục (SPEC 5.1, 7). Báo lỗi bằng câu tiếng Việt khi tổ hợp đã có chủ.

use std::{collections::BTreeMap, str::FromStr};

use chup_core::config::Hotkeys as HotkeyConfig;
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use super::{app::Action, win::emit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    SelectRegion,
    Capture,
    Freeze,
}

pub struct Hotkeys {
    mgr: GlobalHotKeyManager,
    registered: Vec<(HotKey, Kind)>,
}

/// RegisterHotKey thất bại gần như luôn vì tổ hợp đã có chủ (app khác hoặc chính Windows).
fn taken_message(accel: &str) -> String {
    format!("Không dùng được {accel} vì app khác (hoặc Windows) đang dùng tổ hợp này. Hãy chọn tổ hợp khác.")
}

fn invalid_message(accel: &str) -> String {
    format!("Phím tắt \"{accel}\" không hợp lệ. Hãy bấm vào ô và nhấn lại tổ hợp phím.")
}

impl Hotkeys {
    pub fn new() -> Result<Hotkeys, String> {
        let mgr = GlobalHotKeyManager::new().map_err(|e| format!("Không khởi tạo được phím tắt: {e}"))?;
        GlobalHotKeyEvent::set_event_handler(Some(|e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                emit(Action::Hotkey(e.id));
            }
        }));
        Ok(Hotkeys { mgr, registered: vec![] })
    }

    /// Huỷ hết rồi đăng ký lại; trả về lỗi theo tên (`selectRegion`, `capture`, `freeze`).
    pub fn apply(&mut self, cfg: &HotkeyConfig) -> BTreeMap<String, String> {
        self.pause();
        let mut errors = BTreeMap::new();
        for (name, accel, kind) in [
            ("selectRegion", cfg.select_region.trim(), Kind::SelectRegion),
            ("capture", cfg.capture.trim(), Kind::Capture),
            ("freeze", cfg.freeze.trim(), Kind::Freeze),
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
