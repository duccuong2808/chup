//! State duy nhất của app (SPEC 6.1, 6.6). Frontend chỉ hiển thị.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use serde::Serialize;

use crate::{
    config::Config,
    coords::{MonitorInfo, Rect},
    stack::Stack,
};

#[derive(Clone, Debug)]
pub struct Region {
    pub rect: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameMode {
    Hidden,
    Normal,
    Faded,
}

pub struct Paths {
    pub config_file: PathBuf,
    pub thumbs_dir: PathBuf,
    pub default_save_dir: PathBuf,
}

pub struct Inner {
    pub config: Config,
    pub region: Option<Region>,
    pub last_region: Option<Region>,
    pub mode: FrameMode,
    pub stack: Stack,
    pub monitors: Vec<MonitorInfo>,
    /// label overlay -> màn hình mà nó phủ.
    pub overlays: BTreeMap<String, MonitorInfo>,
    pub selecting: bool,
    /// Lề physical của cửa sổ frame quanh region.
    pub frame_margin: i32,
    /// Hình học cửa sổ frame theo event Moved/Resized (physical).
    pub frame_pos: (i32, i32),
    pub frame_size: (u32, u32),
    /// Kích thước nội dung stack do frontend báo (CSS px).
    pub stack_css: (f64, f64),
    pub stack_visible: bool,
    /// Màn hình stack đang đứng; chốt lúc stack hiện từ trạng thái ẩn.
    pub stack_monitor_key: Option<String>,
    /// Lỗi đăng ký hotkey: tên hotkey -> lý do.
    pub hotkey_errors: BTreeMap<String, String>,
    /// `false` nếu không loại được cửa sổ của Chup khỏi ảnh chụp.
    pub capture_exclusion_ok: bool,
}

pub struct AppState {
    inner: Mutex<Inner>,
    pub paths: Paths,
}

impl AppState {
    pub fn new(config: Config, paths: Paths) -> Self {
        Self {
            inner: Mutex::new(Inner {
                config,
                region: None,
                last_region: None,
                mode: FrameMode::Hidden,
                stack: Stack::default(),
                monitors: vec![],
                overlays: BTreeMap::new(),
                selecting: false,
                frame_margin: 0,
                frame_pos: (0, 0),
                frame_size: (0, 0),
                stack_css: (0.0, 0.0),
                stack_visible: false,
                stack_monitor_key: None,
                hotkey_errors: BTreeMap::new(),
                capture_exclusion_ok: true,
            }),
            paths,
        }
    }

    /// Không giữ guard qua `.await`.
    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}
