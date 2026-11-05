//! Cửa sổ Cài đặt (SPEC 5.6) bằng control AppKit. Tạm thời (Mốc 1, bước đầu): chưa có giao diện.

use std::{collections::BTreeMap, rc::Rc};

use chup_core::config::Config;
use objc2::MainThreadMarker;

pub struct SettingsView {
    pub config: Config,
    pub hotkey_errors: BTreeMap<String, String>,
    pub preview: String,
}

pub struct Settings;

impl Settings {
    pub fn create(_mtm: MainThreadMarker) -> Rc<Settings> {
        Rc::new(Settings)
    }
    pub fn show(&self) {}
    pub fn refresh(&self, _v: SettingsView) {}
    pub fn retext(&self) {}
    pub fn set_status(&self, _text: &str, _error: bool) {}
}
