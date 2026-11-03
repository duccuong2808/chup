//! Điều phối: nhận `Action` (từ cửa sổ, tray, hotkey, worker), gọi `Model`, áp `Effect` lên cửa sổ.
//! Chụp và lưu file chạy ở worker thread; kết quả quay về bằng `emit`.

use std::{rc::Rc, sync::Arc, time::Duration};

use chup_core::{
    autoselect::{Frame as AutoFrame, WinFrame},
    capture::{self, MonitorShot, PersistOutcome},
    coords::{monitor_at, Rect},
    config::{Edge, StackMonitor},
    model::{Effect, FrameMode, FreezeAction, Model, OcrJob, OcrSource, Toast, FREEZE_DELAY_MS},
    ocr::pack,
};
use image::RgbaImage;
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, PostQuitMessage, SetTimer, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
};

use super::{
    autoselect,
    frame::Frame,
    hotkeys::{Hotkeys, Kind},
    ocr::{self, Worker},
    ocr_download::Outcome,
    overlay::Overlay,
    preview::Preview,
    settings::{Settings, SettingsView},
    stack::StackWin,
    toolbar::Toolbar,
    tray::Tray,
    util::{self, cursor_pos},
    win::{self, emit},
};

pub enum Action {
    StartSelection,
    CancelSelection,
    RegionSelected(Rect),
    FrameMoved(Rect),
    ToggleFade,
    CloseRegion,
    Capture,
    Grabbed(Result<RgbaImage, String>),
    /// Nút [T] trên toolbar Live.
    CaptureText,
    /// Ảnh của "Chụp text" (vùng đã chụp, để biết scale).
    TextGrabbed(Result<RgbaImage, String>, Rect),
    Persisted(Result<PersistOutcome, String>),
    StackRemove(String),
    StackClear,
    ShotOpen(String),
    ShotReveal(String),
    ShotCopy(String),
    ShotMissing(String),
    Toast(Toast),
    Menu(String),
    Hotkey(u32),
    OpenSettings,
    SettingsClosed,
    ConfigPatch(serde_json::Value),
    PauseHotkeys,
    ResumeHotkeys,
    OpenSaveDir,
    DisplayChanged,
    FreezeTimerFired,
    /// Mục tray "Chụp đóng băng", sau khi chờ menu tray tắt hẳn.
    FreezeNow,
    FreezeGrabbed(Result<Vec<MonitorShot>, String>),
    /// Overlay (HWND) vừa bắt đầu chọn vùng mới: các overlay khác bỏ vùng của mình.
    FreezeSelecting(isize),
    FreezeConfirm { rect: Rect, action: FreezeAction },
    FreezeCancel,
    FreezeCopied(Result<(), String>),
    /// Overlay: chuột ở điểm `p` (desktop) trên cửa sổ `win`; hỏi worker chuỗi khung (SPEC 16).
    AutoHover { p: (i32, i32), win: WinFrame, mon: Rect },
    /// Worker: chuỗi khung thô cho điểm `p` của phiên `gen`.
    AutoFrames { gen: u32, p: (i32, i32), window: isize, chain: Vec<AutoFrame> },
    /// Cuộn chuột / ↑↓ (dương = lên khung cha); `0` = nhấn / nhả Shift, chỉ vẽ lại.
    AutoLevel(i32),
    /// Ảnh cho Preview đã giải mã xong (worker).
    PreviewLoaded(std::path::PathBuf, Result<RgbaImage, String>),
    PreviewCopy(std::path::PathBuf),
    PreviewReveal(std::path::PathBuf),
    /// Preview vừa đóng: huỷ job OCR nền, hẹn lại trim working set.
    PreviewClosed,
    /// Người dùng vừa kéo, đổi cỡ hoặc zoom Preview: khung mới.
    PreviewBox(Rect),
    /// Nút [T] trên Preview: OCR ảnh đang xem, hoặc huỷ nếu đang OCR nó.
    PreviewOcr(std::path::PathBuf),
    /// Worker OCR xong job (chữ đã sắp theo thứ tự đọc, hoặc câu lỗi cho người dùng).
    OcrDone(u32, Result<chup_core::ocr::TextLayout, String>),
    /// Copy chữ người dùng vừa chọn trên Preview.
    CopyText(String),
    /// Nút "Tải gói" trong Cài đặt.
    OcrDownload,
    /// Nút "Xoá gói" trong Cài đặt.
    OcrRemove,
    /// Hộp thoại tải gói đã đóng.
    OcrPack(Outcome),
}

/// Id timer hẹn giờ "Chụp đóng băng sau 3 giây" trên cửa sổ chính (mod.rs bắn `FreezeTimerFired`).
pub const TIMER_FREEZE: usize = 5;
/// Mục tray "Chụp đóng băng": chờ menu tray tắt hẳn (hiệu ứng mờ dần của mục vừa chọn ~200–350 ms)
/// rồi mới chụp, để menu không lọt vào ảnh đóng băng.
pub const TIMER_FREEZE_MENU: usize = 6;
const FREEZE_MENU_DELAY_MS: u32 = 350;

pub struct App {
    pub model: Model,
    overlays: Vec<Rc<Overlay>>,
    frame: Rc<Frame>,
    toolbar: Rc<Toolbar>,
    stack: Rc<StackWin>,
    preview: Rc<Preview>,
    settings: Option<Rc<Settings>>,
    tray: Option<Tray>,
    hotkeys: Option<Hotkeys>,
    /// Màn hình stack đang đứng; chốt lúc stack bắt đầu hiện để nó không nhảy theo chuột.
    stack_monitor: Option<String>,
    ui_hidden_for_capture: bool,
    /// Ảnh Freeze vừa chụp, chờ `ShowFreezeOverlays` chia cho từng overlay.
    frozen: Vec<MonitorShot>,
    /// Ảnh của màn hình chứa vùng vừa xác nhận, chờ `FreezePersist`/`FreezeCopy`.
    picked: Option<(Rect, Arc<RgbaImage>)>,
    /// Worker tự động chọn vùng (SPEC 16).
    auto: autoselect::Worker,
    /// Thế hệ phiên tự chọn vùng hiện tại; `auto_on` = đang có phiên (overlay đang hiện).
    auto_gen: u32,
    auto_on: bool,
    /// Danh sách cửa sổ chụp lúc bắt đầu Freeze (trước khi overlay lấy focus làm menu đóng).
    freeze_wins: Option<Rc<Vec<WinFrame>>>,
    /// Overlay đã được loại khỏi ảnh chụp: Live mới chụp ngầm để nhận khung từ ảnh.
    overlay_excluded: bool,
    /// Lúc bắt đầu chụp Freeze, để log thời gian tới lúc ảnh hiện (ngân sách 250 ms, SPEC 14.6).
    freeze_t0: Option<std::time::Instant>,
    ocr: Worker,
    /// `%LOCALAPPDATA%\app.chup.desktop\ocr`: gói dữ liệu tiếng Việt.
    ocr_root: std::path::PathBuf,
    /// Ảnh cho `OcrSource::Image` (crop từ Freeze, Chụp text), giữ tới khi job bắt đầu (kể cả lúc chờ tải gói).
    ocr_image: Option<RgbaImage>,
}

impl App {
    pub fn new(model: Model, ocr_root: std::path::PathBuf) -> windows::core::Result<App> {
        let frame = Frame::create()?;
        let toolbar = Toolbar::create()?;
        let stack = StackWin::create()?;
        let preview = Preview::create()?;
        let mut app = App {
            model,
            overlays: vec![],
            frame,
            toolbar,
            stack,
            preview,
            settings: None,
            tray: None,
            hotkeys: None,
            stack_monitor: None,
            ui_hidden_for_capture: false,
            frozen: vec![],
            picked: None,
            freeze_t0: None,
            ocr: Worker::start(ocr_root.clone()),
            ocr_root,
            ocr_image: None,
            auto: autoselect::Worker::start(),
            auto_gen: 0,
            auto_on: false,
            freeze_wins: None,
            overlay_excluded: false,
        };
        pack::cleanup_old(&app.ocr_root);
        let st = ocr::vi_state(&app.ocr_root);
        app.model.set_ocr_vi(st);

        // SPEC 6.4: loại frame/toolbar/stack khỏi ảnh chụp (Windows 10 bản 2004 trở lên).
        let ok = [app.frame.hwnd, app.toolbar.hwnd, app.stack.hwnd]
            .iter()
            .all(|h| unsafe { SetWindowDisplayAffinity(*h, WDA_EXCLUDEFROMCAPTURE) }.is_ok());
        if !ok {
            log::warn!("SetWindowDisplayAffinity không được hỗ trợ: dùng fallback ẩn UI lúc chụp");
        }
        app.model.capture_exclusion_ok = ok;

        app.sync_overlays();
        match Tray::create() {
            Ok(t) => app.tray = Some(t),
            Err(e) => log::error!("{e}"),
        }
        match Hotkeys::new() {
            Ok(h) => app.hotkeys = Some(h),
            Err(e) => log::error!("{e}"),
        }
        app.apply_hotkeys();
        app.sync_autostart();
        Ok(app)
    }

    // ---------- Action ----------

    pub fn handle(&mut self, a: Action) {
        let fx = match a {
            Action::StartSelection => self.model.start_selection(),
            Action::CancelSelection => self.model.cancel_selection(),
            Action::RegionSelected(r) => self.model.select_region(r),
            Action::FrameMoved(outer) => self.model.frame_moved(outer),
            Action::ToggleFade => self.model.toggle_fade(),
            Action::CloseRegion => self.model.close_region(),
            Action::Capture => self.model.request_capture(),
            Action::Grabbed(res) => self.on_grabbed(res),
            Action::CaptureText => self.model.request_capture_text(),
            Action::TextGrabbed(res, rect) => {
                self.restore_ui_after_capture();
                match res {
                    Err(e) => vec![Effect::Toast(Toast::error(e))],
                    Ok(img) => {
                        self.ocr_image = Some(img);
                        self.model.ocr_request(OcrSource::Image { scale: self.model.scale_at(&rect) })
                    }
                }
            }
            Action::Persisted(res) => self.model.capture_persisted(res),
            Action::StackRemove(id) => self.model.stack_remove(&id),
            Action::StackClear => self.model.stack_clear(),
            Action::ShotOpen(id) => self.model.shot_open(&id),
            Action::ShotReveal(id) => self.with_shot_file(&id, util::shell_reveal),
            Action::ShotCopy(id) => {
                let (path, fx) = self.model.ensure_shot_file(&id);
                if let Some(p) = path {
                    std::thread::spawn(move || {
                        if let Err(e) = chup_core::clipboard::copy_file_image(&p) {
                            emit(Action::Toast(Toast::error(e)));
                        }
                    });
                }
                fx
            }
            Action::ShotMissing(id) => self.model.ensure_shot_file(&id).1,
            Action::Toast(t) => vec![Effect::Toast(t)],
            Action::Menu(id) => match id.as_str() {
                "select" => self.model.start_selection(),
                "recapture" => self.model.request_capture(),
                "freeze" => {
                    unsafe { SetTimer(Some(win::app_hwnd()), TIMER_FREEZE_MENU, FREEZE_MENU_DELAY_MS, None) };
                    vec![]
                }
                "freeze_delayed" => self.model.start_freeze_delayed(),
                "open_folder" => self.open_save_dir(),
                "settings" => {
                    self.open_settings();
                    vec![]
                }
                "quit" => {
                    unsafe { PostQuitMessage(0) };
                    vec![]
                }
                _ => vec![],
            },
            Action::Hotkey(id) => match self.hotkeys.as_ref().and_then(|h| h.kind_of(id)) {
                Some(Kind::SelectRegion) => self.model.start_selection(),
                Some(Kind::Capture) => self.model.request_capture(),
                Some(Kind::Freeze) => self.model.start_freeze(),
                Some(Kind::Ocr) => self.model.start_freeze_ocr(),
                None => vec![],
            },
            Action::OpenSettings => {
                self.open_settings();
                vec![]
            }
            Action::SettingsClosed => {
                self.settings = None;
                // Đang ghi phím mà đóng cửa sổ: bật lại hotkey.
                self.apply_hotkeys();
                vec![]
            }
            Action::ConfigPatch(p) => match self.model.update_config(&p) {
                Ok(fx) => {
                    if let Some(s) = &self.settings {
                        s.set_status("Đã lưu", false);
                    }
                    fx
                }
                Err(e) => {
                    if let Some(s) = &self.settings {
                        s.set_status(&e, true);
                    }
                    vec![Effect::ConfigChanged]
                }
            },
            Action::PauseHotkeys => {
                if let Some(h) = self.hotkeys.as_mut() {
                    h.pause();
                }
                vec![]
            }
            Action::ResumeHotkeys => {
                self.apply_hotkeys();
                vec![Effect::ConfigChanged]
            }
            Action::OpenSaveDir => self.open_save_dir(),
            Action::FreezeTimerFired => self.model.freeze_timer_fired(),
            Action::FreezeNow => self.model.start_freeze(),
            Action::FreezeGrabbed(res) => {
                let shots = res.unwrap_or_else(|e| {
                    log::error!("freeze: {e}");
                    vec![]
                });
                for (r, e) in shots.iter().filter_map(|(r, i)| i.as_ref().err().map(|e| (r, e))) {
                    log::warn!("freeze: màn hình {r:?}: {e}");
                }
                let ok = shots.iter().any(|(_, i)| i.is_ok());
                self.frozen = shots;
                self.model.freeze_grabbed(ok)
            }
            Action::FreezeSelecting(h) => {
                for o in self.overlays.iter().filter(|o| o.hwnd.0 as isize != h) {
                    o.clear_frozen_selection();
                }
                vec![]
            }
            Action::FreezeConfirm { rect, action } => {
                // Lấy ảnh trước khi HideOverlays giải phóng nó.
                let (cx, cy) = rect.center();
                self.picked = self
                    .overlays
                    .iter()
                    .find(|o| o.mon.rect.contains_point(cx, cy))
                    .and_then(|o| o.take_frozen().map(|img| (o.mon.rect, img)));
                self.model.freeze_confirm(rect, action)
            }
            Action::FreezeCancel => self.model.freeze_cancel(),
            Action::FreezeCopied(r) => self.model.freeze_copied(r),
            Action::AutoHover { p, win, mon } => {
                if self.auto_on {
                    self.auto.query(self.auto_gen, p, win, mon);
                    // Chuột vừa sang màn hình này: overlay màn hình kia không nhận WM_MOUSEMOVE nữa, tự xoá khung.
                    for o in self.overlays.iter().filter(|o| o.mon.rect != mon) {
                        o.auto_clear();
                    }
                }
                vec![]
            }
            Action::AutoFrames { gen, p, window, chain } => {
                if self.auto_on && gen == self.auto_gen {
                    if let Some(o) = self.overlays.iter().find(|o| o.mon.rect.contains_point(p.0, p.1)) {
                        o.auto_result(p, window, &chain);
                    }
                }
                vec![]
            }
            Action::AutoLevel(steps) => {
                let (cx, cy) = cursor_pos();
                if let Some(o) = self.overlays.iter().find(|o| o.mon.rect.contains_point(cx, cy)) {
                    o.auto_level(steps);
                }
                vec![]
            }
            Action::PreviewLoaded(path, res) => self.on_preview_loaded(path, res),
            Action::PreviewCopy(path) => {
                std::thread::spawn(move || {
                    let t = match chup_core::clipboard::copy_file_image(&path) {
                        Ok(()) => Toast::info("Đã copy ảnh"),
                        Err(e) => Toast::error(e),
                    };
                    emit(Action::Toast(t));
                });
                vec![]
            }
            Action::PreviewReveal(path) => match util::shell_reveal(&path) {
                Ok(()) => vec![],
                Err(e) => vec![Effect::Toast(Toast::error(e))],
            },
            Action::PreviewClosed => self.model.preview_closed(),
            Action::PreviewBox(r) => self.model.set_preview_box(r),
            Action::PreviewOcr(path) => match &self.model.ocr_busy {
                Some(OcrJob { source: OcrSource::Preview(p), copy: true, .. }) if *p == path => self.model.ocr_cancel(),
                _ => self.model.ocr_request(OcrSource::Preview(path)),
            },
            Action::OcrDone(id, r) => self.model.ocr_done(id, r),
            Action::CopyText(text) => vec![Effect::CopyText(text)],
            Action::OcrDownload => {
                // Exe phụ còn sống (gói vừa hỏng/xoá dở) giữ thư mục gói: tắt trước để bước rename khi cài không lỗi.
                self.ocr.stop_helper();
                post_ocr_download(false);
                // Job đang chạy đã bị dừng, sẽ không báo kết quả.
                self.model.ocr_abort()
            }
            Action::OcrRemove => {
                self.ocr.stop_helper();
                let mut fx = self.model.ocr_abort();
                fx.push(match pack::remove(&self.ocr_root) {
                    Ok(()) => Effect::Toast(Toast::info("Đã xoá gói OCR tiếng Việt")),
                    Err(e) => Effect::Toast(Toast::error(e)),
                });
                fx.extend(self.model.set_ocr_vi(ocr::vi_state(&self.ocr_root)));
                fx
            }
            Action::OcrPack(out) => {
                if out == Outcome::Installed {
                    let had_pending = self.model.ocr_has_pending();
                    let mut fx = self.model.ocr_pack_installed(ocr::vi_state(&self.ocr_root));
                    if !had_pending {
                        fx.push(Effect::Toast(Toast::info("Đã tải gói OCR tiếng Việt")));
                    }
                    fx
                } else {
                    self.model.ocr_pack_aborted();
                    self.ocr_image = None;
                    vec![]
                }
            }
            Action::DisplayChanged => {
                let fx = self.model.set_monitors(util::monitors());
                self.sync_overlays();
                fx
            }
        };
        self.apply(fx);
        // Ảnh Freeze không được dùng tới (phiên đã huỷ, lỗi) thì bỏ ngay, không giữ vài chục MB.
        self.frozen.clear();
        self.picked = None;
    }

    /// App đang rảnh: không chọn vùng, Cài đặt đã đóng (frame có thể đang hiện, nhất là ở chế độ Mờ).
    pub fn is_idle(&self) -> bool {
        !self.model.selecting
            && self.model.freeze.is_none()
            && !self.model.freeze_timer
            && self.settings.is_none()
            && !self.preview.is_visible()
    }

    /// Rảnh đủ lâu: giải phóng DIB của overlay và trả working set lại cho Windows.
    /// Trang nhớ chỉ bị đưa ra khỏi RAM (không mất dữ liệu); lần dùng sau nạp lại rất nhanh.
    pub fn trim(&self) {
        for o in &self.overlays {
            o.release();
        }
        unsafe {
            // Gom và trả phần heap trống (sau khi encode PNG ảnh lớn heap giữ lại vài chục MB).
            if let Ok(heap) = windows::Win32::System::Memory::GetProcessHeap() {
                let _ = windows::Win32::System::Memory::HeapCompact(heap, windows::Win32::System::Memory::HEAP_FLAGS(0));
            }
            let _ = windows::Win32::System::ProcessStatus::EmptyWorkingSet(
                windows::Win32::System::Threading::GetCurrentProcess(),
            );
        }
        log::debug!("đã trim working set");
    }

    fn with_shot_file(&mut self, id: &str, f: impl FnOnce(&std::path::Path) -> Result<(), String>) -> Vec<Effect> {
        let (path, mut fx) = self.model.ensure_shot_file(id);
        if let Some(p) = path {
            if let Err(e) = f(&p) {
                fx.push(Effect::Toast(Toast::error(e)));
            }
        }
        fx
    }

    fn open_save_dir(&mut self) -> Vec<Effect> {
        let dir = self.model.config.save_dir.clone();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return vec![Effect::Toast(Toast::error(format!("Không tạo được {}: {e}", dir.display())))];
        }
        match util::shell_open(&dir) {
            Ok(()) => vec![],
            Err(e) => vec![Effect::Toast(Toast::error(e))],
        }
    }

    // ---------- Effect ----------

    fn apply(&mut self, fx: Vec<Effect>) {
        for e in fx {
            match e {
                Effect::ShowOverlays => self.show_overlays(),
                Effect::HideOverlays => {
                    self.auto_end();
                    self.overlays.iter().for_each(|o| o.hide());
                }
                Effect::ShowFrame => {
                    if let Some(outer) = self.model.frame_outer() {
                        let faded = self.model.mode == FrameMode::Faded;
                        self.frame.show(outer, self.model.frame_margin, self.model.region_scale(), faded);
                    }
                }
                Effect::UpdateFrameMode => self.frame.set_faded(self.model.mode == FrameMode::Faded),
                Effect::HideFrame => self.frame.hide(),
                Effect::LayoutToolbar => {
                    if let Some(r) = self.model.toolbar_rect() {
                        let faded = self.model.mode == FrameMode::Faded;
                        self.toolbar.show(r, self.model.region_scale(), faded);
                    }
                }
                Effect::HideToolbar => self.toolbar.hide(),
                Effect::StackChanged => self.layout_stack(),
                Effect::Toast(t) => {
                    log::info!("toast: {}", t.message);
                    self.layout_stack_target();
                    self.stack.toast(t);
                }
                Effect::StartCapture(job) => self.start_capture(job.rect, job.hide_ui, job.text),
                Effect::ApplyHotkeys => self.apply_hotkeys(),
                Effect::SyncAutostart => self.sync_autostart(),
                Effect::ConfigChanged => {
                    self.refresh_settings();
                    let p = &self.model.config.preview;
                    self.preview.apply_config(p.always_on_top, p.opacity);
                }
                Effect::StartFreezeTimer => {
                    unsafe { SetTimer(Some(win::app_hwnd()), TIMER_FREEZE, FREEZE_DELAY_MS, None) };
                    if let Some(t) = &self.tray {
                        t.set_waiting(true);
                    }
                }
                Effect::CancelFreezeTimer => {
                    unsafe {
                        let _ = KillTimer(Some(win::app_hwnd()), TIMER_FREEZE);
                    }
                    if let Some(t) = &self.tray {
                        t.set_waiting(false);
                    }
                }
                Effect::FreezeGrab => self.start_freeze_grab(),
                Effect::ShowFreezeOverlays => self.show_freeze_overlays(),
                Effect::FreezePersist(rect) => self.freeze_output(rect, true),
                Effect::FreezeCopy(rect) => self.freeze_output(rect, false),
                Effect::OpenPreview(path) => self.open_preview(path),
                Effect::FreezeOcr(rect) => self.freeze_ocr(rect),
                Effect::StartOcr(job) => self.start_ocr(job),
                Effect::CancelOcr => self.ocr.cancel(),
                Effect::OcrBusyChanged => {
                    let busy = match &self.model.ocr_busy {
                        Some(OcrJob { source: OcrSource::Preview(p), copy: true, .. }) => Some(p.clone()),
                        _ => None,
                    };
                    self.preview.set_ocr_busy(busy);
                }
                Effect::CopyText(text) => {
                    std::thread::spawn(move || {
                        let t = match chup_core::clipboard::copy_text(&text) {
                            Ok(()) => Toast::info(chup_core::ocr::copied_message(&text)),
                            Err(e) => Toast::error(e),
                        };
                        emit(Action::Toast(t));
                    });
                }
                Effect::AskOcrDownload => {
                    self.ocr.stop_helper();
                    post_ocr_download(true);
                    let fx = self.model.ocr_abort();
                    self.apply(fx);
                }
                Effect::PreviewText(path, text) => self.preview.set_text(&path, text),
            }
        }
    }

    fn sync_overlays(&mut self) {
        let mons = self.model.monitors.clone();
        let same = self.overlays.len() == mons.len() && self.overlays.iter().zip(&mons).all(|(o, m)| &o.mon == m);
        if same {
            return;
        }
        for o in self.overlays.drain(..) {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(o.hwnd);
            }
        }
        for m in mons {
            match Overlay::create(m) {
                Ok(o) => self.overlays.push(o),
                Err(e) => log::error!("Không tạo được overlay: {e}"),
            }
        }
        // Loại overlay khỏi ảnh chụp: Live chụp ngầm màn hình để nhận khung từ ảnh (SPEC 16, thiết kế mục 4).
        // Hệ quả: app chia sẻ màn hình không thấy overlay.
        self.overlay_excluded = !self.overlays.is_empty()
            && self
                .overlays
                .iter()
                .all(|o| unsafe { SetWindowDisplayAffinity(o.hwnd, WDA_EXCLUDEFROMCAPTURE) }.is_ok());
        log::info!("overlay: {} màn hình, loại khỏi ảnh chụp: {}", self.overlays.len(), self.overlay_excluded);
    }

    /// Bắt đầu phiên tự chọn vùng cho các overlay vừa hiện. `wins`: danh sách cửa sổ chụp trước đó.
    fn auto_begin(&mut self, wins: Rc<Vec<WinFrame>>, frozen: Vec<(Rect, Arc<RgbaImage>)>) {
        let cfg = &self.model.config.auto_detect;
        let image_edges = cfg.image_edges;
        let live_capture = image_edges && frozen.is_empty() && self.overlay_excluded;
        self.auto_gen = self.auto_gen.wrapping_add(1);
        self.auto_on = true;
        self.auto.begin(self.auto_gen, frozen, image_edges, live_capture);
        for o in &self.overlays {
            o.auto_begin(wins.clone());
        }
        // Hỏi ngay ở vị trí con trỏ: tô khung cửa sổ liền, và để Chromium / Electron kịp bật cây accessibility.
        let (cx, cy) = cursor_pos();
        if let Some(o) = self.overlays.iter().find(|o| o.mon.rect.contains_point(cx, cy)) {
            o.auto_refresh();
        }
    }

    fn auto_end(&mut self) {
        if std::mem::take(&mut self.auto_on) {
            self.auto.end(self.auto_gen);
        }
        self.freeze_wins = None;
    }

    fn show_overlays(&mut self) {
        // Màn hình có thể đã đổi mà chưa kịp nhận WM_DISPLAYCHANGE.
        let mons = util::monitors();
        if !mons.is_empty() && mons != self.model.monitors {
            let fx = self.model.set_monitors(mons);
            self.apply(fx.into_iter().filter(|e| *e != Effect::HideOverlays).collect());
            self.model.selecting = true;
        }
        let auto = self.model.config.auto_detect.enabled;
        self.sync_overlays();
        let (cx, cy) = cursor_pos();
        let focus = self.overlays.iter().position(|o| o.mon.rect.contains_point(cx, cy)).unwrap_or(0);
        // Hiện overlay không lấy focus trước, overlay dưới con trỏ sau cùng (nhận phím Esc).
        for (i, o) in self.overlays.iter().enumerate() {
            if i != focus {
                o.show(false, auto);
            }
        }
        if let Some(o) = self.overlays.get(focus) {
            o.show(true, auto);
        }
        if self.model.config.auto_detect.enabled {
            let wins = Rc::new(autoselect::snapshot_windows());
            self.auto_begin(wins, vec![]);
        }
    }

    /// Chọn màn hình đặt stack theo cấu hình; chốt lại khi stack đang hiện.
    fn layout_stack_target(&mut self) -> Option<chup_core::coords::MonitorInfo> {
        let mons = &self.model.monitors;
        let keep = self
            .stack_monitor
            .as_ref()
            .filter(|_| self.stack.is_visible())
            .and_then(|k| mons.iter().find(|m| &m.key == k));
        let mon = keep
            .or_else(|| match self.model.config.stack.monitor {
                StackMonitor::Primary => mons.iter().find(|m| m.primary),
                StackMonitor::Cursor => {
                    let (x, y) = cursor_pos();
                    monitor_at(mons, x, y)
                }
            })
            .or(mons.first())
            .cloned()?;
        self.stack_monitor = Some(mon.key.clone());
        let edge_right = self.model.config.stack.edge == Edge::Right;
        // Chỉ cập nhật vị trí (giữ danh sách shot cũ) để toast hiện đúng chỗ.
        self.stack.update(self.model.stack.list(), edge_right, mon.work, mon.scale);
        Some(mon)
    }

    fn layout_stack(&mut self) {
        if self.layout_stack_target().is_none() {
            log::warn!("Không có màn hình nào để đặt stack");
        }
    }

    fn start_capture(&mut self, rect: Rect, hide_ui: bool, text: bool) {
        if hide_ui {
            self.toolbar.set_hidden_for_capture(true);
            self.stack.set_hidden_for_capture(true);
            self.ui_hidden_for_capture = true;
        }
        std::thread::spawn(move || {
            if hide_ui {
                // Chờ DWM vẽ lại màn hình sau khi ẩn cửa sổ.
                std::thread::sleep(Duration::from_millis(80));
            }
            let t0 = std::time::Instant::now();
            let r = capture::grab_region(rect);
            log::info!("chụp {}×{} mất {:?}", rect.w, rect.h, t0.elapsed());
            emit(if text { Action::TextGrabbed(r, rect) } else { Action::Grabbed(r) });
        });
    }

    fn restore_ui_after_capture(&mut self) {
        if std::mem::take(&mut self.ui_hidden_for_capture) {
            self.toolbar.set_hidden_for_capture(false);
            self.stack.set_hidden_for_capture(false);
        }
    }

    fn on_grabbed(&mut self, res: Result<RgbaImage, String>) -> Vec<Effect> {
        self.restore_ui_after_capture();
        match res {
            Err(e) => self.model.capture_grabbed(Err(e)),
            Ok(img) => {
                let fx = self.model.capture_grabbed(Ok(()));
                let opts = self.model.persist_options();
                std::thread::spawn(move || {
                    let t0 = std::time::Instant::now();
                    let r = capture::persist_shot(&img, &opts);
                    log::info!("lưu ảnh mất {:?}", t0.elapsed());
                    emit(Action::Persisted(r));
                });
                fx
            }
        }
    }

    // ---------- Preview (SPEC 17) ----------

    fn open_preview(&mut self, path: std::path::PathBuf) {
        self.preview.request(path.clone());
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let r = image::open(&path).map(|i| i.to_rgba8()).map_err(|e| format!("Không mở được ảnh: {e}"));
            log::info!("preview: giải mã ảnh mất {:?}", t0.elapsed());
            emit(Action::PreviewLoaded(path, r));
        });
    }

    fn on_preview_loaded(&mut self, path: std::path::PathBuf, res: Result<RgbaImage, String>) -> Vec<Effect> {
        // Người dùng đã bấm thumbnail khác trong lúc giải mã: bỏ ảnh này.
        if !self.preview.is_pending(&path) {
            return vec![];
        }
        let img = match res {
            Ok(img) => img,
            Err(e) => {
                self.preview.cancel_pending();
                return vec![Effect::Toast(Toast::error(e))];
            }
        };
        let Some((view, work)) = self.preview_view(img.dimensions()) else { return vec![] };
        let p = &self.model.config.preview;
        self.preview.show(path.clone(), img, view, work, p.always_on_top, p.opacity);
        // `show` có thể thất bại (thiếu bộ nhớ) và tự đóng: khi đó không nhận diện nền.
        if !self.preview.is_visible() {
            return vec![];
        }
        self.model.preview_shown(path)
    }

    /// Chỗ và cỡ mở Preview theo Cài đặt (SPEC 17). Trả về view và work area của màn hình đó.
    fn preview_view(&self, img: (u32, u32)) -> Option<(chup_core::preview::View, Rect)> {
        use chup_core::{config::PreviewPosition, preview};
        let mons = &self.model.monitors;
        let p = &self.model.config.preview;
        if p.position == PreviewPosition::Remember {
            if let Some(b) = p.last {
                let bx = Rect::new(b.x, b.y, b.w, b.h);
                let (cx, cy) = bx.center();
                // Màn hình chứa khung cũ đã bị rút thì mở như "cạnh stack".
                if let Some(m) = monitor_at(mons, cx, cy).filter(|m| m.rect.contains_point(cx, cy)) {
                    let min = super::preview::min_size(m.scale);
                    return Some((preview::open_in_box(img, bx, m.work, min), m.work));
                }
            }
        }
        // Màn hình của stack (nơi vừa bấm thumbnail).
        let mon = self
            .stack_monitor
            .as_ref()
            .and_then(|k| mons.iter().find(|m| &m.key == k))
            .or_else(|| {
                let (x, y) = cursor_pos();
                monitor_at(mons, x, y)
            })
            .or(mons.first())?;
        let min = super::preview::min_size(mon.scale);
        let view = if p.position == PreviewPosition::Center {
            preview::open_center(img, mon.work, p.size, min)
        } else {
            let edge_right = self.model.config.stack.edge == Edge::Right;
            let gap = (12.0 * mon.scale).round() as i32;
            preview::open_beside_stack(img, mon.work, p.size, min, self.stack.window_rect(), edge_right, gap)
        };
        Some((view, mon.work))
    }

    // ---------- Freeze (SPEC 14) ----------

    fn start_freeze_grab(&mut self) {
        self.freeze_t0 = Some(std::time::Instant::now());
        // Trước khi chụp và trước khi overlay lấy focus: menu chuột phải còn mở nên còn trong danh sách.
        self.freeze_wins =
            self.model.config.auto_detect.enabled.then(|| Rc::new(autoselect::snapshot_windows()));
        std::thread::spawn(|| {
            let t0 = std::time::Instant::now();
            let r = capture::grab_monitors();
            log::info!("freeze: chụp mọi màn hình mất {:?}", t0.elapsed());
            emit(Action::FreezeGrabbed(r));
        });
    }

    fn show_freeze_overlays(&mut self) {
        self.sync_overlays();
        let mut shots = std::mem::take(&mut self.frozen);
        let (cx, cy) = cursor_pos();
        let focus = self.overlays.iter().position(|o| o.mon.rect.contains_point(cx, cy)).unwrap_or(0);
        // Overlay không lấy focus hiện trước, overlay dưới con trỏ sau cùng (nhận Enter, Esc).
        let order = (0..self.overlays.len()).filter(|i| *i != focus).chain(std::iter::once(focus));
        let mut shown = false;
        for i in order {
            let Some(o) = self.overlays.get(i) else { continue };
            // Ghép theo góc trên-trái: kích thước xcap báo có thể lệch vài px so với EnumDisplayMonitors.
            let img = shots
                .iter()
                .position(|(r, _)| (r.x, r.y) == (o.mon.rect.x, o.mon.rect.y))
                .and_then(|p| shots.swap_remove(p).1.ok());
            if img.is_none() {
                log::warn!("freeze: không có ảnh cho màn hình {}", o.mon.key);
            }
            shown |= o.show_frozen(i == focus, img, self.model.freeze_ocr, self.model.config.auto_detect.enabled);
        }
        if let Some(t0) = self.freeze_t0.take() {
            log::info!("freeze: từ nhấn hotkey tới lúc ảnh hiện {:?}", t0.elapsed());
        }
        if let (true, Some(wins)) = (shown, self.freeze_wins.take()) {
            let frozen = self.overlays.iter().filter_map(|o| o.frozen_image().map(|i| (o.mon.rect, i))).collect();
            self.auto_begin(wins, frozen);
        }
        if !shown {
            // Không overlay nào hiện được: không còn chỗ nhận Esc, phải tự kết thúc phiên.
            let mut fx = self.model.freeze_cancel();
            fx.push(Effect::Toast(Toast::error("Không đủ bộ nhớ để hiện ảnh đóng băng")));
            self.apply(fx);
        }
    }

    fn freeze_output(&mut self, rect: Rect, save: bool) {
        let Some((mon, img)) = self.picked.take() else {
            log::warn!("freeze: không có ảnh để crop");
            return;
        };
        let mut opts = self.model.persist_options();
        opts.scale = self.model.scale_at(&rect);
        std::thread::spawn(move || {
            let shot = chup_core::freeze::crop(&img, mon, rect);
            drop(img);
            let Some(shot) = shot else {
                emit(Action::Toast(Toast::error("Vùng chọn nằm ngoài ảnh")));
                return;
            };
            if save {
                let t0 = std::time::Instant::now();
                let r = capture::persist_shot(&shot, &opts);
                log::info!("freeze: lưu ảnh mất {:?}", t0.elapsed());
                emit(Action::Persisted(r));
            } else {
                emit(Action::FreezeCopied(chup_core::clipboard::copy_image(&shot, None)));
            }
        });
    }

    // ---------- OCR (SPEC 15) ----------

    fn freeze_ocr(&mut self, rect: Rect) {
        let Some((mon, img)) = self.picked.take() else {
            log::warn!("freeze: không có ảnh để OCR");
            return;
        };
        let Some(shot) = chup_core::freeze::crop(&img, mon, rect) else {
            self.apply(vec![Effect::Toast(Toast::error("Vùng chọn nằm ngoài ảnh"))]);
            return;
        };
        drop(img);
        self.ocr_image = Some(shot);
        let fx = self.model.ocr_request(OcrSource::Image { scale: self.model.scale_at(&rect) });
        self.apply(fx);
    }

    fn start_ocr(&mut self, job: OcrJob) {
        let img = match &job.source {
            // Preview đã giải mã sẵn; đã đóng hoặc đổi ảnh (vd. chờ tải gói xong) thì đọc lại file.
            OcrSource::Preview(p) => self.preview.image_of(p).or_else(|| image::open(p).ok().map(|i| i.to_rgba8())),
            OcrSource::Image { .. } => self.ocr_image.take(),
        };
        match img {
            Some(img) => {
                self.ocr.submit(ocr::Job { id: job.id, lang: job.lang, scale: job.scale, img, background: !job.copy })
            }
            None => {
                let fx = self.model.ocr_done(job.id, Err("Không đọc được ảnh để OCR".into()));
                self.apply(fx);
            }
        }
    }

    fn apply_hotkeys(&mut self) {
        let Some(h) = self.hotkeys.as_mut() else { return };
        self.model.hotkey_errors = h.apply(&self.model.config.hotkeys);
        if let Some(t) = &self.tray {
            t.set_hotkey_errors(self.model.hotkey_errors.len());
            let hk = &self.model.config.hotkeys;
            let errs = &self.model.hotkey_errors;
            let show = |name: &str, key: &str| if errs.contains_key(name) { String::new() } else { key.trim().to_string() };
            let keys = [
                show("selectRegion", &hk.select_region),
                show("capture", &hk.capture),
                show("freeze", &hk.freeze),
            ];
            t.set_shortcuts([&keys[0], &keys[1], &keys[2]]);
        }
        self.refresh_settings();
    }

    fn sync_autostart(&mut self) {
        let want = self.model.config.launch_at_startup;
        if util::autostart_enabled() != want {
            if let Err(e) = util::set_autostart(want) {
                self.apply(vec![Effect::Toast(Toast::error(e))]);
            }
        }
    }

    fn settings_view(&self) -> SettingsView {
        SettingsView {
            config: self.model.config.clone(),
            hotkey_errors: self.model.hotkey_errors.clone(),
            preview: format!(
                "{}.png",
                capture::render_file_name(&self.model.config.file_name_pattern, &chrono_now())
            ),
            exclusion_ok: self.model.capture_exclusion_ok,
            ocr_vi: self.model.ocr_vi,
        }
    }

    fn open_settings(&mut self) {
        if self.settings.is_none() {
            match Settings::create() {
                Ok(s) => self.settings = Some(s),
                Err(e) => {
                    log::error!("Không mở được Cài đặt: {e}");
                    return;
                }
            }
        }
        self.refresh_settings();
        if let Some(s) = &self.settings {
            s.show();
        }
    }

    fn refresh_settings(&self) {
        if let Some(s) = &self.settings {
            s.refresh(self.settings_view());
        }
    }
}

/// Hộp thoại tải gói là modal: cửa sổ chính mở nó sau khi App xử lý xong Action hiện tại.
fn post_ocr_download(ask: bool) {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::PostMessageW,
    };
    unsafe {
        let _ = PostMessageW(Some(win::app_hwnd()), win::WM_APP_OCR_DOWNLOAD, WPARAM(ask as usize), LPARAM(0));
    }
}

fn chrono_now() -> chrono::DateTime<chrono::Local> {
    chrono::Local::now()
}
