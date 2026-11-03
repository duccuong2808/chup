//! Load / save / migrate `config.json` (SPEC 6.10).

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const CONFIG_VERSION: u32 = 2;

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
    /// "Chụp text" (SPEC 15.2): mở overlay Freeze, chọn vùng xong là OCR luôn. Mặc định chưa gán.
    pub ocr: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            select_region: "Ctrl+Alt+A".into(),
            capture: "Ctrl+Alt+S".into(),
            freeze: "F1".into(),
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
    /// Vị trí và cỡ lần trước (người dùng kéo, đổi cỡ là nhớ). Chưa có thì như `Stack`.
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
    /// Cỡ khi mở (khi không có khung lần trước): ảnh tối đa bấy nhiêu % work area mỗi chiều, 20–100.
    pub size: u8,
    /// Khung lần trước, dùng khi `position = remember`.
    pub last: Option<SavedRect>,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self { always_on_top: true, opacity: 100, position: PreviewPosition::Remember, size: 50, last: None }
    }
}

impl PreviewConfig {
    fn clamp(&mut self) {
        self.opacity = crate::preview::clamp_opacity(self.opacity);
        self.size = self.size.clamp(20, 100);
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
            // v1 chưa có hotkey Freeze (serde điền "F1"). Nếu người dùng đã dùng phím đó cho việc khác thì để trống.
            let h = &self.hotkeys;
            if [&h.select_region, &h.capture, &h.ocr].iter().any(|k| k.eq_ignore_ascii_case(&h.freeze)) {
                self.hotkeys.freeze.clear();
            }
        }
        self.version = CONFIG_VERSION;
        self.preview.clamp();
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
                warning: Some(format!(
                    "File cấu hình bị lỗi ({e}). Đã lưu bản cũ thành {} và dùng cấu hình mặc định.",
                    bak.display()
                )),
            }
        }
    }
}

/// Ghi nguyên tử: ghi file tạm rồi rename.
pub fn save(path: &Path, cfg: &Config) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Không tạo được thư mục cấu hình: {e}"))?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("Không ghi được cấu hình: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("Không ghi được cấu hình: {e}"))
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
    let mut next: Config = serde_json::from_value(v).map_err(|e| format!("Giá trị không hợp lệ: {e}"))?;
    next.version = CONFIG_VERSION;
    next.preview.clamp();
    if next.file_name_pattern.trim().is_empty() {
        return Err("Pattern tên file không được để trống".into());
    }
    let used = next.hotkeys.assigned();
    for (i, a) in used.iter().enumerate() {
        if used[i + 1..].iter().any(|b| a.eq_ignore_ascii_case(b)) {
            return Err("Hai phím tắt đang trùng nhau".into());
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
        assert_eq!(next.hotkeys.select_region, "Ctrl+Alt+A");
        assert_eq!(next.stack.edge, Edge::Left);
        assert_eq!(next.save_dir, PathBuf::from("/tmp/pics"));
    }

    #[test]
    fn patch_rejects_duplicate_hotkeys_and_empty_pattern() {
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert!(apply_patch(&cfg, &json!({ "hotkeys": { "capture": "ctrl+alt+a" } })).is_err());
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
        assert_eq!(cfg.version, 2);
        assert_eq!(cfg.hotkeys.freeze, "F1");
        // v1 đã dùng F1 cho hotkey khác: không cướp, để Freeze trống.
        fs::write(&p, r#"{ "version": 1, "hotkeys": { "capture": "f1" } }"#).unwrap();
        assert_eq!(load(&p, Path::new("/pics")).config.hotkeys.freeze, "");
        // v2 giữ nguyên giá trị người dùng, kể cả khi đã tắt.
        fs::write(&p, r#"{ "version": 2, "hotkeys": { "freeze": "" } }"#).unwrap();
        assert_eq!(load(&p, Path::new("/pics")).config.hotkeys.freeze, "");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn patch_rejects_freeze_duplicating_other_hotkey() {
        let cfg = Config::with_save_dir(Path::new("/x"));
        assert!(apply_patch(&cfg, &json!({ "hotkeys": { "freeze": "Ctrl+Alt+S" } })).is_err());
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
        assert!(apply_patch(&cfg, &json!({ "hotkeys": { "ocr": "f1" } })).is_err(), "trùng Freeze");
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
