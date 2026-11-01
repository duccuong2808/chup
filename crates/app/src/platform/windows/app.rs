//! Điều phối: nhận `Action` (từ cửa sổ, tray, hotkey, worker), gọi `Model`, áp `Effect` lên cửa sổ.
//! Chụp và lưu file chạy ở worker thread; kết quả quay về bằng `emit`.

use std::{rc::Rc, time::Duration};

use chup_core::{
    capture::{self, PersistOutcome},
    coords::{monitor_at, Rect},
    config::{Edge, StackMonitor},
    model::{Effect, FrameMode, Model, Toast, FREEZE_DELAY_MS},
};
use image::RgbaImage;
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, PostQuitMessage, SetTimer, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
};

use super::{
    frame::Frame,
    hotkeys::{Hotkeys, Kind},
    overlay::Overlay,
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
}

/// Id timer hẹn giờ "Chụp đóng băng sau 3 giây" trên cửa sổ chính (mod.rs bắn `FreezeTimerFired`).
pub const TIMER_FREEZE: usize = 5;

pub struct App {
    pub model: Model,
    overlays: Vec<Rc<Overlay>>,
    frame: Rc<Frame>,
    toolbar: Rc<Toolbar>,
    stack: Rc<StackWin>,
    settings: Option<Rc<Settings>>,
    tray: Option<Tray>,
    hotkeys: Option<Hotkeys>,
    /// Màn hình stack đang đứng; chốt lúc stack bắt đầu hiện để nó không nhảy theo chuột.
    stack_monitor: Option<String>,
    ui_hidden_for_capture: bool,
}

impl App {
    pub fn new(model: Model) -> windows::core::Result<App> {
        let frame = Frame::create()?;
        let toolbar = Toolbar::create()?;
        let stack = StackWin::create()?;
        let mut app = App {
            model,
            overlays: vec![],
            frame,
            toolbar,
            stack,
            settings: None,
            tray: None,
            hotkeys: None,
            stack_monitor: None,
            ui_hidden_for_capture: false,
        };

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
            Action::Persisted(res) => self.model.capture_persisted(res),
            Action::StackRemove(id) => self.model.stack_remove(&id),
            Action::StackClear => self.model.stack_clear(),
            Action::ShotOpen(id) => self.with_shot_file(&id, util::shell_open),
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
                "freeze" => self.model.start_freeze(),
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
            Action::DisplayChanged => {
                let fx = self.model.set_monitors(util::monitors());
                self.sync_overlays();
                fx
            }
        };
        self.apply(fx);
    }

    /// App đang rảnh: không chọn vùng, Cài đặt đã đóng (frame có thể đang hiện, nhất là ở chế độ Mờ).
    pub fn is_idle(&self) -> bool {
        !self.model.selecting && self.model.freeze.is_none() && !self.model.freeze_timer && self.settings.is_none()
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
                Effect::HideOverlays => self.overlays.iter().for_each(|o| o.hide()),
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
                Effect::StartCapture(job) => self.start_capture(job.rect, job.hide_ui),
                Effect::ApplyHotkeys => self.apply_hotkeys(),
                Effect::SyncAutostart => self.sync_autostart(),
                Effect::ConfigChanged => self.refresh_settings(),
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
                // Freeze: nối ở Task 10 của docs/plans/2026-11-01-freeze.md.
                _ => {}
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
    }

    fn show_overlays(&mut self) {
        // Màn hình có thể đã đổi mà chưa kịp nhận WM_DISPLAYCHANGE.
        let mons = util::monitors();
        if !mons.is_empty() && mons != self.model.monitors {
            let fx = self.model.set_monitors(mons);
            self.apply(fx.into_iter().filter(|e| *e != Effect::HideOverlays).collect());
            self.model.selecting = true;
        }
        self.sync_overlays();
        let (cx, cy) = cursor_pos();
        let focus = self.overlays.iter().position(|o| o.mon.rect.contains_point(cx, cy)).unwrap_or(0);
        // Hiện overlay không lấy focus trước, overlay dưới con trỏ sau cùng (nhận phím Esc).
        for (i, o) in self.overlays.iter().enumerate() {
            if i != focus {
                o.show(false);
            }
        }
        if let Some(o) = self.overlays.get(focus) {
            o.show(true);
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

    fn start_capture(&mut self, rect: Rect, hide_ui: bool) {
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
            emit(Action::Grabbed(r));
        });
    }

    fn on_grabbed(&mut self, res: Result<RgbaImage, String>) -> Vec<Effect> {
        if std::mem::take(&mut self.ui_hidden_for_capture) {
            self.toolbar.set_hidden_for_capture(false);
            self.stack.set_hidden_for_capture(false);
        }
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

    fn apply_hotkeys(&mut self) {
        let Some(h) = self.hotkeys.as_mut() else { return };
        self.model.hotkey_errors = h.apply(&self.model.config.hotkeys);
        if let Some(t) = &self.tray {
            t.set_hotkey_errors(self.model.hotkey_errors.len());
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

fn chrono_now() -> chrono::DateTime<chrono::Local> {
    chrono::Local::now()
}
