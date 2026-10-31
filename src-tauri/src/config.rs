//! Load / save / migrate `config.json` (SPEC 6.10).

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const CONFIG_VERSION: u32 = 1;

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
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            select_region: "Ctrl+Alt+A".into(),
            capture: "Ctrl+Alt+S".into(),
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
        // Chỗ dành cho migrate: if self.version < 2 { ... }
        self.version = CONFIG_VERSION;
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
    if next.file_name_pattern.trim().is_empty() {
        return Err("Pattern tên file không được để trống".into());
    }
    let (a, b) = (&next.hotkeys.select_region, &next.hotkeys.capture);
    if !a.is_empty() && a.eq_ignore_ascii_case(b) {
        return Err("Hai phím tắt đang trùng nhau".into());
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
