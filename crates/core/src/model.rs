//! State machine của app (SPEC 5, 6.6). Không biết gì về cửa sổ: nhận sự kiện từ lớp platform
//! và trả về danh sách `Effect` để lớp platform thực hiện. Nhờ vậy logic test được trên mọi OS.

use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    capture::{PersistOptions, PersistOutcome, Shot},
    config::{self, AfterCapture, Config},
    coords::{self, frame_margin, monitor_at, MonitorInfo, Rect},
    stack::{self, Stack},
};

/// Kích thước toolbar (CSS px = DIP) theo mode.
pub const TOOLBAR_NORMAL_DIP: (f64, f64) = (220.0, 44.0);
pub const TOOLBAR_FADED_DIP: (f64, f64) = (116.0, 40.0);
/// Khoảng cách toolbar với viền frame (DIP).
const TOOLBAR_GAP_DIP: f64 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameMode {
    Hidden,
    Normal,
    Faded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub kind: ToastKind,
    pub message: String,
}

impl Toast {
    pub fn error(m: impl Into<String>) -> Self {
        Self { kind: ToastKind::Error, message: m.into() }
    }
    pub fn info(m: impl Into<String>) -> Self {
        Self { kind: ToastKind::Info, message: m.into() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeAction {
    Save,
    Copy,
}

/// Phiên Freeze (SPEC 14.3). Vùng đang chọn là state của overlay, Model không giữ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezePhase {
    /// Đang chụp mọi màn hình, chưa hiện gì.
    Grabbing,
    /// Overlay đang hiện ảnh đóng băng.
    Selecting,
}

/// Hẹn giờ "Chụp đóng băng sau 3 giây".
pub const FREEZE_DELAY_MS: u32 = 3_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureJob {
    pub rect: Rect,
    /// Fallback SPEC 6.4: ẩn toolbar và stack trước khi chụp.
    pub hide_ui: bool,
}

/// Việc lớp platform phải làm. Dữ liệu chi tiết (rect frame, rect toolbar...) đọc lại từ `Model`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    ShowOverlays,
    HideOverlays,
    /// Hiện frame quanh `Model::frame_outer()` theo `Model::mode`.
    ShowFrame,
    /// Mode của frame đổi (Normal <-> Faded): vẽ lại, đổi click-through.
    UpdateFrameMode,
    HideFrame,
    /// Đặt lại toolbar theo `Model::toolbar_rect()` và hiện nó.
    LayoutToolbar,
    HideToolbar,
    /// Danh sách shot hoặc cấu hình stack đổi: vẽ và đặt lại cửa sổ stack.
    StackChanged,
    Toast(Toast),
    StartCapture(CaptureJob),
    ApplyHotkeys,
    SyncAutostart,
    /// Config đổi: cửa sổ Cài đặt (nếu đang mở) cập nhật lại.
    ConfigChanged,
    /// Chụp mọi màn hình (chưa hiện cửa sổ nào), xong thì gọi `freeze_grabbed`.
    FreezeGrab,
    /// Hiện overlay ở chế độ Freeze với ảnh vừa chụp.
    ShowFreezeOverlays,
    /// Bắt đầu, hoặc đếm lại, hẹn giờ `FREEZE_DELAY_MS`. Tray đổi icon.
    StartFreezeTimer,
    /// Tắt hẹn giờ, tray về icon thường.
    CancelFreezeTimer,
    /// Crop vùng (physical px toàn desktop) từ ảnh đóng băng rồi `persist_shot`.
    FreezePersist(Rect),
    /// Crop rồi chỉ copy vào clipboard: không tạo file, không vào stack.
    FreezeCopy(Rect),
}

pub struct Paths {
    pub config_file: PathBuf,
    pub thumbs_dir: PathBuf,
    pub default_save_dir: PathBuf,
}

pub struct Model {
    pub config: Config,
    pub paths: Paths,
    pub monitors: Vec<MonitorInfo>,
    pub selecting: bool,
    pub region: Option<Rect>,
    pub last_region: Option<Rect>,
    pub mode: FrameMode,
    /// Lề physical của cửa sổ frame quanh region (chứa viền và handle).
    pub frame_margin: i32,
    pub stack: Stack,
    /// Lỗi đăng ký hotkey: tên hotkey (`selectRegion`, `capture`) -> câu báo lỗi.
    pub hotkey_errors: BTreeMap<String, String>,
    /// `false` nếu OS không loại được cửa sổ của Chup khỏi ảnh chụp.
    pub capture_exclusion_ok: bool,
    pub freeze: Option<FreezePhase>,
    /// Đang đếm ngược "Chụp đóng băng sau 3 giây".
    pub freeze_timer: bool,
}

impl Model {
    pub fn new(config: Config, paths: Paths, monitors: Vec<MonitorInfo>) -> Self {
        Self {
            config,
            paths,
            monitors,
            selecting: false,
            region: None,
            last_region: None,
            mode: FrameMode::Hidden,
            frame_margin: 0,
            stack: Stack::default(),
            hotkey_errors: BTreeMap::new(),
            capture_exclusion_ok: true,
            freeze: None,
            freeze_timer: false,
        }
    }

    fn monitor_of(&self, r: &Rect) -> Option<&MonitorInfo> {
        let (cx, cy) = r.center();
        monitor_at(&self.monitors, cx, cy)
    }

    // ---------- chọn vùng (SPEC 5.1) ----------

    pub fn start_selection(&mut self) -> Vec<Effect> {
        if self.selecting || self.freeze.is_some() {
            return vec![];
        }
        let mut fx = vec![];
        if self.mode != FrameMode::Hidden {
            fx.extend(self.close_region());
        }
        self.selecting = true;
        fx.push(Effect::ShowOverlays);
        fx
    }

    pub fn cancel_selection(&mut self) -> Vec<Effect> {
        self.selecting = false;
        vec![Effect::HideOverlays]
    }

    /// Overlay báo vùng đã chọn (physical px toàn desktop).
    pub fn select_region(&mut self, rect: Rect) -> Vec<Effect> {
        self.selecting = false;
        let mut fx = vec![Effect::HideOverlays];
        if rect.w >= 4 && rect.h >= 4 {
            fx.extend(self.open_region(rect));
        }
        fx
    }

    fn open_region(&mut self, rect: Rect) -> Vec<Effect> {
        let scale = self.monitor_of(&rect).map(|m| m.scale).unwrap_or(1.0);
        self.frame_margin = frame_margin(scale);
        self.region = Some(rect);
        self.last_region = Some(rect);
        self.mode = FrameMode::Normal;
        vec![Effect::ShowFrame, Effect::LayoutToolbar]
    }

    /// Rect của cửa sổ frame = region nới ra mỗi cạnh `frame_margin`.
    pub fn frame_outer(&self) -> Option<Rect> {
        self.region.map(|r| r.expand(self.frame_margin))
    }

    /// Người dùng kéo hoặc resize frame: `outer` là rect mới của cửa sổ frame.
    pub fn frame_moved(&mut self, outer: Rect) -> Vec<Effect> {
        if self.mode == FrameMode::Hidden {
            return vec![];
        }
        let inner = outer.expand(-self.frame_margin);
        if inner.is_empty() || Some(inner) == self.region {
            return vec![];
        }
        self.region = Some(inner);
        self.last_region = Some(inner);
        vec![Effect::LayoutToolbar]
    }

    pub fn toggle_fade(&mut self) -> Vec<Effect> {
        self.mode = match self.mode {
            FrameMode::Normal => FrameMode::Faded,
            FrameMode::Faded => FrameMode::Normal,
            FrameMode::Hidden => return vec![],
        };
        vec![Effect::UpdateFrameMode, Effect::LayoutToolbar]
    }

    pub fn close_region(&mut self) -> Vec<Effect> {
        if self.mode == FrameMode::Hidden {
            return vec![];
        }
        self.region = None;
        self.mode = FrameMode::Hidden;
        vec![Effect::HideFrame, Effect::HideToolbar]
    }

    /// Rect physical của toolbar cạnh region (SPEC 5.2). Kích thước theo DPI của màn hình chứa region.
    pub fn toolbar_rect(&self) -> Option<Rect> {
        let region = self.region?;
        let mon = self.monitor_of(&region).or(self.monitors.first())?;
        let dip = if self.mode == FrameMode::Faded { TOOLBAR_FADED_DIP } else { TOOLBAR_NORMAL_DIP };
        let size = ((dip.0 * mon.scale).ceil() as u32, (dip.1 * mon.scale).ceil() as u32);
        let gap = self.frame_margin + (TOOLBAR_GAP_DIP * mon.scale).round() as i32;
        let (x, y) = coords::place_toolbar(&region, size, &mon.work, gap);
        Some(Rect::new(x, y, size.0, size.1))
    }

    /// Scale của màn hình chứa region (để vẽ frame, toolbar).
    pub fn region_scale(&self) -> f64 {
        self.region
            .as_ref()
            .and_then(|r| self.monitor_of(r))
            .map(|m| m.scale)
            .unwrap_or(1.0)
    }

    // ---------- chụp (SPEC 6.8) ----------

    /// Hotkey Chụp hoặc nút Chụp: chụp region hiện tại, không có thì vùng cuối.
    pub fn request_capture(&mut self) -> Vec<Effect> {
        if self.freeze.is_some() {
            return vec![];
        }
        let Some(rect) = self.region.or(self.last_region) else { return vec![] };
        if self.monitor_of(&rect).is_none() {
            let mut fx = self.close_region();
            self.last_region = None;
            fx.push(Effect::Toast(Toast::error("Màn hình chứa vùng chụp không còn. Hãy chọn vùng mới.")));
            return fx;
        }
        let hide_ui = self.config.hide_ui_during_capture || !self.capture_exclusion_ok;
        vec![Effect::StartCapture(CaptureJob { rect, hide_ui })]
    }

    /// Đã lấy xong ảnh (chưa lưu). Normal mode thì đóng frame theo cấu hình.
    pub fn capture_grabbed(&mut self, result: Result<(), String>) -> Vec<Effect> {
        match result {
            Err(e) => vec![Effect::Toast(Toast::error(e))],
            Ok(()) if self.mode == FrameMode::Normal && self.config.after_capture_normal == AfterCapture::Close => {
                self.close_region()
            }
            Ok(()) => vec![],
        }
    }

    pub fn persist_options(&self) -> PersistOptions {
        PersistOptions {
            save_dir: self.config.save_dir.clone(),
            fallback_dir: self.paths.default_save_dir.clone(),
            pattern: self.config.file_name_pattern.clone(),
            copy_to_clipboard: self.config.copy_to_clipboard,
            thumbs_dir: self.paths.thumbs_dir.clone(),
        }
    }

    /// Đã lưu file (hoặc lỗi).
    pub fn capture_persisted(&mut self, result: Result<PersistOutcome, String>) -> Vec<Effect> {
        match result {
            Err(e) => vec![Effect::Toast(Toast::error(e))],
            Ok(out) => {
                let evicted = self.stack.push(out.shot);
                stack::drop_thumbnails(&evicted);
                let mut fx = vec![Effect::StackChanged];
                fx.extend(out.warnings.into_iter().map(|w| Effect::Toast(Toast::error(w))));
                fx
            }
        }
    }

    // ---------- Freeze (SPEC 14) ----------

    pub fn start_freeze(&mut self) -> Vec<Effect> {
        if self.freeze.is_some() || self.selecting {
            return vec![];
        }
        let mut fx = vec![];
        if std::mem::take(&mut self.freeze_timer) {
            fx.push(Effect::CancelFreezeTimer);
        }
        fx.extend(self.close_region());
        self.freeze = Some(FreezePhase::Grabbing);
        fx.push(Effect::FreezeGrab);
        fx
    }

    pub fn start_freeze_delayed(&mut self) -> Vec<Effect> {
        if self.freeze.is_some() || self.selecting {
            return vec![];
        }
        self.freeze_timer = true;
        vec![Effect::StartFreezeTimer]
    }

    pub fn freeze_timer_fired(&mut self) -> Vec<Effect> {
        if !self.freeze_timer {
            return vec![];
        }
        // start_freeze tự tắt hẹn giờ (CancelFreezeTimer đưa tray về icon thường).
        let fx = self.start_freeze();
        if fx.is_empty() {
            // Đang chọn vùng Live: bỏ lượt này, nhưng vẫn trả tray về bình thường.
            self.freeze_timer = false;
            return vec![Effect::CancelFreezeTimer];
        }
        fx
    }

    /// `any_ok`: có ít nhất 1 màn hình chụp được.
    pub fn freeze_grabbed(&mut self, any_ok: bool) -> Vec<Effect> {
        if self.freeze != Some(FreezePhase::Grabbing) {
            return vec![];
        }
        if any_ok {
            self.freeze = Some(FreezePhase::Selecting);
            vec![Effect::ShowFreezeOverlays]
        } else {
            self.freeze = None;
            vec![Effect::Toast(Toast::error("Không chụp được màn hình"))]
        }
    }

    /// Overlay xác nhận vùng (physical px toàn desktop).
    pub fn freeze_confirm(&mut self, rect: Rect, action: FreezeAction) -> Vec<Effect> {
        if self.freeze != Some(FreezePhase::Selecting) {
            return vec![];
        }
        self.freeze = None;
        let mut fx = vec![Effect::HideOverlays];
        if !rect.is_empty() {
            fx.push(match action {
                FreezeAction::Save => Effect::FreezePersist(rect),
                FreezeAction::Copy => Effect::FreezeCopy(rect),
            });
        }
        fx
    }

    pub fn freeze_cancel(&mut self) -> Vec<Effect> {
        match self.freeze.take() {
            Some(FreezePhase::Selecting) => vec![Effect::HideOverlays],
            _ => vec![],
        }
    }

    pub fn freeze_copied(&self, result: Result<(), String>) -> Vec<Effect> {
        match result {
            Ok(()) => vec![Effect::Toast(Toast::info("Đã copy ảnh vào clipboard"))],
            Err(e) => vec![Effect::Toast(Toast::error(e))],
        }
    }

    // ---------- stack (SPEC 5.4) ----------

    pub fn stack_remove(&mut self, id: &str) -> Vec<Effect> {
        match self.stack.remove(id) {
            Some(s) => {
                stack::drop_thumbnails(&[s]);
                vec![Effect::StackChanged]
            }
            None => vec![],
        }
    }

    pub fn stack_clear(&mut self) -> Vec<Effect> {
        let removed = self.stack.clear();
        stack::drop_thumbnails(&removed);
        vec![Effect::StackChanged]
    }

    pub fn shot(&self, id: &str) -> Option<&Shot> {
        self.stack.get(id)
    }

    /// Kiểm tra file của shot còn không; mất thì bỏ khỏi stack và báo (SPEC 7).
    pub fn ensure_shot_file(&mut self, id: &str) -> (Option<PathBuf>, Vec<Effect>) {
        let Some(path) = self.shot(id).map(|s| s.path.clone()) else { return (None, vec![]) };
        if path.is_file() {
            return (Some(path), vec![]);
        }
        let mut fx = self.stack_remove(id);
        fx.push(Effect::Toast(Toast::error(format!(
            "File {} không còn, đã bỏ khỏi stack",
            path.display()
        ))));
        (None, fx)
    }

    // ---------- màn hình (SPEC 7) ----------

    pub fn set_monitors(&mut self, mons: Vec<MonitorInfo>) -> Vec<Effect> {
        if mons == self.monitors || mons.is_empty() {
            return vec![];
        }
        self.monitors = mons;
        let mut fx = vec![];
        if self.selecting {
            fx.extend(self.cancel_selection());
        }
        // Huỷ cả khi đang chụp: ảnh có thể không còn khớp với màn hình.
        fx.extend(self.freeze_cancel());
        if let Some(r) = self.region {
            if self.monitor_of(&r).is_none() {
                fx.extend(self.close_region());
                self.last_region = None;
                fx.push(Effect::Toast(Toast::info("Màn hình chứa vùng chọn đã bị rút, vùng chọn đã đóng.")));
            } else {
                fx.push(Effect::LayoutToolbar);
            }
        }
        fx.push(Effect::StackChanged);
        fx
    }

    // ---------- cài đặt ----------

    /// Áp patch JSON từ màn hình Cài đặt, lưu file, trả về các việc kéo theo.
    pub fn update_config(&mut self, patch: &serde_json::Value) -> Result<Vec<Effect>, String> {
        let next = config::apply_patch(&self.config, patch)?;
        self.commit_config(next)
    }

    pub fn commit_config(&mut self, next: Config) -> Result<Vec<Effect>, String> {
        config::save(&self.paths.config_file, &next)?;
        let prev = std::mem::replace(&mut self.config, next);
        let mut fx = vec![];
        if prev.hotkeys != self.config.hotkeys {
            fx.push(Effect::ApplyHotkeys);
        }
        if prev.launch_at_startup != self.config.launch_at_startup {
            fx.push(Effect::SyncAutostart);
        }
        if prev.stack != self.config.stack {
            fx.push(Effect::StackChanged);
        }
        fx.push(Effect::ConfigChanged);
        Ok(fx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn mon(x: i32, w: u32, scale: f64) -> MonitorInfo {
        MonitorInfo {
            key: format!("m{x}"),
            rect: Rect::new(x, 0, w, 1080),
            work: Rect::new(x, 0, w, 1032),
            scale,
            primary: x == 0,
        }
    }

    fn model() -> Model {
        let dir = std::env::temp_dir().join(format!("chup-model-{}", ulid::Ulid::generate()));
        Model::new(
            Config::with_save_dir(&dir.join("pics")),
            Paths {
                config_file: dir.join("config.json"),
                thumbs_dir: dir.join("thumbs"),
                default_save_dir: dir.join("pics"),
            },
            vec![mon(0, 1920, 1.0), mon(1920, 2560, 1.5)],
        )
    }

    #[test]
    fn select_then_frame_and_toolbar() {
        let mut m = model();
        assert_eq!(m.start_selection(), vec![Effect::ShowOverlays]);
        assert!(m.start_selection().is_empty(), "đang chọn thì bỏ qua");
        let fx = m.select_region(Rect::new(2000, 100, 400, 300));
        assert_eq!(fx, vec![Effect::HideOverlays, Effect::ShowFrame, Effect::LayoutToolbar]);
        assert_eq!(m.frame_margin, 18, "màn hình 150%");
        assert_eq!(m.frame_outer(), Some(Rect::new(1982, 82, 436, 336)));
        let tb = m.toolbar_rect().unwrap();
        assert_eq!((tb.w, tb.h), (330, 66));
        assert!(tb.y >= 400 + 18, "toolbar nằm dưới frame");
    }

    #[test]
    fn tiny_selection_is_cancelled() {
        let mut m = model();
        m.start_selection();
        assert_eq!(m.select_region(Rect::new(10, 10, 3, 50)), vec![Effect::HideOverlays]);
        assert_eq!(m.mode, FrameMode::Hidden);
        assert!(!m.selecting);
    }

    #[test]
    fn frame_move_updates_region_and_last_region() {
        let mut m = model();
        m.select_region(Rect::new(100, 100, 200, 100));
        let outer = m.frame_outer().unwrap().offset(50, 20);
        assert_eq!(m.frame_moved(outer), vec![Effect::LayoutToolbar]);
        assert_eq!(m.region, Some(Rect::new(150, 120, 200, 100)));
        assert_eq!(m.last_region, m.region);
        assert!(m.frame_moved(outer).is_empty(), "không đổi thì không làm gì");
    }

    #[test]
    fn fade_and_capture_keeps_frame() {
        let mut m = model();
        m.select_region(Rect::new(100, 100, 200, 100));
        assert_eq!(m.toggle_fade(), vec![Effect::UpdateFrameMode, Effect::LayoutToolbar]);
        let tb = m.toolbar_rect().unwrap();
        assert_eq!((tb.w, tb.h), (116, 40));
        let fx = m.request_capture();
        assert!(matches!(fx[0], Effect::StartCapture(CaptureJob { rect, hide_ui: false }) if rect == Rect::new(100, 100, 200, 100)));
        assert!(m.capture_grabbed(Ok(())).is_empty(), "Faded: giữ frame");
        assert_eq!(m.mode, FrameMode::Faded);
    }

    #[test]
    fn normal_capture_closes_frame_and_recapture_uses_last_region() {
        let mut m = model();
        m.select_region(Rect::new(100, 100, 200, 100));
        m.request_capture();
        assert_eq!(m.capture_grabbed(Ok(())), vec![Effect::HideFrame, Effect::HideToolbar]);
        assert_eq!(m.region, None);
        let fx = m.request_capture();
        assert!(matches!(fx[0], Effect::StartCapture(j) if j.rect == Rect::new(100, 100, 200, 100)));
    }

    #[test]
    fn keep_setting_and_no_region() {
        let mut m = model();
        assert!(m.request_capture().is_empty(), "chưa có vùng nào thì bỏ qua");
        m.config.after_capture_normal = AfterCapture::Keep;
        m.select_region(Rect::new(100, 100, 200, 100));
        assert!(m.capture_grabbed(Ok(())).is_empty());
        assert_eq!(m.mode, FrameMode::Normal);
        assert_eq!(m.capture_grabbed(Err("x".into())), vec![Effect::Toast(Toast::error("x"))]);
    }

    #[test]
    fn hide_ui_when_exclusion_unsupported() {
        let mut m = model();
        m.capture_exclusion_ok = false;
        m.select_region(Rect::new(100, 100, 200, 100));
        assert!(matches!(m.request_capture()[0], Effect::StartCapture(CaptureJob { hide_ui: true, .. })));
    }

    #[test]
    fn unplugged_monitor_closes_region() {
        let mut m = model();
        m.select_region(Rect::new(2000, 100, 200, 100));
        let fx = m.set_monitors(vec![mon(0, 1920, 1.0)]);
        assert!(fx.contains(&Effect::HideFrame));
        assert!(fx.iter().any(|e| matches!(e, Effect::Toast(t) if t.kind == ToastKind::Info)));
        assert_eq!(m.last_region, None);
        assert!(m.request_capture().is_empty());
        assert!(m.set_monitors(vec![mon(0, 1920, 1.0)]).is_empty(), "không đổi");
    }

    #[test]
    fn new_selection_closes_old_frame() {
        let mut m = model();
        m.select_region(Rect::new(100, 100, 200, 100));
        assert_eq!(
            m.start_selection(),
            vec![Effect::HideFrame, Effect::HideToolbar, Effect::ShowOverlays]
        );
    }

    #[test]
    fn freeze_flow_save() {
        let mut m = model();
        m.select_region(Rect::new(100, 100, 200, 100));
        assert_eq!(m.start_freeze(), vec![Effect::HideFrame, Effect::HideToolbar, Effect::FreezeGrab]);
        assert!(m.start_freeze().is_empty(), "đang freeze thì bỏ qua F1");
        assert!(m.start_selection().is_empty(), "đang freeze thì bỏ qua Chọn vùng");
        assert!(m.request_capture().is_empty(), "đang freeze thì bỏ qua Chụp");
        assert_eq!(m.freeze_grabbed(true), vec![Effect::ShowFreezeOverlays]);
        let r = Rect::new(10, 10, 50, 40);
        assert_eq!(m.freeze_confirm(r, FreezeAction::Save), vec![Effect::HideOverlays, Effect::FreezePersist(r)]);
        assert_eq!(m.freeze, None);
        assert_eq!(m.last_region, Some(Rect::new(100, 100, 200, 100)), "Freeze không đổi vùng cuối");
    }

    #[test]
    fn freeze_copy_cancel_and_failure() {
        let mut m = model();
        m.start_freeze();
        m.freeze_grabbed(true);
        let r = Rect::new(0, 0, 8, 8);
        assert_eq!(m.freeze_confirm(r, FreezeAction::Copy), vec![Effect::HideOverlays, Effect::FreezeCopy(r)]);
        assert!(m.freeze_confirm(r, FreezeAction::Copy).is_empty(), "phiên đã kết thúc");

        m.start_freeze();
        m.freeze_grabbed(true);
        assert_eq!(m.freeze_cancel(), vec![Effect::HideOverlays]);

        m.start_freeze();
        assert_eq!(m.freeze_grabbed(false), vec![Effect::Toast(Toast::error("Không chụp được màn hình"))]);
        assert_eq!(m.freeze, None);

        // Huỷ lúc đang chụp: kết quả chụp về sau bị bỏ qua.
        m.start_freeze();
        assert!(m.freeze_cancel().is_empty());
        assert!(m.freeze_grabbed(true).is_empty());

        assert_eq!(m.freeze_copied(Ok(())), vec![Effect::Toast(Toast::info("Đã copy ảnh vào clipboard"))]);
    }

    #[test]
    fn freeze_timer() {
        let mut m = model();
        assert_eq!(m.start_freeze_delayed(), vec![Effect::StartFreezeTimer]);
        assert_eq!(m.start_freeze_delayed(), vec![Effect::StartFreezeTimer], "chọn lại = đếm lại");
        assert_eq!(m.freeze_timer_fired(), vec![Effect::CancelFreezeTimer, Effect::FreezeGrab]);
        assert!(m.freeze_timer_fired().is_empty(), "timer cũ bắn muộn thì bỏ qua");
        m.freeze_cancel();

        // F1 lúc đang đếm ngược: chụp ngay, huỷ hẹn giờ.
        m.start_freeze_delayed();
        assert_eq!(m.start_freeze(), vec![Effect::CancelFreezeTimer, Effect::FreezeGrab]);
        assert!(!m.freeze_timer);
        m.freeze_cancel();

        // Hết giờ đúng lúc đang chọn vùng Live: bỏ lượt chụp, tray về bình thường.
        m.start_freeze_delayed();
        m.start_selection();
        assert_eq!(m.freeze_timer_fired(), vec![Effect::CancelFreezeTimer]);
        assert!(!m.freeze_timer);
    }

    #[test]
    fn display_change_cancels_freeze() {
        let mut m = model();
        m.start_freeze();
        m.freeze_grabbed(true);
        let fx = m.set_monitors(vec![mon(0, 1920, 1.0)]);
        assert!(fx.contains(&Effect::HideOverlays));
        assert_eq!(m.freeze, None);
    }

    #[test]
    fn config_update_reports_side_effects() {
        let mut m = model();
        let fx = m
            .update_config(&serde_json::json!({ "hotkeys": { "capture": "F2" }, "launchAtStartup": true }))
            .unwrap();
        assert_eq!(fx, vec![Effect::ApplyHotkeys, Effect::SyncAutostart, Effect::ConfigChanged]);
        assert!(m.paths.config_file.is_file());
        assert!(m.update_config(&serde_json::json!({ "fileNamePattern": "" })).is_err());
        assert_eq!(m.config.hotkeys.capture, "F2");
    }
}
