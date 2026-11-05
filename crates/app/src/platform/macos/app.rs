//! Điều phối (giống `windows/app.rs`): nhận `Action` (cửa sổ, tray, hotkey, worker), gọi `Model`, áp `Effect` lên
//! cửa sổ. Chụp (ScreenCaptureKit) và lưu file chạy ngoài main thread; kết quả quay về bằng `emit`.

use std::{rc::Rc, sync::Arc};

use chup_core::{
    autoselect::{Frame as AutoFrame, WinFrame},
    capture::{self, MonitorShot, PersistOutcome},
    coords::{monitor_at, Rect},
    config::{Edge, StackMonitor},
    i18n::{fill, t},
    model::{Effect, FrameMode, FreezeAction, FreezePhase, Model, OcrJob, OcrSource, OcrViState, Toast, FREEZE_DELAY_MS},
};
use image::RgbaImage;
use objc2::MainThreadMarker;

use super::{
    autoselect,
    capture as sck,
    frame::Frame,
    hotkeys::{Hotkeys, Kind},
    overlay::Overlay,
    preview::Preview,
    settings::{Settings, SettingsView},
    stack::StackWin,
    toolbar::Toolbar,
    tray::Tray,
    util::{self, cursor_pos},
    vision,
    win::{self, emit, Timer},
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
    Open(std::path::PathBuf),
    DisplayChanged,
    /// Giao diện sáng / tối đổi.
    AppearanceChanged,
    FreezeTimerFired,
    FreezeNow,
    FreezeGrabbed(u32, Result<Vec<MonitorShot>, String>),
    /// Overlay (khoá màn hình) vừa bắt đầu chọn vùng mới: các overlay khác bỏ vùng của mình.
    FreezeSelecting(String),
    FreezeConfirm { rect: Rect, action: FreezeAction },
    FreezeCancel,
    FreezeCopied(Result<(), String>),
    /// Overlay: chuột ở điểm `p` (desktop) trên cửa sổ `win`; hỏi worker chuỗi khung (SPEC 16).
    AutoHover { p: (i32, i32), win: WinFrame, mon: Rect },
    /// Worker: chuỗi khung thô cho điểm `p` của phiên `gen`.
    AutoFrames { gen: u32, p: (i32, i32), window: isize, chain: Vec<AutoFrame> },
    /// Cuộn chuột / ↑↓ (dương = lên khung cha); `0` = nhấn / nhả Shift.
    AutoLevel(i32),
    PreviewLoaded(std::path::PathBuf, Result<RgbaImage, String>),
    PreviewCopy(std::path::PathBuf),
    PreviewReveal(std::path::PathBuf),
    PreviewClosed,
    PreviewBox(Rect),
    PreviewOcr(std::path::PathBuf),
    OcrDone(u32, Result<chup_core::ocr::TextLayout, String>),
    CopyText(String),
    /// Chưa có quyền Screen Recording: hỏi người dùng (sau khi App xử lý xong Action hiện tại).
    AskPermission,
    /// Bước của chế độ tự kiểm tra (`selftest.rs`).
    SelfTest(u32),
    Quit,
}

/// Mục tray "Chụp đóng băng": chờ menu tắt hẳn rồi mới chụp, để menu không lọt vào ảnh.
const FREEZE_MENU_DELAY_MS: u32 = 350;

pub struct App {
    pub model: Model,
    mtm: MainThreadMarker,
    overlays: Vec<Rc<Overlay>>,
    frame: Rc<Frame>,
    toolbar: Rc<Toolbar>,
    stack: Rc<StackWin>,
    preview: Rc<Preview>,
    settings: Option<Rc<Settings>>,
    tray: Option<Tray>,
    hotkeys: Option<Hotkeys>,
    stack_monitor: Option<String>,
    frozen: Vec<MonitorShot>,
    picked: Option<(Rect, Arc<RgbaImage>)>,
    freeze_timer: Option<Timer>,
    freeze_t0: Option<std::time::Instant>,
    ocr: vision::Worker,
    ocr_image: Option<RgbaImage>,
    auto: autoselect::Worker,
    auto_gen: u32,
    auto_on: bool,
    /// Danh sách cửa sổ chụp lúc bắt đầu Freeze (trước khi overlay hiện).
    freeze_wins: Option<Rc<Vec<WinFrame>>>,
}

impl App {
    pub fn new(model: Model, mtm: MainThreadMarker) -> App {
        let mut app = App {
            model,
            mtm,
            overlays: vec![],
            frame: Frame::create(mtm),
            toolbar: Toolbar::create(mtm),
            stack: StackWin::create(mtm),
            preview: Preview::create(mtm),
            settings: None,
            tray: None,
            hotkeys: None,
            stack_monitor: None,
            frozen: vec![],
            picked: None,
            freeze_timer: None,
            freeze_t0: None,
            ocr: vision::Worker::start(),
            ocr_image: None,
            auto: autoselect::Worker::start(),
            auto_gen: 0,
            auto_on: false,
            freeze_wins: None,
        };
        // Apple Vision có sẵn cả tiếng Việt (macOS 26): không có gói tải thêm.
        app.model.set_ocr_vi(OcrViState::Ready);
        // ScreenCaptureKit loại cửa sổ của Chup khỏi ảnh chụp (SPEC 6.4).
        app.model.capture_exclusion_ok = true;
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
        app
    }

    // ---------- Action ----------

    pub fn handle(&mut self, a: Action) {
        let mem_label = match &a {
            Action::StartSelection => Some("mở overlay chọn vùng"),
            Action::Persisted(_) => Some("chụp và lưu xong"),
            Action::FreezeGrabbed(..) => Some("hiện ảnh Freeze"),
            Action::FreezeConfirm { .. } => Some("Freeze xong"),
            Action::PreviewLoaded(..) => Some("mở Preview"),
            _ => None,
        };
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
                        let t = match chup_core::clipboard::copy_file_image(&p) {
                            Ok(()) => Toast::info(t().image_copied),
                            Err(e) => Toast::error(e),
                        };
                        emit(Action::Toast(t));
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
                    self.freeze_timer = Some(win::emit_after(FREEZE_MENU_DELAY_MS, || Action::FreezeNow));
                    vec![]
                }
                "freeze_delayed" => self.model.start_freeze_delayed(),
                "freeze_toggle" => {
                    let on = !self.model.config.hotkeys.freeze_enabled;
                    emit(Action::ConfigPatch(serde_json::json!({ "hotkeys": { "freezeEnabled": on } })));
                    vec![]
                }
                "open_folder" => self.open_save_dir(),
                "settings" => {
                    self.open_settings();
                    vec![]
                }
                "quit" => {
                    emit(Action::Quit);
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
                self.apply_hotkeys();
                vec![]
            }
            Action::ConfigPatch(p) => match self.model.update_config(&p) {
                Ok(fx) => {
                    if let Some(s) = &self.settings {
                        s.set_status(t().st_saved, false);
                    }
                    fx
                }
                Err(e) => {
                    if let Some(s) = &self.settings {
                        s.set_status(&e, true);
                    }
                    self.apply_hotkeys();
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
            Action::Open(target) => match util::shell_open(&target) {
                Ok(()) => vec![],
                Err(e) => vec![Effect::Toast(Toast::error(e))],
            },
            Action::FreezeTimerFired => {
                self.freeze_timer = None;
                self.model.freeze_timer_fired()
            }
            Action::FreezeNow => {
                self.freeze_timer = None;
                self.model.start_freeze()
            }
            Action::FreezeGrabbed(gen, res) => {
                let current = gen == self.model.freeze_gen;
                if current || self.model.freeze != Some(FreezePhase::Grabbing) {
                    self.restore_ui_after_capture();
                }
                if current {
                    let shots = res.unwrap_or_else(|e| {
                        log::error!("freeze: {e}");
                        vec![]
                    });
                    for (r, e) in shots.iter().filter_map(|(r, i)| i.as_ref().err().map(|e| (r, e))) {
                        log::warn!("freeze: màn hình {r:?}: {e}");
                    }
                    let ok = shots.iter().any(|(_, i)| i.is_ok());
                    self.frozen = shots;
                    self.model.freeze_grabbed(gen, ok)
                } else {
                    vec![]
                }
            }
            Action::FreezeSelecting(key) => {
                for o in self.overlays.iter().filter(|o| o.mon.key != key) {
                    o.clear_frozen_selection();
                }
                vec![]
            }
            Action::FreezeConfirm { rect, action } => {
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
                        Ok(()) => Toast::info(t().image_copied),
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
            Action::AskPermission => {
                // Ngoài lúc App bận: hộp thoại modal chạy vòng lặp riêng, Action vẫn được xử lý.
                win::after(0, super::ask_permission);
                vec![]
            }
            Action::DisplayChanged => {
                let fx = self.model.set_monitors(util::monitors());
                self.sync_overlays();
                fx
            }
            Action::AppearanceChanged => {
                self.layout_stack();
                self.refresh_settings();
                vec![]
            }
            Action::SelfTest(n) => {
                self.self_test(n);
                vec![]
            }
            Action::Quit => {
                log::info!("thoát");
                log::logger().flush();
                std::process::exit(0);
            }
        };
        self.apply(fx);
        self.frozen.clear();
        self.picked = None;
        if let Some(label) = mem_label {
            util::log_memory(label);
        }
    }

    pub fn is_idle(&self) -> bool {
        !self.model.selecting
            && self.model.freeze.is_none()
            && !self.model.freeze_timer
            && self.settings.is_none()
            && !self.preview.is_visible()
    }

    /// Rảnh đủ lâu: bỏ bộ đệm cỡ màn hình của overlay.
    pub fn trim(&self) {
        for o in &self.overlays {
            o.release();
        }
        util::log_memory("sau khi trả RAM");
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
            return vec![Effect::Toast(Toast::error(fill(t().err_create_dir, &[&dir.display(), &e])))];
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
                Effect::UiLanguageChanged => {
                    util::apply_ui_language(self.model.config.ui_language);
                    if let Some(t) = &self.tray {
                        t.retext();
                    }
                    if let Some(s) = &self.settings {
                        s.retext();
                    }
                    self.apply_hotkeys();
                }
                Effect::ConfigChanged => {
                    self.refresh_settings();
                    let p = &self.model.config.preview;
                    self.preview.apply_config(p.always_on_top, p.opacity);
                }
                Effect::StartFreezeTimer => {
                    self.freeze_timer = Some(win::emit_after(FREEZE_DELAY_MS, || Action::FreezeTimerFired));
                    if let Some(t) = &self.tray {
                        t.set_waiting(true);
                    }
                }
                Effect::CancelFreezeTimer => {
                    self.freeze_timer = None;
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
                // Vision có sẵn tiếng Việt: không bao giờ phải tải gói.
                Effect::AskOcrDownload => {
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
            o.hide();
        }
        for m in mons {
            self.overlays.push(Overlay::create(self.mtm, m));
        }
        log::info!("overlay: {} màn hình", self.overlays.len());
    }

    /// Chưa có quyền Screen Recording: không mở overlay (ảnh sẽ chỉ có hình nền), hỏi người dùng.
    fn permission_ok(&mut self) -> bool {
        if sck::has_permission() || super::selftest::dir().is_some() {
            return true;
        }
        log::warn!("chưa có quyền Screen Recording");
        emit(Action::AskPermission);
        false
    }

    fn show_overlays(&mut self) {
        if !self.permission_ok() {
            let fx = self.model.cancel_selection();
            self.apply(fx);
            return;
        }
        let mons = util::monitors();
        if !mons.is_empty() && mons != self.model.monitors {
            let fx = self.model.set_monitors(mons);
            self.apply(fx.into_iter().filter(|e| *e != Effect::HideOverlays).collect());
            self.model.selecting = true;
        }
        self.sync_overlays();
        let (cx, cy) = cursor_pos();
        let focus = self.overlays.iter().position(|o| o.mon.rect.contains_point(cx, cy)).unwrap_or(0);
        for (i, o) in self.overlays.iter().enumerate() {
            if i != focus {
                o.show(false);
            }
        }
        if let Some(o) = self.overlays.get(focus) {
            o.show(true);
        }
        if self.model.config.auto_detect.enabled {
            let wins = Rc::new(autoselect::snapshot_windows(&util::layout()));
            self.auto_begin(wins, vec![]);
        }
    }

    /// Bắt đầu phiên tự chọn vùng cho các overlay vừa hiện. `frozen`: ảnh Freeze (nhận khung từ ảnh).
    fn auto_begin(&mut self, wins: Rc<Vec<WinFrame>>, frozen: Vec<(Rect, Arc<RgbaImage>)>) {
        let image_edges = self.model.config.auto_detect.image_edges && !frozen.is_empty();
        self.auto_gen = self.auto_gen.wrapping_add(1);
        self.auto_on = true;
        self.auto.begin(self.auto_gen, frozen, image_edges, util::layout());
        for o in &self.overlays {
            o.auto_begin(wins.clone());
        }
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
        self.stack.update(self.model.stack.list(), edge_right, mon.work, mon.scale);
        Some(mon)
    }

    fn layout_stack(&mut self) {
        if self.layout_stack_target().is_none() {
            log::warn!("Không có màn hình nào để đặt stack");
        }
    }

    fn start_capture(&mut self, rect: Rect, hide_ui: bool, text: bool) {
        if !sck::has_permission() {
            emit(Action::AskPermission);
            emit(if text { Action::TextGrabbed(Err(t().capture_failed.into()), rect) } else { Action::Grabbed(Err(t().capture_failed.into())) });
            return;
        }
        // ScreenCaptureKit đã loại cửa sổ của Chup; "Ẩn toolbar và stack" trong Cài đặt vẫn tôn trọng.
        if hide_ui {
            self.toolbar.set_hidden_for_capture(true);
            self.stack.set_hidden_for_capture(true);
        }
        let t0 = std::time::Instant::now();
        sck::grab_region(rect, move |r| {
            log::info!("chụp {}×{} mất {:?}", rect.w, rect.h, t0.elapsed());
            emit(if text { Action::TextGrabbed(r, rect) } else { Action::Grabbed(r) });
        });
    }

    fn restore_ui_after_capture(&mut self) {
        self.toolbar.set_hidden_for_capture(false);
        self.stack.set_hidden_for_capture(false);
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
            let r = image::open(&path).map(|i| i.to_rgba8()).map_err(|e| fill(t().err_open_image, &[&e]));
            emit(Action::PreviewLoaded(path, r));
        });
    }

    fn on_preview_loaded(&mut self, path: std::path::PathBuf, res: Result<RgbaImage, String>) -> Vec<Effect> {
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
        if !self.preview.is_visible() {
            return vec![];
        }
        self.model.preview_shown(path)
    }

    fn preview_view(&self, img: (u32, u32)) -> Option<(chup_core::preview::View, Rect)> {
        use chup_core::{config::PreviewPosition, preview};
        let mons = &self.model.monitors;
        let p = &self.model.config.preview;
        if p.position == PreviewPosition::Remember {
            if let Some(b) = p.last {
                let bx = Rect::new(b.x, b.y, b.w, b.h);
                let (cx, cy) = bx.center();
                if let Some(m) = monitor_at(mons, cx, cy).filter(|m| m.rect.contains_point(cx, cy)) {
                    let min = chup_ui::preview::min_size(m.scale);
                    return Some((preview::open_in_box(img, bx, m.work, min), m.work));
                }
            }
        }
        let mon = self
            .stack_monitor
            .as_ref()
            .and_then(|k| mons.iter().find(|m| &m.key == k))
            .or_else(|| {
                let (x, y) = cursor_pos();
                monitor_at(mons, x, y)
            })
            .or(mons.first())?;
        let min = chup_ui::preview::min_size(mon.scale);
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
        if !self.permission_ok() {
            let fx = self.model.freeze_cancel();
            self.apply(fx);
            return;
        }
        self.freeze_t0 = Some(std::time::Instant::now());
        // Trước khi overlay hiện: menu đang mở còn trong danh sách cửa sổ.
        self.freeze_wins =
            self.model.config.auto_detect.enabled.then(|| Rc::new(autoselect::snapshot_windows(&util::layout())));
        if self.model.config.hide_ui_during_capture {
            self.stack.set_hidden_for_capture(true);
        }
        let gen = self.model.freeze_gen;
        if super::selftest::dir().is_some() && !sck::has_permission() {
            // Tự kiểm tra chưa có quyền: ảnh tổng hợp thay cho ảnh chụp.
            let shots = self.model.monitors.iter().map(|m| (m.rect, Ok(super::selftest::synthetic(m.rect.w, m.rect.h)))).collect();
            emit(Action::FreezeGrabbed(gen, Ok(shots)));
            return;
        }
        let t0 = std::time::Instant::now();
        sck::grab_monitors(move |r| {
            log::info!("freeze: chụp mọi màn hình mất {:?}", t0.elapsed());
            emit(Action::FreezeGrabbed(gen, r));
        });
    }

    fn show_freeze_overlays(&mut self) {
        let mons = util::monitors();
        if !mons.is_empty() && mons != self.model.monitors {
            log::info!("freeze: màn hình đổi trong lúc chụp, chụp lại");
            self.frozen.clear();
            let ocr = self.model.freeze_ocr;
            let mut fx = self.model.set_monitors(mons);
            fx.extend(if ocr { self.model.start_freeze_ocr() } else { self.model.start_freeze() });
            self.apply(fx);
            return;
        }
        self.sync_overlays();
        let mut shots = std::mem::take(&mut self.frozen);
        let (cx, cy) = cursor_pos();
        let focus = self.overlays.iter().position(|o| o.mon.rect.contains_point(cx, cy)).unwrap_or(0);
        let order = (0..self.overlays.len()).filter(|i| *i != focus).chain(std::iter::once(focus));
        let (mut shown, mut focus_shown, mut last_shown) = (false, false, None);
        for i in order {
            let Some(o) = self.overlays.get(i) else { continue };
            let img = shots.iter().position(|(r, _)| *r == o.mon.rect).and_then(|p| shots.swap_remove(p).1.ok());
            if img.is_none() {
                log::warn!("freeze: không có ảnh cho màn hình {}", o.mon.key);
            }
            let ok = o.show_frozen(i == focus, img, self.model.freeze_ocr);
            shown |= ok;
            if ok {
                last_shown = Some(i);
                focus_shown |= i == focus;
            }
        }
        if let (false, Some(i)) = (focus_shown, last_shown) {
            if let Some(o) = self.overlays.get(i) {
                o.take_focus();
            }
        }
        if let Some(t0) = self.freeze_t0.take() {
            log::info!("freeze: từ nhấn hotkey tới lúc ảnh hiện {:?}", t0.elapsed());
        }
        if let (true, Some(wins)) = (shown, self.freeze_wins.take()) {
            let frozen = self.overlays.iter().filter_map(|o| o.frozen_image().map(|i| (o.mon.rect, i))).collect();
            self.auto_begin(wins, frozen);
        }
        if !shown {
            let mut fx = self.model.freeze_cancel();
            fx.push(Effect::Toast(Toast::error(t().err_freeze_memory)));
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
                emit(Action::Toast(Toast::error(t().err_selection_outside)));
                return;
            };
            if save {
                emit(Action::Persisted(capture::persist_shot(&shot, &opts)));
            } else {
                emit(Action::FreezeCopied(chup_core::clipboard::copy_image(&shot, None)));
            }
        });
    }

    // ---------- OCR (SPEC 15, Apple Vision) ----------

    fn freeze_ocr(&mut self, rect: Rect) {
        let Some((mon, img)) = self.picked.take() else {
            log::warn!("freeze: không có ảnh để OCR");
            return;
        };
        let Some(shot) = chup_core::freeze::crop(&img, mon, rect) else {
            self.apply(vec![Effect::Toast(Toast::error(t().err_selection_outside))]);
            return;
        };
        drop(img);
        self.ocr_image = Some(shot);
        let fx = self.model.ocr_request(OcrSource::Image { scale: self.model.scale_at(&rect) });
        self.apply(fx);
    }

    fn start_ocr(&mut self, job: OcrJob) {
        let img = match &job.source {
            OcrSource::Preview(p) => self.preview.image_of(p).or_else(|| image::open(p).ok().map(|i| i.to_rgba8())),
            OcrSource::Image { .. } => self.ocr_image.take(),
        };
        match img {
            Some(img) => self.ocr.submit(vision::Job { id: job.id, lang: job.lang, img }),
            None => {
                let fx = self.model.ocr_done(job.id, Err(t().ocr_read_image_failed.into()));
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
                if hk.freeze_enabled { show("freeze", &hk.freeze) } else { String::new() },
            ];
            t.set_shortcuts([&keys[0], &keys[1], &keys[2]]);
            t.set_freeze_toggle(hk.freeze_enabled, &hk.freeze);
        }
        self.refresh_settings();
    }

    fn sync_autostart(&mut self) {
        let want = self.model.config.launch_at_startup;
        if !util::bundled() {
            return;
        }
        if util::autostart_enabled() != want {
            if let Err(e) = util::set_autostart(want) {
                self.apply(vec![Effect::Toast(Toast::error(e))]);
            }
        }
    }

    // ---------- cho tự kiểm tra ----------

    pub(super) fn focus_overlay(&self) -> Option<Rc<Overlay>> {
        let (cx, cy) = cursor_pos();
        self.overlays.iter().find(|o| o.mon.rect.contains_point(cx, cy)).or(self.overlays.first()).cloned()
    }

    pub(super) fn frame_ref(&self) -> Rc<Frame> {
        self.frame.clone()
    }

    pub(super) fn toolbar_ref(&self) -> Rc<Toolbar> {
        self.toolbar.clone()
    }

    pub(super) fn preview_ref(&self) -> Rc<Preview> {
        self.preview.clone()
    }

    pub(super) fn open_preview_for_test(&mut self, path: std::path::PathBuf) {
        self.open_preview(path);
    }

    pub(super) fn settings_ref(&self) -> Option<Rc<Settings>> {
        self.settings.clone()
    }

    pub(super) fn stack_ref(&self) -> Rc<StackWin> {
        self.stack.clone()
    }

    fn settings_view(&self) -> SettingsView {
        SettingsView {
            config: self.model.config.clone(),
            hotkey_errors: self.model.hotkey_errors.clone(),
            preview: format!("{}.png", capture::render_file_name(&self.model.config.file_name_pattern, &chrono::Local::now())),
        }
    }

    fn open_settings(&mut self) {
        if self.settings.is_none() {
            self.settings = Some(Settings::create(self.mtm));
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
