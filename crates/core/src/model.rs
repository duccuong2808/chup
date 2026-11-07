//! State machine của app (SPEC 5, 6.6). Không biết gì về cửa sổ: nhận sự kiện từ lớp platform
//! và trả về danh sách `Effect` để lớp platform thực hiện. Nhờ vậy logic test được trên mọi OS.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    capture::{PersistOptions, PersistOutcome, Shot},
    config::{self, AfterCapture, Config, OcrLanguage},
    coords::{self, frame_margin, monitor_at, MonitorInfo, Rect},
    i18n::{fill, t},
    ocr::TextLayout,
    stack::{self, Stack},
};

/// Kích thước toolbar (CSS px = DIP) theo mode.
/// Normal: [Chụp] [Mờ] [T] [✕]; Faded: 4 icon.
pub const TOOLBAR_NORMAL_DIP: (f64, f64) = (252.0, 44.0);
pub const TOOLBAR_FADED_DIP: (f64, f64) = (148.0, 40.0);
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
    /// OCR vùng chọn, copy text (SPEC 15.2). Không lưu file, không vào stack.
    Ocr,
}

/// Ảnh cần OCR. Ảnh không đi qua `Model`: lớp platform giữ nó.
#[derive(Clone, Debug, PartialEq)]
pub enum OcrSource {
    /// Ảnh Preview đang xem (file trong stack). Kết quả được cache theo file.
    Preview(PathBuf),
    /// Ảnh lớp platform vừa crop / chụp (Freeze, Chụp text). `scale`: scale màn hình lúc chụp.
    Image { scale: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct OcrJob {
    pub id: u32,
    pub source: OcrSource,
    pub lang: OcrLanguage,
    /// Scale màn hình lúc chụp, để `ocr::prepare` biết có phóng ×2 không.
    pub scale: f64,
    /// `true`: người dùng yêu cầu (xong thì copy, nút [T] quay "bận"). `false`: job nền nhận diện chữ cho Preview.
    pub copy: bool,
}

/// OCR tiếng Việt dùng được chưa (lớp platform kiểm tra file và báo cho Model).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrViState {
    Ready,
    /// Còn thiếu dữ liệu tessdata hoặc engine `chup-ocr.exe` (bộ cài Lite, chạy exe rời): cần tải.
    NoPack,
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
    /// "Chụp text" (SPEC 15.2): chụp rồi OCR, không lưu file, không vào stack, không đóng vùng chọn.
    pub text: bool,
}

/// Việc lớp platform phải làm. Dữ liệu chi tiết (rect frame, rect toolbar...) đọc lại từ `Model`.
/// `PartialEq` (không `Eq`): `OcrJob` có scale kiểu f64.
#[derive(Clone, Debug, PartialEq)]
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
    /// Ngôn ngữ giao diện trong config đổi: platform chọn lại ngôn ngữ, dựng lại chữ của tray / Cài đặt.
    UiLanguageChanged,
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
    /// Mở cửa sổ Preview cho file ảnh này (SPEC 17).
    OpenPreview(PathBuf),
    /// Crop vùng từ ảnh đóng băng rồi gọi `ocr_request(OcrSource::Image)`.
    FreezeOcr(Rect),
    /// Giao job cho worker OCR (job mới thay job đang chạy/chờ).
    StartOcr(OcrJob),
    /// Huỷ job OCR đang chạy.
    CancelOcr,
    /// Đang OCR bật/tắt: Preview, toolbar vẽ lại nút [T] (spinner).
    OcrBusyChanged,
    /// Ghi text vào clipboard rồi báo (`ocr::copied_message`).
    CopyText(String),
    /// Chưa có gói tiếng Việt: hỏi người dùng có tải không.
    AskOcrDownload,
    /// Chữ nhận diện được của ảnh Preview này (để chọn chữ, SPEC 17.2).
    PreviewText(PathBuf, Arc<TextLayout>),
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
    /// Số thứ tự lần chụp Freeze gần nhất. Kết quả chụp mang số cũ (phiên đã huỷ, vd. vì đổi màn hình) thì bỏ.
    pub freeze_gen: u32,
    /// Đang đếm ngược "Chụp đóng băng sau 3 giây".
    pub freeze_timer: bool,
    /// Phiên Freeze hiện tại là "Chụp text": thả chuột là OCR luôn, không có toolbar.
    pub freeze_ocr: bool,
    pub ocr_vi: OcrViState,
    /// Job OCR đang chạy (cả job nền).
    pub ocr_busy: Option<OcrJob>,
    ocr_next_id: u32,
    /// Yêu cầu OCR đang chờ tải gói tiếng Việt xong.
    ocr_pending: Option<OcrSource>,
    /// Kết quả cuối: (file Preview, ngôn ngữ) -> chữ.
    ocr_cache: Option<(PathBuf, OcrLanguage, Arc<TextLayout>)>,
    /// File Preview đang mở (ảnh đã giải mã xong): để nhận diện nền và gửi chữ cho Preview.
    preview_open: Option<PathBuf>,
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
            freeze_gen: 0,
            freeze_timer: false,
            freeze_ocr: false,
            ocr_vi: OcrViState::NoPack,
            ocr_busy: None,
            ocr_next_id: 0,
            ocr_pending: None,
            ocr_cache: None,
            preview_open: None,
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
        self.capture_job(false)
    }

    /// Nút [T] trên toolbar Live: chụp vùng rồi OCR (SPEC 15.2).
    pub fn request_capture_text(&mut self) -> Vec<Effect> {
        self.capture_job(true)
    }

    fn capture_job(&mut self, text: bool) -> Vec<Effect> {
        if self.freeze.is_some() {
            return vec![];
        }
        let Some(rect) = self.region.or(self.last_region) else { return vec![] };
        if self.monitor_of(&rect).is_none() {
            let mut fx = self.close_region();
            self.last_region = None;
            fx.push(Effect::Toast(Toast::error(t().monitor_gone_choose_again)));
            return fx;
        }
        let hide_ui = self.config.hide_ui_during_capture || !self.capture_exclusion_ok;
        vec![Effect::StartCapture(CaptureJob { rect, hide_ui, text })]
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
            scale: self.region.or(self.last_region).map(|r| self.scale_at(&r)).unwrap_or(1.0),
        }
    }

    /// Scale của màn hình chứa tâm `r` (1,0 nếu không thuộc màn hình nào).
    pub fn scale_at(&self, r: &Rect) -> f64 {
        self.monitor_of(r).map(|m| m.scale).unwrap_or(1.0)
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
        self.start_freeze_mode(false)
    }

    /// Hotkey "Chụp text" (SPEC 15.2): overlay Freeze, chọn vùng xong là OCR luôn.
    pub fn start_freeze_ocr(&mut self) -> Vec<Effect> {
        self.start_freeze_mode(true)
    }

    fn start_freeze_mode(&mut self, ocr: bool) -> Vec<Effect> {
        if self.freeze.is_some() || self.selecting {
            return vec![];
        }
        self.freeze_ocr = ocr;
        let mut fx = vec![];
        if std::mem::take(&mut self.freeze_timer) {
            fx.push(Effect::CancelFreezeTimer);
        }
        fx.extend(self.close_region());
        self.freeze = Some(FreezePhase::Grabbing);
        self.freeze_gen = self.freeze_gen.wrapping_add(1);
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

    /// Kết quả chụp của lần `gen` (xem `freeze_gen`). `any_ok`: có ít nhất 1 màn hình chụp được.
    pub fn freeze_grabbed(&mut self, gen: u32, any_ok: bool) -> Vec<Effect> {
        if self.freeze != Some(FreezePhase::Grabbing) || gen != self.freeze_gen {
            return vec![];
        }
        if any_ok {
            self.freeze = Some(FreezePhase::Selecting);
            vec![Effect::ShowFreezeOverlays]
        } else {
            self.freeze = None;
            vec![Effect::Toast(Toast::error(t().capture_failed))]
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
                FreezeAction::Ocr => Effect::FreezeOcr(rect),
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
            Ok(()) => vec![Effect::Toast(Toast::info(t().image_copied_clipboard))],
            Err(e) => vec![Effect::Toast(Toast::error(e))],
        }
    }

    // ---------- OCR (SPEC 15) ----------

    /// Đang có job người dùng yêu cầu (nút [T] quay "bận"). Job nền không tính.
    fn ocr_visible_busy(&self) -> bool {
        self.ocr_busy.as_ref().is_some_and(|j| j.copy)
    }

    fn ocr_cached(&self, p: &Path, lang: OcrLanguage) -> Option<Arc<TextLayout>> {
        self.ocr_cache.as_ref().filter(|(cp, cl, _)| cp == p && *cl == lang).map(|(_, _, t)| t.clone())
    }

    fn ocr_copy(text: &TextLayout) -> Effect {
        if text.is_empty() {
            Effect::Toast(Toast::info(t().no_text_found))
        } else {
            Effect::CopyText(text.all_text())
        }
    }

    fn ocr_start(&mut self, source: OcrSource, lang: OcrLanguage, copy: bool) -> Vec<Effect> {
        let scale = match &source {
            OcrSource::Preview(p) => self.stack.iter().find(|s| &s.path == p).map(|s| s.scale).unwrap_or(1.0),
            OcrSource::Image { scale } => *scale,
        };
        self.ocr_next_id = self.ocr_next_id.wrapping_add(1);
        let job = OcrJob { id: self.ocr_next_id, source, lang, scale, copy };
        let was_busy = self.ocr_visible_busy();
        self.ocr_busy = Some(job.clone());
        let mut fx = vec![Effect::StartOcr(job)];
        if was_busy != self.ocr_visible_busy() {
            fx.push(Effect::OcrBusyChanged);
        }
        fx
    }

    /// Người dùng yêu cầu OCR (nút [T], Freeze, Chụp text).
    pub fn ocr_request(&mut self, source: OcrSource) -> Vec<Effect> {
        let lang = self.config.ocr.language;
        if let OcrSource::Preview(p) = &source {
            if let Some(text) = self.ocr_cached(p, lang) {
                return vec![Self::ocr_copy(&text)];
            }
            // Job nền của đúng ảnh này đang chạy: không chạy lại, chỉ cần copy khi xong.
            if let Some(j) = self.ocr_busy.as_mut().filter(|j| !j.copy && j.source == source && j.lang == lang) {
                j.copy = true;
                return vec![Effect::OcrBusyChanged];
            }
        }
        if lang == OcrLanguage::Vi {
            match self.ocr_vi {
                OcrViState::Ready => {}
                OcrViState::NoPack => {
                    self.ocr_pending = Some(source);
                    return vec![Effect::AskOcrDownload];
                }
            }
        }
        self.ocr_start(source, lang, true)
    }

    /// Job nền nhận diện chữ cho Preview đang mở, nếu cần và chạy được mà không phải hỏi ai
    /// (tiếng Việt thiếu gói hay `chup-ocr.exe` thì bỏ qua). Không thay job người dùng đang chạy.
    fn ocr_auto_preview(&mut self) -> Vec<Effect> {
        let lang = self.config.ocr.language;
        let Some(p) = self.preview_open.clone() else { return vec![] };
        let src = OcrSource::Preview(p.clone());
        let blocked = self.ocr_busy.as_ref().is_some_and(|j| j.copy || j.source == src);
        if !self.config.ocr.auto_preview
            || blocked
            || self.ocr_cached(&p, lang).is_some()
            || (lang == OcrLanguage::Vi && self.ocr_vi != OcrViState::Ready)
        {
            return vec![];
        }
        self.ocr_start(src, lang, false)
    }

    /// Preview vừa hiện ảnh `path` (đã giải mã xong).
    pub fn preview_shown(&mut self, path: PathBuf) -> Vec<Effect> {
        self.preview_open = Some(path.clone());
        match self.ocr_cached(&path, self.config.ocr.language) {
            Some(text) => {
                let mut fx = vec![Effect::PreviewText(path, text)];
                // Job nền của ảnh trước còn chạy: huỷ, không thì kết quả của nó đè cache của ảnh này.
                if self.ocr_busy.take_if(|j| !j.copy).is_some() {
                    fx.push(Effect::CancelOcr);
                }
                fx
            }
            None => self.ocr_auto_preview(),
        }
    }

    /// Preview đã đóng: huỷ job nền (job người dùng vẫn chạy tiếp, xong thì copy).
    pub fn preview_closed(&mut self) -> Vec<Effect> {
        self.preview_open = None;
        match self.ocr_busy.take_if(|j| !j.copy) {
            Some(_) => vec![Effect::CancelOcr],
            None => vec![],
        }
    }

    /// Worker OCR báo kết quả. Job đã bị thay hoặc huỷ thì bỏ qua.
    pub fn ocr_done(&mut self, id: u32, result: Result<TextLayout, String>) -> Vec<Effect> {
        let Some(job) = self.ocr_busy.take_if(|j| j.id == id) else { return vec![] };
        let mut fx = vec![];
        if job.copy {
            fx.push(Effect::OcrBusyChanged);
        }
        match result {
            // Job nền lỗi: không làm phiền người dùng (worker đã ghi log).
            Err(e) => {
                if job.copy {
                    fx.push(Effect::Toast(Toast::error(e)));
                }
            }
            Ok(text) => {
                let text = Arc::new(text);
                if let OcrSource::Preview(p) = &job.source {
                    self.ocr_cache = Some((p.clone(), job.lang, text.clone()));
                    if self.preview_open.as_ref() == Some(p) {
                        fx.push(Effect::PreviewText(p.clone(), text.clone()));
                    }
                }
                if job.copy {
                    fx.push(Self::ocr_copy(&text));
                }
            }
        }
        // Job khác vừa chiếm chỗ job nền: chạy lại job nền. Job của chính ảnh Preview thì không (lỗi sẽ lặp lại).
        if !matches!(&job.source, OcrSource::Preview(p) if self.preview_open.as_ref() == Some(p)) {
            fx.extend(self.ocr_auto_preview());
        }
        fx
    }

    /// Lớp platform đã dừng worker (tắt exe phụ để tải/xoá gói): job đang chạy sẽ không bao giờ báo kết quả.
    pub fn ocr_abort(&mut self) -> Vec<Effect> {
        match self.ocr_busy.take() {
            Some(j) if j.copy => vec![Effect::OcrBusyChanged],
            _ => vec![],
        }
    }

    /// Huỷ job đang chạy (bấm [T] lần nữa). Không chạy lại job nền.
    pub fn ocr_cancel(&mut self) -> Vec<Effect> {
        let visible = self.ocr_visible_busy();
        match self.ocr_busy.take() {
            Some(_) if visible => vec![Effect::CancelOcr, Effect::OcrBusyChanged],
            Some(_) => vec![Effect::CancelOcr],
            None => vec![],
        }
    }

    /// Lớp platform vừa kiểm tra lại file (khởi động, cài hoặc xoá gói).
    pub fn set_ocr_vi(&mut self, state: OcrViState) -> Vec<Effect> {
        if self.ocr_vi == state {
            return vec![];
        }
        self.ocr_vi = state;
        vec![Effect::ConfigChanged]
    }

    /// Tải gói tiếng Việt xong: chuyển sang tiếng Việt (người dùng vừa chủ động chọn) và chạy yêu cầu đang chờ.
    pub fn ocr_pack_installed(&mut self, state: OcrViState) -> Vec<Effect> {
        let mut fx = self.set_ocr_vi(state);
        if self.config.ocr.language != OcrLanguage::Vi {
            let mut next = self.config.clone();
            next.ocr.language = OcrLanguage::Vi;
            match self.commit_config(next) {
                Ok(more) => fx.extend(more),
                Err(e) => fx.push(Effect::Toast(Toast::error(e))),
            }
        }
        if let Some(src) = self.ocr_pending.take() {
            fx.extend(self.ocr_request(src));
        }
        fx.extend(self.ocr_auto_preview());
        fx
    }

    /// Có yêu cầu OCR đang chờ tải gói xong.
    pub fn ocr_has_pending(&self) -> bool {
        self.ocr_pending.is_some()
    }

    /// Không tải gói (từ chối, huỷ, lỗi): bỏ yêu cầu đang chờ.
    pub fn ocr_pack_aborted(&mut self) {
        self.ocr_pending = None;
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
        fx.push(Effect::Toast(Toast::error(fill(t().file_missing_removed, &[&path.display()]))));
        (None, fx)
    }

    /// Click thumbnail: mở Preview (SPEC 17). File mất thì bỏ khỏi stack và báo.
    pub fn shot_open(&mut self, id: &str) -> Vec<Effect> {
        let (path, mut fx) = self.ensure_shot_file(id);
        fx.extend(path.map(Effect::OpenPreview));
        fx
    }

    /// Người dùng vừa kéo hoặc đổi cỡ Preview: nhớ khung để lần sau mở đúng chỗ (chỉ khi chọn "như lần trước").
    pub fn set_preview_box(&mut self, r: Rect) -> Vec<Effect> {
        self.save_preview(r, None)
    }

    /// Người dùng kéo mép Preview xong: nhớ chỗ, và nhớ tỉ lệ so với cỡ tự tính để áp cho ảnh sau (SPEC 17.2).
    pub fn preview_resized(&mut self, r: Rect, img: (u32, u32), zoom: f64) -> Vec<Effect> {
        let (cx, cy) = r.center();
        let frac = self.config.preview.size;
        let learned = monitor_at(&self.monitors, cx, cy).and_then(|m| crate::preview::learned_scale(img, m.work, frac, zoom));
        self.save_preview(r, learned)
    }

    fn save_preview(&mut self, r: Rect, size_scale: Option<u16>) -> Vec<Effect> {
        if self.config.preview.position != config::PreviewPosition::Remember {
            return vec![];
        }
        let mut next = self.config.clone();
        next.preview.last = Some(config::SavedRect { x: r.x, y: r.y, w: r.w, h: r.h });
        next.preview.size_scale = size_scale.unwrap_or(next.preview.size_scale);
        if next == self.config {
            return vec![];
        }
        match self.commit_config(next) {
            Ok(fx) => fx,
            Err(e) => vec![Effect::Toast(Toast::error(e))],
        }
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
                fx.push(Effect::Toast(Toast::info(t().monitor_unplugged_closed)));
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
        if prev.ui_language != self.config.ui_language {
            fx.push(Effect::UiLanguageChanged);
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
        assert_eq!((tb.w, tb.h), (378, 66));
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
        assert_eq!((tb.w, tb.h), (148, 40));
        let fx = m.request_capture();
        assert!(matches!(fx[0], Effect::StartCapture(CaptureJob { rect, hide_ui: false, text: false }) if rect == Rect::new(100, 100, 200, 100)));
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
        assert_eq!(m.freeze_grabbed(m.freeze_gen, true), vec![Effect::ShowFreezeOverlays]);
        let r = Rect::new(10, 10, 50, 40);
        assert_eq!(m.freeze_confirm(r, FreezeAction::Save), vec![Effect::HideOverlays, Effect::FreezePersist(r)]);
        assert_eq!(m.freeze, None);
        assert_eq!(m.last_region, Some(Rect::new(100, 100, 200, 100)), "Freeze không đổi vùng cuối");
    }

    #[test]
    fn freeze_copy_cancel_and_failure() {
        let mut m = model();
        m.start_freeze();
        m.freeze_grabbed(m.freeze_gen, true);
        let r = Rect::new(0, 0, 8, 8);
        assert_eq!(m.freeze_confirm(r, FreezeAction::Copy), vec![Effect::HideOverlays, Effect::FreezeCopy(r)]);
        assert!(m.freeze_confirm(r, FreezeAction::Copy).is_empty(), "phiên đã kết thúc");

        m.start_freeze();
        m.freeze_grabbed(m.freeze_gen, true);
        assert_eq!(m.freeze_cancel(), vec![Effect::HideOverlays]);

        m.start_freeze();
        assert_eq!(m.freeze_grabbed(m.freeze_gen, false), vec![Effect::Toast(Toast::error(t().capture_failed))]);
        assert_eq!(m.freeze, None);

        // Huỷ lúc đang chụp: kết quả chụp về sau bị bỏ qua.
        m.start_freeze();
        assert!(m.freeze_cancel().is_empty());
        assert!(m.freeze_grabbed(m.freeze_gen, true).is_empty());

        assert_eq!(m.freeze_copied(Ok(())), vec![Effect::Toast(Toast::info(t().image_copied_clipboard))]);
    }

    #[test]
    fn freeze_ignores_result_of_cancelled_grab() {
        // F1, phiên bị huỷ khi còn đang chụp (vd. đổi màn hình), F1 lần nữa: ảnh của lần chụp đầu tới muộn
        // không được nhận vào phiên mới (hình học màn hình có thể đã khác).
        let mut m = model();
        m.start_freeze();
        let first = m.freeze_gen;
        assert!(m.freeze_cancel().is_empty());
        m.start_freeze();
        assert_ne!(m.freeze_gen, first);
        assert!(m.freeze_grabbed(first, true).is_empty());
        assert_eq!(m.freeze, Some(FreezePhase::Grabbing));
        assert_eq!(m.freeze_grabbed(m.freeze_gen, true), vec![Effect::ShowFreezeOverlays]);
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
        m.freeze_grabbed(m.freeze_gen, true);
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

    #[test]
    fn shot_open_previews_existing_file_or_drops_missing_one() {
        let mut m = model();
        let dir = std::env::temp_dir().join(format!("chup-shot-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let shot = |id: &str| crate::capture::Shot {
            id: id.into(),
            path: dir.join(format!("{id}.png")),
            thumb_path: dir.join(format!("t-{id}.png")),
            thumb: std::sync::Arc::new(image::RgbaImage::new(1, 1)),
            width: 1,
            height: 1,
            created_at: chrono::Local::now(),
            scale: 1.0,
        };
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        m.stack.push(shot("a"));
        m.stack.push(shot("b"));
        assert_eq!(m.shot_open("a"), vec![Effect::OpenPreview(dir.join("a.png"))]);
        let fx = m.shot_open("b");
        assert_eq!(fx[0], Effect::StackChanged);
        assert!(matches!(fx[1], Effect::Toast(_)));
        assert!(m.shot("b").is_none());
        assert!(m.shot_open("zzz").is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn shot_at(path: &std::path::Path, scale: f64) -> crate::capture::Shot {
        crate::capture::Shot {
            id: ulid::Ulid::generate().to_string(),
            path: path.to_path_buf(),
            thumb_path: path.with_extension("thumb"),
            thumb: std::sync::Arc::new(image::RgbaImage::new(1, 1)),
            width: 1,
            height: 1,
            created_at: chrono::Local::now(),
            scale,
        }
    }

    fn lay(text: &str) -> TextLayout {
        TextLayout::build(vec![crate::ocr::Line { text: text.into(), bbox: [0, 0, 100, 20], conf: 90.0, words: vec![] }], 1.0)
    }

    fn started(fx: &[Effect]) -> OcrJob {
        fx.iter()
            .find_map(|e| match e {
                Effect::StartOcr(j) => Some(j.clone()),
                _ => None,
            })
            .expect("phải có StartOcr")
    }

    #[test]
    fn ocr_english_runs_copies_and_caches_preview() {
        let mut m = model();
        let p = PathBuf::from("/pics/a.png");
        m.stack.push(shot_at(&p, 1.25));
        let fx = m.ocr_request(OcrSource::Preview(p.clone()));
        let job = started(&fx);
        assert_eq!(fx[1], Effect::OcrBusyChanged);
        assert_eq!((job.lang, job.scale), (OcrLanguage::En, 1.25), "scale lấy từ shot trong stack");
        assert_eq!(
            m.ocr_done(job.id, Ok(lay("Xin chào"))),
            vec![Effect::OcrBusyChanged, Effect::CopyText("Xin chào".into())]
        );
        assert!(m.ocr_busy.is_none());
        assert_eq!(m.ocr_request(OcrSource::Preview(p.clone())), vec![Effect::CopyText("Xin chào".into())], "cache");
        // Đổi ngôn ngữ thì không dùng cache cũ.
        m.config.ocr.language = OcrLanguage::Vi;
        m.ocr_vi = OcrViState::Ready;
        let job = started(&m.ocr_request(OcrSource::Preview(p)));
        assert_eq!(job.lang, OcrLanguage::Vi);
    }

    #[test]
    fn ocr_newer_job_replaces_older_and_cancel() {
        let mut m = model();
        let a = started(&m.ocr_request(OcrSource::Image { scale: 2.0 }));
        let fx = m.ocr_request(OcrSource::Image { scale: 1.0 });
        let b = started(&fx);
        assert_eq!(fx.len(), 1, "đã bận thì không đổi trạng thái spinner");
        assert_ne!(a.id, b.id);
        assert!(m.ocr_done(a.id, Ok(lay("cũ"))).is_empty(), "kết quả job cũ bị bỏ qua");
        assert_eq!(m.ocr_cancel(), vec![Effect::CancelOcr, Effect::OcrBusyChanged]);
        assert!(m.ocr_done(b.id, Ok(lay("muộn"))).is_empty(), "đã huỷ");
        assert!(m.ocr_cancel().is_empty());
    }

    #[test]
    fn ocr_empty_and_error_results() {
        let mut m = model();
        let j = started(&m.ocr_request(OcrSource::Image { scale: 1.0 }));
        assert_eq!(
            m.ocr_done(j.id, Ok(lay(" \n"))),
            vec![Effect::OcrBusyChanged, Effect::Toast(Toast::info(t().no_text_found))]
        );
        let j = started(&m.ocr_request(OcrSource::Image { scale: 1.0 }));
        assert_eq!(m.ocr_done(j.id, Err("hỏng".into()))[1], Effect::Toast(Toast::error("hỏng")));
    }

    #[test]
    fn ocr_vietnamese_without_pack_asks_then_runs_after_install() {
        let mut m = model();
        m.config.ocr.language = OcrLanguage::Vi;
        let src = OcrSource::Image { scale: 1.0 };
        assert_eq!(m.ocr_request(src.clone()), vec![Effect::AskOcrDownload]);
        assert!(m.ocr_busy.is_none());
        let fx = m.ocr_pack_installed(OcrViState::Ready);
        assert_eq!(fx[0], Effect::ConfigChanged);
        assert_eq!(started(&fx).source, src, "chạy luôn yêu cầu đang chờ");

        // Từ chối tải: lần cài sau không chạy lại yêu cầu cũ.
        let mut m = model();
        m.config.ocr.language = OcrLanguage::Vi;
        m.ocr_request(src);
        m.ocr_pack_aborted();
        assert!(!m.ocr_pack_installed(OcrViState::Ready).iter().any(|e| matches!(e, Effect::StartOcr(_))));
    }

    #[test]
    fn ocr_pack_installed_from_settings_switches_language() {
        let mut m = model();
        let fx = m.ocr_pack_installed(OcrViState::Ready);
        assert_eq!(m.config.ocr.language, OcrLanguage::Vi);
        assert!(fx.contains(&Effect::ConfigChanged));
        assert!(m.paths.config_file.is_file(), "đã lưu config");
        assert!(m.set_ocr_vi(OcrViState::Ready).is_empty());
    }

    #[test]
    fn preview_background_ocr_sends_text_without_spinner() {
        let mut m = model();
        let p = PathBuf::from("/pics/a.png");
        let fx = m.preview_shown(p.clone());
        let job = started(&fx);
        assert_eq!(fx.len(), 1, "job nền: không quay bận");
        assert!(!job.copy);
        let fx = m.ocr_done(job.id, Ok(lay("Xin chào")));
        assert!(matches!(&fx[..], [Effect::PreviewText(fp, t)] if *fp == p && t.all_text() == "Xin chào"));
        // Mở lại cùng ảnh: lấy cache. [T]: copy luôn.
        assert!(matches!(&m.preview_shown(p.clone())[..], [Effect::PreviewText(..)]));
        assert_eq!(m.ocr_request(OcrSource::Preview(p)), vec![Effect::CopyText("Xin chào".into())]);
    }

    #[test]
    fn preview_button_upgrades_background_job() {
        let mut m = model();
        let p = PathBuf::from("/pics/a.png");
        let job = started(&m.preview_shown(p.clone()));
        assert_eq!(m.ocr_request(OcrSource::Preview(p.clone())), vec![Effect::OcrBusyChanged], "không chạy lại");
        let fx = m.ocr_done(job.id, Ok(lay("abc")));
        assert_eq!(fx[0], Effect::OcrBusyChanged);
        assert!(matches!(fx[1], Effect::PreviewText(..)));
        assert_eq!(fx[2], Effect::CopyText("abc".into()));
        // Ảnh trống: cache rỗng, [T] báo không có chữ.
        let q = PathBuf::from("/pics/b.png");
        let job = started(&m.preview_shown(q.clone()));
        m.ocr_done(job.id, Ok(lay(" ")));
        assert_eq!(m.ocr_request(OcrSource::Preview(q)), vec![Effect::Toast(Toast::info(t().no_text_found))]);
    }

    #[test]
    fn user_job_replaces_background_job_which_restarts_after() {
        let mut m = model();
        let p = PathBuf::from("/pics/a.png");
        let bg = started(&m.preview_shown(p.clone()));
        let fx = m.ocr_request(OcrSource::Image { scale: 1.0 });
        let user = started(&fx);
        assert!(fx.contains(&Effect::OcrBusyChanged), "job người dùng: quay bận");
        assert!(m.ocr_done(bg.id, Ok(lay("cũ"))).is_empty(), "job nền đã bị thay");
        let fx = m.ocr_done(user.id, Ok(lay("chữ")));
        assert_eq!(fx[..2], [Effect::OcrBusyChanged, Effect::CopyText("chữ".into())]);
        let again = started(&fx);
        assert_eq!((again.source.clone(), again.copy), (OcrSource::Preview(p.clone()), false), "chạy lại job nền");
        // Mở ảnh khác lúc job nền đang chạy: thay. Đóng Preview: huỷ job nền.
        let q = PathBuf::from("/pics/b.png");
        assert_eq!(started(&m.preview_shown(q)).source, OcrSource::Preview(PathBuf::from("/pics/b.png")));
        assert_eq!(m.preview_closed(), vec![Effect::CancelOcr]);
        assert!(m.ocr_busy.is_none());
        assert!(m.preview_closed().is_empty());
    }

    #[test]
    fn background_job_never_asks_and_respects_setting() {
        let mut m = model();
        m.config.ocr.language = OcrLanguage::Vi;
        assert!(m.preview_shown(PathBuf::from("/a.png")).is_empty(), "thiếu gói: không hỏi");
        let mut m = model();
        m.config.ocr.auto_preview = false;
        assert!(m.preview_shown(PathBuf::from("/a.png")).is_empty(), "tắt tự nhận diện");
        // Job nền lỗi: không toast, không chạy lại.
        let mut m = model();
        let job = started(&m.preview_shown(PathBuf::from("/a.png")));
        assert!(m.ocr_done(job.id, Err("hỏng".into())).is_empty());
        // Đang có job người dùng: mở Preview không chen vào; người dùng huỷ [T] thì không chạy lại.
        let mut m = model();
        m.ocr_request(OcrSource::Image { scale: 1.0 });
        assert!(m.preview_shown(PathBuf::from("/a.png")).is_empty());
        assert_eq!(m.ocr_cancel(), vec![Effect::CancelOcr, Effect::OcrBusyChanged]);
    }

    #[test]
    fn abort_clears_job_and_cached_preview_cancels_other_background_job() {
        let mut m = model();
        let a = PathBuf::from("/pics/a.png");
        let job = started(&m.preview_shown(a.clone()));
        assert!(m.ocr_abort().is_empty(), "job nền: không có spinner để tắt");
        assert!(m.ocr_busy.is_none());
        assert!(m.ocr_done(job.id, Ok(lay("muộn"))).is_empty());
        m.ocr_request(OcrSource::Image { scale: 1.0 });
        assert_eq!(m.ocr_abort(), vec![Effect::OcrBusyChanged]);
        // b đã có trong cache; mở a (job nền chạy) rồi quay lại b: huỷ job của a.
        let b = PathBuf::from("/pics/b.png");
        let jb = started(&m.preview_shown(b.clone()));
        m.ocr_done(jb.id, Ok(lay("bê")));
        started(&m.preview_shown(a));
        let fx = m.preview_shown(b);
        assert!(matches!(fx[0], Effect::PreviewText(..)));
        assert_eq!(fx[1], Effect::CancelOcr);
        assert!(m.ocr_busy.is_none());
    }

    #[test]
    fn pack_installed_starts_background_job_for_open_preview() {
        let mut m = model();
        m.config.ocr.language = OcrLanguage::Vi;
        m.preview_shown(PathBuf::from("/a.png"));
        let job = started(&m.ocr_pack_installed(OcrViState::Ready));
        assert!(!job.copy);
    }

    #[test]
    fn capture_text_and_freeze_ocr_mode() {
        let mut m = model();
        m.select_region(Rect::new(100, 100, 200, 100));
        assert!(matches!(m.request_capture_text()[0], Effect::StartCapture(CaptureJob { text: true, .. })));
        assert_eq!(m.start_freeze_ocr(), vec![Effect::HideFrame, Effect::HideToolbar, Effect::FreezeGrab]);
        assert!(m.freeze_ocr);
        assert!(m.start_freeze().is_empty(), "đang freeze: không đổi chế độ");
        assert!(m.freeze_ocr);
        m.freeze_cancel();
        m.start_freeze();
        assert!(!m.freeze_ocr, "Freeze thường");
    }

    #[test]
    fn freeze_ocr_confirm() {
        let mut m = model();
        m.start_freeze();
        m.freeze_grabbed(m.freeze_gen, true);
        let r = Rect::new(1, 2, 30, 40);
        assert_eq!(m.freeze_confirm(r, FreezeAction::Ocr), vec![Effect::HideOverlays, Effect::FreezeOcr(r)]);
    }

    #[test]
    fn preview_move_keeps_size_scale_but_resize_learns_it() {
        let mut m = model();
        let work = m.monitors[0].work;
        let img = (work.w * 2, work.h * 2);
        let base = crate::preview::auto_zoom(img, work, m.config.preview.size, 100);
        let r = Rect::new(work.x + 10, work.y + 10, 400, 300);
        assert_eq!(m.preview_resized(r, img, base * 1.5), vec![Effect::ConfigChanged]);
        assert_eq!(m.config.preview.size_scale, 150);
        assert_eq!(m.config.preview.last, Some(config::SavedRect { x: r.x, y: r.y, w: 400, h: 300 }));
        // Di chuyển chỉ đổi chỗ.
        m.set_preview_box(r.offset(5, 5));
        assert_eq!(m.config.preview.size_scale, 150);
        // Ảnh đang 1:1: kéo mép chỉ nhớ chỗ, không học.
        m.preview_resized(r, (100, 100), 0.5);
        assert_eq!(m.config.preview.size_scale, 150);
        assert_eq!(m.config.preview.last, Some(config::SavedRect { x: r.x, y: r.y, w: 400, h: 300 }));
        // Không ở chế độ nhớ thì không ghi gì.
        m.config.preview.position = config::PreviewPosition::Center;
        assert!(m.preview_resized(r, img, base * 0.5).is_empty());
        assert_eq!(m.config.preview.size_scale, 150);
    }

    #[test]
    fn preview_box_is_saved_only_when_remembering() {
        let mut m = model();
        let r = Rect::new(-500, 10, 400, 300);
        assert_eq!(m.set_preview_box(r), vec![Effect::ConfigChanged]);
        assert_eq!(m.config.preview.last, Some(config::SavedRect { x: -500, y: 10, w: 400, h: 300 }));
        assert!(m.set_preview_box(r).is_empty());
        m.config.preview.position = config::PreviewPosition::Center;
        assert!(m.set_preview_box(Rect::new(0, 0, 10, 10)).is_empty());
    }
}
