//! Load / save / migrate `config.json` (SPEC 6.10).

use crate::i18n::{fill, t, UiLanguage};
use crate::share::{self, Expiry, ShareKind};
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const CONFIG_VERSION: u32 = 4;
/// Mặc định cũ (v1, v2) của hotkey "Chụp". Hay bị app khác giữ, v3 đổi sang [`V3_CAPTURE`] (chỉ có trên Windows).
const OLD_DEFAULT_CAPTURE: &str = "Ctrl+Alt+S";
const V3_CAPTURE: &str = "Ctrl+Alt+X";

/// Hotkey mặc định (SPEC Q2). Tránh `PrtScn` (Windows 11 gán cho Snipping Tool); Freeze không dùng `Alt`.
#[cfg(not(target_os = "macos"))]
mod defaults {
    pub const SELECT: &str = "Ctrl+Alt+A";
    pub const CAPTURE: &str = "Ctrl+Alt+X";
    pub const FREEZE: &str = "F1";
}

/// macOS (SPEC 11): `Ctrl+Cmd` (`Super` = Cmd). Tránh `Cmd+Shift+3/4/5` của hệ thống; F1 là phím độ sáng.
#[cfg(target_os = "macos")]
mod defaults {
    pub const SELECT: &str = "Ctrl+Super+A";
    pub const CAPTURE: &str = "Ctrl+Super+X";
    pub const FREEZE: &str = "Ctrl+Super+Z";
}

pub use defaults::{CAPTURE as DEFAULT_CAPTURE, FREEZE as DEFAULT_FREEZE, SELECT as DEFAULT_SELECT};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StackMonitor {
    Primary,
    Cursor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AfterCapture {
    Close,
    Keep,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkeys {
    /// Rỗng = tắt.
    pub select_region: String,
    pub capture: String,
    /// Chụp đóng băng (SPEC 14). Không dùng Alt: Alt làm đóng menu đang mở trước khi kịp chụp.
    pub freeze: String,
    /// Bật/tắt hotkey Freeze mà không mất phím đã gán (Cài đặt, menu tray). Tắt thì phím (F1) trả lại cho app khác.
    pub freeze_enabled: bool,
    /// "Chụp text" (SPEC 15.2): mở overlay Freeze, chọn vùng xong là OCR luôn. Mặc định chưa gán.
    pub ocr: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            select_region: DEFAULT_SELECT.into(),
            capture: DEFAULT_CAPTURE.into(),
            freeze: DEFAULT_FREEZE.into(),
            freeze_enabled: true,
            ocr: String::new(),
        }
    }
}

impl Hotkeys {
    /// Các hotkey đang gán (bỏ chuỗi rỗng).
    fn assigned(&self) -> Vec<&String> {
        [&self.select_region, &self.capture, &self.freeze, &self.ocr].into_iter().filter(|s| !s.is_empty()).collect()
    }
}

/// Ngôn ngữ OCR (SPEC 15.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OcrLanguage {
    /// Tiếng Anh: Windows OCR có sẵn, không tải gì.
    #[default]
    En,
    /// Tiếng Việt + Anh: Tesseract (`chup-ocr.exe`) với dữ liệu tessdata_fast tải về.
    Vi,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OcrConfig {
    pub language: OcrLanguage,
    /// Tự nhận diện chữ ở nền khi mở Preview để chọn chữ được ngay (SPEC 17.2). Config cũ thiếu field: bật.
    pub auto_preview: bool,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self { language: OcrLanguage::default(), auto_preview: true }
    }
}

/// Tự cập nhật (SPEC 18, chup-22n). Config cũ thiếu field: bật tự kiểm tra.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateConfig {
    /// Mỗi ngày tự kiểm tra bản mới (có bản mới thì hỏi trước khi cài).
    pub auto_check: bool,
    /// Lần kiểm tra gần nhất (giây Unix, 0 = chưa).
    pub last_check: u64,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self { auto_check: true, last_check: 0 }
    }
}

/// Tự động chọn vùng (SPEC 16). Config cũ thiếu field: bật cả 2.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoDetectConfig {
    /// Rê chuột trong overlay là tô khung cửa sổ / phần tử dưới con trỏ.
    pub enabled: bool,
    /// Nhận khung từ ảnh khi UI Automation không có phần tử con chứa con trỏ.
    pub image_edges: bool,
}

impl Default for AutoDetectConfig {
    fn default() -> Self {
        Self { enabled: true, image_edges: true }
    }
}

/// Người dùng đã đồng ý gửi ảnh ra ngoài theo từng kiểu (hộp thoại lần đầu, SPEC 19.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShareConsent {
    pub image: bool,
    pub private: bool,
    pub permanent: bool,
}

/// Chia sẻ ảnh qua link (SPEC 19.3). Field mới có serde default nên không tăng version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShareConfig {
    pub expiry: Expiry,
    /// Kiểu ở đầu menu Upload.
    pub default_kind: ShareKind,
    /// PrivateBin: xoá sau lần xem đầu.
    pub burn_after_reading: bool,
    /// Đã chuẩn hoá (`https://…/`).
    pub privatebin_url: String,
    /// Đã cất userhash catbox trong kho bí mật của OS (bản thân userhash không bao giờ nằm trong config).
    pub catbox_account: bool,
    pub consent: ShareConsent,
}

impl Default for ShareConfig {
    fn default() -> Self {
        Self {
            expiry: Expiry::default(),
            default_kind: ShareKind::Image,
            burn_after_reading: false,
            privatebin_url: share::DEFAULT_PRIVATEBIN.into(),
            catbox_account: false,
            consent: ShareConsent::default(),
        }
    }
}

impl ShareConfig {
    /// Địa chỉ hỏng thì về mặc định; chưa có tài khoản catbox thì Link lâu dài không làm mặc định được.
    fn normalize(&mut self) {
        self.privatebin_url =
            share::normalize_privatebin_url(&self.privatebin_url).unwrap_or_else(|| share::DEFAULT_PRIVATEBIN.into());
        if self.default_kind == ShareKind::Permanent && !self.catbox_account {
            self.default_kind = ShareKind::Image;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StackConfig {
    pub edge: Edge,
    pub monitor: StackMonitor,
}

impl Default for StackConfig {
    fn default() -> Self {
        Self {
            edge: Edge::Right,
            monitor: StackMonitor::Primary,
        }
    }
}

/// Chỗ mở Preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewPosition {
    /// Chỗ lần trước (người dùng kéo là nhớ), cỡ tự tính theo ảnh. Chưa có chỗ thì như `Stack`.
    Remember,
    /// Sát cạnh stack, căn đáy.
    Stack,
    Center,
}

/// Rect đã lưu (physical px toàn desktop).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Cửa sổ Preview khi bấm thumbnail (SPEC 17).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PreviewConfig {
    /// Luôn nằm trên cửa sổ khác.
    pub always_on_top: bool,
    /// %, 20–100.
    pub opacity: u8,
    pub position: PreviewPosition,
    /// Cỡ khi mở: diện tích tối đa bằng ảnh rộng, cao bấy nhiêu % work area, 20–100 (`preview::auto_zoom`).
    pub size: u8,
    /// Khung lần trước, dùng khi `position = remember`. Chỉ quyết định chỗ mở, không quyết định cỡ.
    pub last: Option<SavedRect>,
    /// % so với cỡ tự tính người dùng đã chọn bằng cách kéo mép, 50–200. Dùng khi `position = remember`.
    pub size_scale: u16,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self { always_on_top: true, opacity: 100, position: PreviewPosition::Remember, size: 50, last: None, size_scale: 100 }
    }
}

impl PreviewConfig {
    fn clamp(&mut self) {
        self.opacity = crate::preview::clamp_opacity(self.opacity);
        self.size = self.size.clamp(20, 100);
        self.size_scale = self.size_scale.clamp(crate::preview::MIN_SIZE_SCALE, crate::preview::MAX_SIZE_SCALE);
    }

    /// Zoom mở ảnh `img` trên work area `work` (SPEC 17.2).
    pub fn open_zoom(&self, img: (u32, u32), work: crate::coords::Rect) -> f64 {
        let user = if self.position == PreviewPosition::Remember { self.size_scale } else { 100 };
        crate::preview::auto_zoom(img, work, self.size, user)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub version: u32,
    pub save_dir: PathBuf,
    pub file_name_pattern: String,
    pub copy_to_clipboard: bool,
    pub hotkeys: Hotkeys,
    pub stack: StackConfig,
    pub after_capture_normal: AfterCapture,
    pub launch_at_startup: bool,
    /// Fallback SPEC 6.4: ẩn frame/toolbar/stack trước khi chụp.
    /// Tự bật nếu không loại được cửa sổ khỏi ảnh chụp; người dùng cũng có thể bật tay.
    pub hide_ui_during_capture: bool,
    /// Phase 2.2: field mới có serde default nên không tăng version.
    pub preview: PreviewConfig,
    /// Phase 2.3 (OCR), cũng không tăng version.
    pub ocr: OcrConfig,
    /// Phase 2.4 (tự động chọn vùng), cũng không tăng version.
    pub auto_detect: AutoDetectConfig,
    /// Ngôn ngữ giao diện (v4, chup-81y). Cài mới: theo hệ điều hành; config cũ: tiếng Việt (xem `normalize`).
    pub ui_language: UiLanguage,
    /// Tự cập nhật (chup-22n), không tăng version.
    pub update: UpdateConfig,
    /// Chia sẻ ảnh qua link (SPEC 19), không tăng version.
    pub share: ShareConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            save_dir: PathBuf::new(),
            file_name_pattern: "chup-{yyyyMMdd-HHmmss-SSS}".into(),
            copy_to_clipboard: true,
            hotkeys: Hotkeys::default(),
            stack: StackConfig::default(),
            after_capture_normal: AfterCapture::Close,
            launch_at_startup: false,
            hide_ui_during_capture: false,
            preview: PreviewConfig::default(),
            ocr: OcrConfig::default(),
            auto_detect: AutoDetectConfig::default(),
            ui_language: UiLanguage::System,
            update: UpdateConfig::default(),
            share: ShareConfig::default(),
        }
    }
}

impl Config {
    pub fn with_save_dir(default_save_dir: &Path) -> Self {
        Self {
            save_dir: default_save_dir.to_path_buf(),
            ..Self::default()
        }
    }

    /// Chuẩn hoá sau khi đọc: điền chỗ trống, migrate version.
    fn normalize(mut self, default_save_dir: &Path) -> Self {
        if self.save_dir.as_os_str().is_empty() {
            self.save_dir = default_save_dir.to_path_buf();
        }
        if self.file_name_pattern.trim().is_empty() {
            self.file_name_pattern = Config::default().file_name_pattern;
        }
        if self.version < 2 {
            // v1 chưa có hotkey Freeze (serde điền mặc định, "F1" trên Windows). Nếu người dùng đã dùng phím đó cho việc khác thì để trống.
            let h = &self.hotkeys;
            if [&h.select_region, &h.capture, &h.ocr].iter().any(|k| k.eq_ignore_ascii_case(&h.freeze)) {
                self.hotkeys.freeze.clear();
            }
        }
        if self.version < 3 && self.hotkeys.capture.eq_ignore_ascii_case(OLD_DEFAULT_CAPTURE) {
            // v3: mặc định cũ của "Chụp" hay bị app khác giữ. Đổi sang mặc định mới nếu phím đó chưa dùng cho việc khác.
            let h = &self.hotkeys;
            if ![&h.select_region, &h.freeze, &h.ocr].iter().any(|k| k.eq_ignore_ascii_case(V3_CAPTURE)) {
                self.hotkeys.capture = V3_CAPTURE.into();
            }
        }
        if self.version < 4 {
            // v4 thêm ngôn ngữ giao diện. Người đã dùng Chup (giao diện chỉ có tiếng Việt) giữ tiếng Việt,
            // không tự đổi sang tiếng Anh chỉ vì Windows của họ là tiếng Anh. Cài mới thì theo hệ điều hành.
            self.ui_language = UiLanguage::Vi;
        }
        self.version = CONFIG_VERSION;
        self.preview.clamp();
        self.share.normalize();
        self
    }
}

pub struct LoadOutcome {
    pub config: Config,
    /// Có giá trị khi file hỏng và đã được backup.
    pub warning: Option<String>,
}

pub fn load(path: &Path, default_save_dir: &Path) -> LoadOutcome {
    let defaults = || Config::with_save_dir(default_save_dir);
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => {
            return LoadOutcome {
                config: defaults(),
                warning: None,
            }
        }
    };
    match serde_json::from_str::<Config>(&raw) {
        Ok(cfg) => LoadOutcome {
            config: cfg.normalize(default_save_dir),
            warning: None,
        },
        Err(e) => {
            let bak = path.with_file_name("config.bak.json");
            let _ = fs::rename(path, &bak);
            LoadOutcome {
                config: defaults(),
                warning: Some(fill(t().config_corrupt, &[&e, &bak.display()])),
            }
        }
    }
}

/// Ghi nguyên tử: ghi file tạm rồi rename.
pub fn save(path: &Path, cfg: &Config) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| fill(t().err_config_dir, &[&e]))?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| fill(t().err_config_write, &[&e]))?;
    fs::rename(&tmp, path).map_err(|e| fill(t().err_config_write, &[&e]))
}

/// Merge sâu `patch` vào `base` (object JSON).
pub fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(b), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                merge_json(b.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// Áp patch từ màn hình Cài đặt, trả về config mới (chưa lưu).
pub fn apply_patch(cfg: &Config, patch: &serde_json::Value) -> Result<Config, String> {
    let mut v = serde_json::to_value(cfg).map_err(|e| e.to_string())?;
    merge_json(&mut v, patch);
    let mut next: Config = serde_json::from_value(v).map_err(|e| fill(t().err_config_value, &[&e]))?;
    next.version = CONFIG_VERSION;
    next.preview.clamp();
    next.share.privatebin_url =
        share::normalize_privatebin_url(&next.share.privatebin_url).ok_or(t().st_share_instance_bad)?;
    if next.share.default_kind == ShareKind::Permanent && !next.share.catbox_account {
        next.share.default_kind = ShareKind::Image;
    }
    if next.file_name_pattern.trim().is_empty() {
        return Err(t().err_pattern_empty.into());
    }
    let used = next.hotkeys.assigned();
    for (i, a) in used.iter().enumerate() {
        if used[i + 1..].iter().any(|b| a.eq_ignore_ascii_case(b)) {
            return Err(t().err_hotkeys_duplicate.into());
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patch_merges_nested_fields() {
        let cfg = Config::with_save_dir(Path::new("/tmp/pics"));
        let next = apply_patch(&cfg, &json!({ "hotkeys": { "capture": "F2" }, "stack": { "edge": "left" } })).unwrap();
        assert_eq!(next.hotkeys.capture, "F2");
        assert_eq!(next.hotkeys.select_region, DEFAULT_SELECT);
        assert_eq!(next.stack.edge, Edge::Left);
        assert_eq!(next.save_dir, PathBuf::from("/tmp/pics"));
    }

    #[test]
    fn share_defaults_and_privatebin_url_validation() {
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert_eq!(cfg.share.privatebin_url, "https://privatebin.net/");
        assert_eq!(cfg.share.expiry, Expiry::D1);
        let next = apply_patch(&cfg, &json!({ "share": { "privatebinUrl": " https://paste.example.org/pb " } })).unwrap();
        assert_eq!(next.share.privatebin_url, "https://paste.example.org/pb/");
        assert!(apply_patch(&cfg, &json!({ "share": { "privatebinUrl": "http://x.org" } })).is_err());
        let next = apply_patch(&cfg, &json!({ "share": { "defaultKind": "permanent" } })).unwrap();
        assert_eq!(next.share.default_kind, ShareKind::Image, "chưa có tài khoản catbox");
        // Config cũ (chưa có "share"): mặc định; địa chỉ hỏng trong file: về mặc định.
        let dir = std::env::temp_dir().join(format!("chup-cfg-share-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        fs::write(&p, r#"{ "version": 4, "share": { "privatebinUrl": "ftp://x", "expiry": "3d" } }"#).unwrap();
        let cfg = load(&p, &dir).config;
        assert_eq!(cfg.share.privatebin_url, "https://privatebin.net/");
        assert_eq!(cfg.share.expiry, Expiry::D3);
        assert!(!cfg.share.consent.image);
    }

    #[test]
    fn patch_rejects_duplicate_hotkeys_and_empty_pattern() {
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert!(apply_patch(&cfg, &json!({ "hotkeys": { "capture": DEFAULT_SELECT.to_lowercase() } })).is_err());
        assert!(apply_patch(&cfg, &json!({ "fileNamePattern": "  " })).is_err());
        assert!(apply_patch(&cfg, &json!({ "stack": { "edge": "top" } })).is_err());
    }

    #[test]
    fn v1_file_migrates_to_v2_with_freeze_hotkey() {
        let dir = std::env::temp_dir().join(format!("chup-cfg-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        fs::write(&p, r#"{ "version": 1, "hotkeys": { "selectRegion": "Ctrl+Alt+A", "capture": "Ctrl+Alt+S" } }"#).unwrap();
        let cfg = load(&p, Path::new("/pics")).config;
        assert_eq!(cfg.version, CONFIG_VERSION);
        assert_eq!(cfg.hotkeys.freeze, DEFAULT_FREEZE);
        assert!(cfg.hotkeys.freeze_enabled);
        // v1 đã dùng phím Freeze mặc định cho hotkey khác: không cướp, để Freeze trống.
        let v1 = json!({ "version": 1, "hotkeys": { "capture": DEFAULT_FREEZE.to_lowercase() } });
        fs::write(&p, v1.to_string()).unwrap();
        assert_eq!(load(&p, Path::new("/pics")).config.hotkeys.freeze, "");
        // v2 giữ nguyên giá trị người dùng, kể cả khi đã tắt.
        fs::write(&p, r#"{ "version": 2, "hotkeys": { "freeze": "" } }"#).unwrap();
        assert_eq!(load(&p, Path::new("/pics")).config.hotkeys.freeze, "");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn v3_moves_old_capture_default_only() {
        let dir = std::env::temp_dir().join(format!("chup-cfg-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        let load_capture = |json: &str| {
            fs::write(&p, json).unwrap();
            load(&p, Path::new("/pics")).config.hotkeys.capture
        };
        assert_eq!(Config::default().hotkeys.capture, DEFAULT_CAPTURE);
        assert_eq!(load_capture(r#"{ "version": 2, "hotkeys": { "capture": "Ctrl+Alt+S" } }"#), "Ctrl+Alt+X");
        // Người dùng tự chọn phím khác: giữ.
        assert_eq!(load_capture(r#"{ "version": 2, "hotkeys": { "capture": "Ctrl+Shift+F9" } }"#), "Ctrl+Shift+F9");
        // Ctrl+Alt+X đã gán cho việc khác: giữ phím cũ, không tạo trùng.
        assert_eq!(
            load_capture(r#"{ "version": 2, "hotkeys": { "capture": "Ctrl+Alt+S", "ocr": "Ctrl+Alt+X" } }"#),
            "Ctrl+Alt+S"
        );
        // Đã là v3: người dùng chủ động chọn lại Ctrl+Alt+S thì giữ.
        assert_eq!(load_capture(r#"{ "version": 3, "hotkeys": { "capture": "Ctrl+Alt+S" } }"#), "Ctrl+Alt+S");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ui_language_new_follows_system_old_keeps_vietnamese() {
        assert_eq!(Config::default().ui_language, UiLanguage::System);
        let dir = std::env::temp_dir().join(format!("chup-cfg-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        assert_eq!(load(&p, Path::new("/pics")).config.ui_language, UiLanguage::System, "chưa có file: cài mới");
        fs::write(&p, r#"{ "version": 3 }"#).unwrap();
        assert_eq!(load(&p, Path::new("/pics")).config.ui_language, UiLanguage::Vi);
        fs::write(&p, r#"{ "version": 4, "uiLanguage": "en" }"#).unwrap();
        assert_eq!(load(&p, Path::new("/pics")).config.ui_language, UiLanguage::En);
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert_eq!(apply_patch(&cfg, &json!({ "uiLanguage": "vi" })).unwrap().ui_language, UiLanguage::Vi);
        assert!(apply_patch(&cfg, &json!({ "uiLanguage": "fr" })).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn freeze_toggle_defaults_on_and_patches() {
        let old: Config = serde_json::from_value(json!({ "version": 2, "hotkeys": { "freeze": "F1" } })).unwrap();
        assert!(old.hotkeys.freeze_enabled, "config cũ thiếu field: bật");
        let cfg = Config::with_save_dir(Path::new("/x"));
        let off = apply_patch(&cfg, &json!({ "hotkeys": { "freezeEnabled": false } })).unwrap();
        assert!(!off.hotkeys.freeze_enabled);
        assert_eq!(off.hotkeys.freeze, DEFAULT_FREEZE, "tắt không làm mất phím");
    }

    #[test]
    fn patch_rejects_freeze_duplicating_other_hotkey() {
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert!(apply_patch(&cfg, &json!({ "hotkeys": { "freeze": DEFAULT_CAPTURE } })).is_err());
        assert!(apply_patch(&cfg, &json!({ "hotkeys": { "freeze": "", "capture": "" } })).is_ok());
    }

    #[test]
    fn preview_defaults_and_opacity_is_clamped() {
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert_eq!(cfg.preview.opacity, 100);
        assert!(cfg.preview.always_on_top);
        assert_eq!(cfg.preview.position, PreviewPosition::Remember);
        assert_eq!(cfg.preview.size, 50);
        let next = apply_patch(
            &cfg,
            &json!({ "preview": { "opacity": 5, "alwaysOnTop": false, "position": "center", "size": 0 } }),
        )
        .unwrap();
        assert_eq!((next.preview.always_on_top, next.preview.opacity), (false, 20));
        assert_eq!((next.preview.position, next.preview.size), (PreviewPosition::Center, 20));
        assert!(apply_patch(&cfg, &json!({ "preview": { "opacity": 300 } })).is_err());
        let next = apply_patch(&cfg, &json!({ "preview": { "last": { "x": -10, "y": 5, "w": 300, "h": 200 } } })).unwrap();
        assert_eq!(next.preview.last, Some(SavedRect { x: -10, y: 5, w: 300, h: 200 }));

        let dir = std::env::temp_dir().join(format!("chup-cfg-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        fs::write(&p, r#"{ "version": 2, "preview": { "opacity": 0 } }"#).unwrap();
        let cfg = load(&p, Path::new("/pics")).config;
        assert_eq!(cfg.preview.opacity, 20);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ocr_defaults_language_and_hotkey() {
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert_eq!(cfg.ocr.language, OcrLanguage::En);
        assert_eq!(cfg.hotkeys.ocr, "");
        let next = apply_patch(&cfg, &json!({ "ocr": { "language": "vi" }, "hotkeys": { "ocr": "Ctrl+Alt+T" } })).unwrap();
        assert_eq!(next.ocr.language, OcrLanguage::Vi);
        assert_eq!(next.hotkeys.ocr, "Ctrl+Alt+T");
        assert!(apply_patch(&cfg, &json!({ "hotkeys": { "ocr": DEFAULT_FREEZE.to_lowercase() } })).is_err(), "trùng Freeze");
        assert!(apply_patch(&cfg, &json!({ "ocr": { "language": "fr" } })).is_err());
        let s = serde_json::to_value(&next).unwrap();
        assert_eq!(s["ocr"], json!({ "language": "vi", "autoPreview": true }));
    }

    #[test]
    fn ocr_auto_preview_defaults_on_for_old_configs() {
        let old: OcrConfig = serde_json::from_value(json!({ "language": "vi" })).unwrap();
        assert!(old.auto_preview, "config cũ thiếu field: bật");
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert!(cfg.ocr.auto_preview);
        let off = apply_patch(&cfg, &json!({ "ocr": { "autoPreview": false } })).unwrap();
        assert!(!off.ocr.auto_preview);
        assert_eq!(off.ocr.language, OcrLanguage::En, "patch một field giữ field kia");
    }

    #[test]
    fn auto_detect_defaults_on_for_old_configs() {
        let old: Config = serde_json::from_value(json!({ "version": 2 })).unwrap();
        assert_eq!(old.auto_detect, AutoDetectConfig { enabled: true, image_edges: true });
        let cfg = Config::with_save_dir(Path::new("/x"));
        let off = apply_patch(&cfg, &json!({ "autoDetect": { "imageEdges": false } })).unwrap();
        assert!(off.auto_detect.enabled, "patch một field giữ field kia");
        assert!(!off.auto_detect.image_edges);
        let s = serde_json::to_value(&off).unwrap();
        assert_eq!(s["autoDetect"], json!({ "enabled": true, "imageEdges": false }));
    }

    #[test]
    fn corrupt_file_is_backed_up() {
        let dir = std::env::temp_dir().join(format!("chup-cfg-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        fs::write(&p, "{ not json").unwrap();
        let out = load(&p, Path::new("/pics"));
        assert!(out.warning.is_some());
        assert!(dir.join("config.bak.json").exists());
        assert_eq!(out.config.save_dir, PathBuf::from("/pics"));
        save(&p, &out.config).unwrap();
        assert_eq!(load(&p, Path::new("/other")).config, out.config);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn default_hotkeys_follow_os() {
        let h = Hotkeys::default();
        if cfg!(target_os = "macos") {
            assert_eq!((h.select_region.as_str(), h.capture.as_str(), h.freeze.as_str()), ("Ctrl+Super+A", "Ctrl+Super+X", "Ctrl+Super+Z"));
        } else {
            assert_eq!((h.select_region.as_str(), h.capture.as_str(), h.freeze.as_str()), ("Ctrl+Alt+A", "Ctrl+Alt+X", "F1"));
        }
        // Mặc định phải parse được và không trùng nhau.
        assert!(apply_patch(&Config::with_save_dir(Path::new("/x")), &json!({})).is_ok());
    }

    #[test]
    fn preview_size_scale_defaults_clamps_and_patches() {
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert_eq!(cfg.preview.size_scale, 100);
        let old: Config = serde_json::from_value(json!({ "version": 4, "preview": { "size": 50 } })).unwrap();
        assert_eq!(old.preview.size_scale, 100, "config cũ thiếu field: 100%");
        let next = apply_patch(&cfg, &json!({ "preview": { "sizeScale": 999 } })).unwrap();
        assert_eq!(next.preview.size_scale, 200);
    }

    #[test]
    fn preview_open_zoom_ignores_previous_box_and_uses_scale_only_when_remembering() {
        let work = crate::coords::Rect::new(0, 0, 3600, 2264);
        let img = (3600, 2128);
        let mut p = PreviewConfig::default();
        let base = p.open_zoom(img, work);
        // Khung dải mỏng của ảnh trước không làm nhỏ ảnh sau.
        p.last = Some(SavedRect { x: 1784, y: 2064, w: 1400, h: 120 });
        assert_eq!(p.open_zoom(img, work), base);
        p.size_scale = 150;
        assert!((p.open_zoom(img, work) - base * 1.5).abs() < 1e-9);
        p.position = PreviewPosition::Center;
        assert_eq!(p.open_zoom(img, work), base);
    }

    #[test]
    fn missing_fields_get_defaults() {
        let dir = std::env::temp_dir().join(format!("chup-cfg-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        fs::write(&p, r#"{ "copyToClipboard": false }"#).unwrap();
        let out = load(&p, Path::new("/pics"));
        assert!(!out.config.copy_to_clipboard);
        assert_eq!(out.config.hotkeys, Hotkeys::default());
        assert_eq!(out.config.save_dir, PathBuf::from("/pics"));
        fs::remove_dir_all(dir).unwrap();
    }
}
