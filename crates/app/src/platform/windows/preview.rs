//! Cửa sổ Preview (SPEC 17): ảnh to khi bấm thumbnail. Chỉ có 1 cửa sổ, tạo lúc khởi động và ẩn đi khi đóng;
//! ảnh, DIB và bitmap Direct2D chỉ tồn tại khi đang mở.
//!
//! Layered window: opacity là `SourceConstantAlpha`. Vẽ 2 lớp:
//! - `base`: ảnh đã scale + nền + viền, chỉ vẽ lại khi kích thước đổi.
//! - `out`: chép `base` rồi vẽ toolbar (khi hover) và nhãn zoom/opacity, đẩy lên cửa sổ.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};

use chup_core::{
    coords::{monitor_at, Rect},
    icons::Icon,
    preview::{self, View},
    selection::Grip,
};
use image::RgbaImage;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Direct2D::ID2D1Bitmap,
    UI::{
        Controls::WM_MOUSELEAVE,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ADD, VK_CONTROL,
            VK_ESCAPE, VK_NUMPAD0, VK_OEM_MINUS, VK_OEM_PLUS, VK_SUBTRACT,
        },
        WindowsAndMessaging::{
            GetWindowRect, KillTimer, LoadCursorW, SendMessageW, SetCursor, SetForegroundWindow, SetTimer,
            SetWindowPos, SetWindowTextW, ShowWindow, HTCAPTION, HWND_NOTOPMOST, HWND_TOPMOST, IDC_HAND, IDC_SIZENESW,
            IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_CAPTURECHANGED, WM_CLOSE, WM_DPICHANGED, WM_KEYDOWN,
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCLBUTTONDOWN, WM_SETCURSOR, WM_TIMER,
            WS_EX_LAYERED, WS_POPUP,
        },
    },
};

use super::{
    app::Action,
    gfx::{rectf, Align, Canvas, Color, Font, Surface, Theme},
    util::{self, cursor_pos, hiword, loword, wide},
    win::{self, emit, Handler},
};

// Kích thước (DIP).
const MIN_W: f32 = 160.0;
const MIN_H: f32 = 48.0;
const RADIUS: f32 = 8.0;
const BTN: f32 = 28.0;
const BTN_GAP: f32 = 4.0;
const BAR_PAD: f32 = 8.0;
/// Bề rộng dải mép cửa sổ để kéo đổi cỡ.
const EDGE: f32 = 6.0;

const TIMER_PILL: usize = 1;
const PILL_MS: u32 = 1200;
/// Zoom bằng lăn chuột xong một lúc thì mới báo App lưu khung (không ghi file mỗi nấc).
const TIMER_SAVE: usize = 2;
/// Đổi DPI: vẽ lại sau, không vẽ ngay trong message Windows gửi giữa lúc đang đẩy ảnh lên cửa sổ.
const TIMER_DPI: usize = 3;
const SAVE_MS: u32 = 700;
const MK_CONTROL: usize = 0x0008;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Pin,
    Copy,
    Folder,
    Close,
}

const BUTTONS: [(Btn, Icon); 4] =
    [(Btn::Pin, Icon::Pin), (Btn::Copy, Icon::Copy), (Btn::Folder, Icon::Folder), (Btn::Close, Icon::Close)];

#[derive(Default)]
struct State {
    /// File vừa được yêu cầu mở, đang giải mã ở worker. Chỉ nhận ảnh khớp file này.
    pending: Option<PathBuf>,
    path: PathBuf,
    img: Option<RgbaImage>,
    /// `None` = đang ẩn.
    view: Option<View>,
    /// View lúc mở (phím 0 quay về).
    home_zoom: f64,
    work: Rect,
    topmost: bool,
    opacity: u8,
    /// Giá trị Cài đặt đã áp lần cuối: chỉ áp lại khi Cài đặt đổi, để không đè ghim/opacity người dùng vừa chỉnh.
    cfg: Option<(bool, u8)>,
    base: Option<Surface>,
    out: Option<Surface>,
    canvas: Option<Canvas>,
    bmp: Option<ID2D1Bitmap>,
    hover: Option<Btn>,
    hovering: bool,
    tracking: bool,
    press: Option<Btn>,
    pill: Option<String>,
    hits: Vec<(Btn, Rect)>,
    /// Phần lẻ của lăn chuột (chuột lăn mịn gửi delta < 120).
    wheel: i32,
    /// Đang kéo mép để đổi cỡ: mép nào, view lúc bắt đầu, điểm bắt đầu (toạ độ màn hình).
    resize: Option<(Grip, View, (i32, i32))>,
    /// Đã zoom, chưa báo App lưu khung (đợi `TIMER_SAVE`).
    save_pending: bool,
}

/// Cỡ cửa sổ nhỏ nhất (physical px) ở scale `k`.
pub fn min_size(k: f64) -> (u32, u32) {
    ((MIN_W as f64 * k).round() as u32, (MIN_H as f64 * k).round() as u32)
}

pub struct Preview {
    pub hwnd: HWND,
    /// Đang mở. Để ngoài `st`: wndproc đọc cờ này ở mọi message, kể cả message Windows gửi đồng bộ
    /// trong lúc `st` đang bị borrow (UpdateLayeredWindow đổi vị trí/cỡ gửi WM_WINDOWPOSCHANGED...).
    open: Cell<bool>,
    st: RefCell<State>,
}

thread_local! {
    static CLASS: Vec<u16> = win::register_class::<Preview>("ChupPreview");
}

impl Preview {
    pub fn create() -> windows::core::Result<std::rc::Rc<Preview>> {
        let p = CLASS.with(|cls| {
            win::create(
                win::CreateParams {
                    class: cls,
                    title: "Chup",
                    style: WS_POPUP,
                    // Không WS_EX_TOOLWINDOW: hiện trên taskbar và Alt+Tab để tìm lại khi không ghim.
                    ex_style: WS_EX_LAYERED,
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1,
                    parent: None,
                },
                |hwnd| Preview { hwnd, open: Cell::new(false), st: RefCell::new(State::default()) },
            )
        })?;
        util::set_app_icon(p.hwnd);
        Ok(p)
    }

    pub fn is_visible(&self) -> bool {
        self.open.get()
    }

    /// Bắt đầu mở `path` (App giải mã ở worker rồi gọi `show`).
    pub fn request(&self, path: PathBuf) {
        self.st.borrow_mut().pending = Some(path);
    }

    pub fn is_pending(&self, path: &PathBuf) -> bool {
        self.st.borrow().pending.as_ref() == Some(path)
    }

    pub fn cancel_pending(&self) {
        self.st.borrow_mut().pending = None;
    }

    /// Hiện ảnh vừa giải mã ở `view` (App tính chỗ và cỡ mở). Đang mở ảnh khác thì thay.
    pub fn show(&self, path: PathBuf, img: RgbaImage, view: View, work: Rect, topmost: bool, opacity: u8) {
        let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // Đặt cửa sổ (đang ẩn) vào chỗ đích trước khi vẽ, để GetDpiForWindow trả DPI của màn hình đó.
        let r = view.rect;
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, r.x, r.y, r.w as i32, r.h as i32, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        {
            let mut st = self.st.borrow_mut();
            st.pending = None;
            st.home_zoom = view.zoom;
            st.view = Some(view);
            st.work = work;
            st.resize = None;
            st.path = path;
            st.img = Some(img);
            // Ảnh mới: bitmap cũ không dùng được nữa.
            st.bmp = None;
            st.topmost = topmost;
            st.opacity = preview::clamp_opacity(opacity);
            st.cfg = Some((topmost, opacity));
            st.pill = None;
        }
        let t = wide(&format!("{title} — Chup"));
        unsafe {
            let _ = SetWindowTextW(self.hwnd, windows::core::PCWSTR(t.as_ptr()));
        }
        self.open.set(true);
        if !self.rescale() {
            self.close();
            emit(Action::Toast(chup_core::model::Toast::error("Không đủ bộ nhớ để mở preview")));
            return;
        }
        self.paint();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.apply_z(topmost);
    }

    /// Cài đặt đổi: áp phần Preview nếu khác lần trước.
    pub fn apply_config(&self, topmost: bool, opacity: u8) {
        let (z, repaint) = {
            let mut st = self.st.borrow_mut();
            let Some((old_top, old_op)) = st.cfg else { return };
            st.cfg = Some((topmost, opacity));
            let z = (old_top != topmost).then_some(topmost);
            if let Some(t) = z {
                st.topmost = t;
            }
            let repaint = old_op != opacity;
            if repaint {
                st.opacity = preview::clamp_opacity(opacity);
            }
            (z, repaint)
        };
        if let Some(t) = z {
            self.apply_z(t);
        }
        if z.is_some() || repaint {
            self.repaint();
        }
    }

    /// Ẩn và bỏ ảnh, DIB, bitmap.
    pub fn close(&self) {
        // Khung đang chờ lưu (vừa zoom xong) thì lưu luôn trước khi đóng.
        self.sync_pos();
        let unsaved = {
            let mut st = self.st.borrow_mut();
            let pending = std::mem::take(&mut st.save_pending);
            st.view.filter(|_| pending).map(|v| v.rect)
        };
        if let Some(r) = unsaved {
            emit(Action::PreviewBox(r));
        }
        let was_open = self.open.replace(false);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_PILL);
            let _ = KillTimer(Some(self.hwnd), TIMER_SAVE);
            let _ = KillTimer(Some(self.hwnd), TIMER_DPI);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        {
            let mut st = self.st.borrow_mut();
            st.view = None;
            // Đóng trong lúc ảnh khác đang giải mã: không tự mở lại khi giải mã xong.
            st.pending = None;
            st.bmp = None;
            st.canvas = None;
            st.base = None;
            st.out = None;
            st.img = None;
            st.cfg = None;
            st.hover = None;
            st.hovering = false;
            st.tracking = false;
            st.press = None;
            st.resize = None;
            st.pill = None;
            st.hits.clear();
        }
        if was_open {
            emit(Action::PreviewClosed);
        }
    }

    fn scale(&self) -> f32 {
        unsafe { GetDpiForWindow(self.hwnd) }.max(96) as f32 / 96.0
    }

    fn apply_z(&self, topmost: bool) {
        let after = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    /// Cửa sổ có thể đã bị kéo đi: đọc lại vị trí thật.
    fn sync_pos(&self) {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut r) }.is_err() {
            return;
        }
        if let Some(v) = self.st.borrow_mut().view.as_mut() {
            v.rect.x = r.left;
            v.rect.y = r.top;
        }
    }

    /// Vẽ lại `base` theo kích thước của view. `false` = không cấp được bộ nhớ.
    fn rescale(&self) -> bool {
        let theme = Theme::current();
        let k = self.scale();
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return false };
        let (w, h) = (v.rect.w as i32, v.rect.h as i32);
        if st.base.as_ref().map(|s| (s.w, s.h)) != Some((w, h)) {
            st.base = None;
            st.out = None;
            st.base = Surface::new(w, h);
            st.out = Surface::new(w, h);
        }
        if st.out.is_none() {
            st.out = Surface::new(w, h);
        }
        let Some(base) = st.base.as_ref() else { return false };
        if st.out.is_none() {
            return false;
        }
        if st.canvas.is_none() {
            st.canvas = Canvas::new(base);
            st.bmp = None;
        }
        let Some(c) = st.canvas.as_ref() else { return false };
        if st.bmp.is_none() {
            st.bmp = c.bitmap(img);
        }
        let Some(bmp) = st.bmp.as_ref() else { return false };
        if !c.bind(base) {
            return false;
        }
        let t0 = std::time::Instant::now();
        let rad = RADIUS * k;
        let full = rectf(0.0, 0.0, w as f32, h as f32);
        // Alpha 1/255 thay vì 0: pixel alpha 0 của layered window không nhận chuột, mà góc bo tròn
        // vẫn phải kéo được để đổi cỡ.
        c.begin(Color::rgba(0, 0, 0, 1.0 / 255.0));
        let ir = v.image_rect(img.dimensions());
        let fills = ir.w as i32 == w && ir.h as i32 == h;
        if !fills {
            // Ảnh nhỏ hơn cỡ tối thiểu: nằm giữa trên nền tối.
            c.fill_round(full, rad, Color::rgba(24, 24, 27, 1.0));
        }
        let img_r = rectf(ir.x as f32, ir.y as f32, ir.w as f32, ir.h as f32);
        c.image_cover(bmp, img_r, if fills { rad } else { 0.0 });
        c.stroke_round(rectf(0.5, 0.5, w as f32 - 1.0, h as f32 - 1.0), rad, 1.0, theme.line);
        c.end();
        log::debug!("preview: vẽ ảnh {}×{} mất {:?}", w, h, t0.elapsed());
        true
    }

    /// Chép `base` sang `out`, vẽ toolbar và nhãn, đẩy lên cửa sổ.
    /// `view.rect` phải là vị trí thật (gọi `repaint` nếu không chủ động đổi vị trí/cỡ).
    fn paint(&self) {
        let k = self.scale();
        // Lấy `out` ra khỏi state rồi mới đẩy lên cửa sổ: UpdateLayeredWindow có thể gửi ngược message
        // (WM_WINDOWPOSCHANGED, WM_DPICHANGED) vào wndproc, không được giữ borrow lúc đó.
        let Some((out, pos, alpha)) = self.compose(k) else { return };
        out.present(self.hwnd, pos, alpha, None);
        let mut st = self.st.borrow_mut();
        let same = st.base.as_ref().map(|b| (b.w, b.h)) == Some((out.w, out.h));
        if self.open.get() && st.out.is_none() && same {
            st.out = Some(out);
        }
    }

    /// Vẽ lại khi vị trí/cỡ không đổi chủ động (hover, nhãn, opacity, ghim): đọc lại vị trí thật trước,
    /// phòng khi cửa sổ đang bị kéo hoặc vừa bị dời bằng Win+Shift+mũi tên.
    fn repaint(&self) {
        self.sync_pos();
        self.paint();
    }

    fn compose(&self, k: f32) -> Option<(Surface, (i32, i32), u8)> {
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let v = st.view?;
        let mut owned = st.out.take()?;
        let (Some(base), Some(c)) = (st.base.as_mut(), st.canvas.as_ref()) else {
            st.out = Some(owned);
            return None;
        };
        let out = &mut owned;
        out.pixels().copy_from_slice(base.pixels());

        // Toolbar góc trên-phải (luôn tính vùng bấm; chỉ vẽ khi hover).
        let (w, h) = (out.w as f32, out.h as f32);
        let b = BTN * k;
        let n = BUTTONS.len() as f32;
        let x0 = w - BAR_PAD * k - n * b - (n - 1.0) * BTN_GAP * k;
        st.hits.clear();
        let draw_bar = st.hovering;
        let draw_pill = st.pill.is_some();
        if (draw_bar || draw_pill) && c.bind(out) {
            c.begin_over();
            for (i, (btn, icon)) in BUTTONS.iter().enumerate() {
                let r = rectf(x0 + i as f32 * (b + BTN_GAP * k), BAR_PAD * k, b, b);
                st.hits.push((*btn, Rect::new(r.left as i32, r.top as i32, b as u32, b as u32)));
                if !draw_bar {
                    continue;
                }
                let hot = st.hover == Some(*btn);
                let bg = if *btn == Btn::Pin && st.topmost {
                    Theme::current().accent.with_alpha(if hot { 1.0 } else { 0.9 })
                } else {
                    Color::rgba(0, 0, 0, if hot { 0.85 } else { 0.6 })
                };
                c.fill_round(r, 6.0 * k, bg);
                let is = 16.0 * k;
                c.icon(*icon, r.left + (b - is) / 2.0, r.top + (b - is) / 2.0, is, Color::rgba(255, 255, 255, 1.0));
            }
            if let (Some(text), Some(f)) = (st.pill.as_ref(), Font::new(12.0 * k, false, Align::Center, false)) {
                let (tw, th) = f.measure(text, 10_000.0);
                let (pw, ph) = (tw + 20.0 * k, th + 8.0 * k);
                let r = rectf((w - pw) / 2.0, h - 12.0 * k - ph, pw, ph);
                c.fill_round(r, ph / 2.0, Color::rgba(0, 0, 0, 0.7));
                c.text(text, &f, r, Color::rgba(255, 255, 255, 1.0));
            }
            c.end();
        } else {
            for (i, (btn, _)) in BUTTONS.iter().enumerate() {
                let x = x0 + i as f32 * (b + BTN_GAP * k);
                st.hits.push((*btn, Rect::new(x as i32, (BAR_PAD * k) as i32, b as u32, b as u32)));
            }
        }
        Some((owned, (v.rect.x, v.rect.y), preview::alpha(st.opacity)))
    }

    /// Báo App khung hiện tại (App lưu nếu đang chọn "như lần trước").
    fn save_box(&self) {
        self.sync_pos();
        let r = {
            let mut st = self.st.borrow_mut();
            st.save_pending = false;
            st.view.map(|v| v.rect)
        };
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SAVE);
        }
        if let Some(r) = r {
            emit(Action::PreviewBox(r));
        }
    }

    fn schedule_save(&self) {
        self.st.borrow_mut().save_pending = true;
        unsafe { SetTimer(Some(self.hwnd), TIMER_SAVE, SAVE_MS, None) };
    }

    /// Mép để đổi cỡ dưới điểm (toạ độ cửa sổ). Nút toolbar được ưu tiên.
    fn edge(&self, x: i32, y: i32) -> Option<Grip> {
        let band = (EDGE * self.scale()).round() as i32;
        let st = self.st.borrow();
        let v = st.view?;
        preview::edge_hit((v.rect.w, v.rect.h), (x, y), band)
    }

    fn hit(&self, x: i32, y: i32) -> Option<Btn> {
        self.st.borrow().hits.iter().find(|(_, r)| r.contains_point(x, y)).map(|(b, _)| *b)
    }

    fn show_pill(&self, text: String) {
        self.st.borrow_mut().pill = Some(text);
        unsafe { SetTimer(Some(self.hwnd), TIMER_PILL, PILL_MS, None) };
    }

    /// Work area của màn hình chứa tâm cửa sổ (người dùng có thể đã kéo sang màn khác).
    fn current_work(&self) -> Rect {
        let st = self.st.borrow();
        let Some(v) = st.view else { return st.work };
        let (cx, cy) = v.rect.center();
        monitor_at(&util::monitors(), cx, cy).map(|m| m.work).unwrap_or(st.work)
    }

    fn min_size(&self) -> (u32, u32) {
        min_size(self.scale() as f64)
    }

    /// Đổi zoom (`steps` nấc, hoặc về `to`) quanh `anchor`.
    fn zoom(&self, steps: i32, to: Option<f64>, anchor: (i32, i32)) {
        self.sync_pos();
        let work = self.current_work();
        let min = self.min_size();
        let zoom = {
            let mut st = self.st.borrow_mut();
            let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return };
            let dims = img.dimensions();
            let nv = match to {
                Some(z) => preview::resize_to(v, dims, z, anchor, work, min),
                None => preview::zoom_at(v, dims, steps, anchor, work, min),
            };
            st.view = Some(nv);
            st.work = work;
            nv.zoom
        };
        self.show_pill(format!("{}%", (zoom * 100.0).round()));
        if !self.rescale() {
            self.close();
            return;
        }
        self.paint();
        self.schedule_save();
    }

    /// Đang kéo mép: tính cỡ mới theo con trỏ.
    fn resize_move(&self) {
        let work = self.current_work();
        let min = self.min_size();
        {
            let mut st = self.st.borrow_mut();
            let (Some((grip, start, from)), Some(img)) = (st.resize, st.img.as_ref()) else { return };
            let nv = preview::drag_edge(start, img.dimensions(), grip, from, cursor_pos(), work, min);
            if Some(nv) == st.view {
                return;
            }
            st.view = Some(nv);
        }
        if !self.rescale() {
            self.close();
            return;
        }
        self.paint();
    }

    /// Kết thúc kéo mép (thả chuột hoặc mất capture).
    fn resize_end(&self) -> bool {
        if self.st.borrow_mut().resize.take().is_none() {
            return false;
        }
        self.save_box();
        true
    }

    fn center(&self) -> (i32, i32) {
        self.sync_pos();
        self.st.borrow().view.map(|v| v.rect.center()).unwrap_or_default()
    }

    fn change_opacity(&self, steps: i32) {
        let op = {
            let mut st = self.st.borrow_mut();
            st.opacity = preview::step_opacity(st.opacity, steps);
            st.opacity
        };
        self.show_pill(format!("Opacity {op}%"));
        self.repaint();
    }

    fn toggle_pin(&self) {
        let top = {
            let mut st = self.st.borrow_mut();
            st.topmost = !st.topmost;
            st.topmost
        };
        self.apply_z(top);
        self.repaint();
    }

    fn click(&self, b: Btn) {
        let path = self.st.borrow().path.clone();
        match b {
            Btn::Pin => self.toggle_pin(),
            Btn::Copy => emit(Action::PreviewCopy(path)),
            Btn::Folder => emit(Action::PreviewReveal(path)),
            Btn::Close => self.close(),
        }
    }
}

impl Handler for Preview {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if msg == WM_CLOSE {
            // Alt+F4, đóng từ taskbar, WM_CLOSE từ tool khác: ẩn chứ không huỷ cửa sổ (kể cả khi đang ẩn).
            self.close();
            return Some(LRESULT(0));
        }
        if !self.open.get() {
            return None;
        }
        match msg {
            WM_MOUSEMOVE => {
                if self.st.borrow().resize.is_some() {
                    self.resize_move();
                    return Some(LRESULT(0));
                }
                let h = self.hit(loword(lp.0), hiword(lp.0));
                let (changed, start_track) = {
                    let mut st = self.st.borrow_mut();
                    let entered = !std::mem::replace(&mut st.hovering, true);
                    let moved = std::mem::replace(&mut st.hover, h) != h;
                    (entered || moved, !std::mem::replace(&mut st.tracking, true))
                };
                if start_track {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                }
                if changed {
                    self.repaint();
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                {
                    let mut st = self.st.borrow_mut();
                    st.tracking = false;
                    st.hovering = false;
                    st.hover = None;
                }
                self.repaint();
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                let mut r = RECT::default();
                let _ = unsafe { GetWindowRect(hwnd, &mut r) };
                let (cx, cy) = cursor_pos();
                let (x, y) = (cx - r.left, cy - r.top);
                let resizing = self.st.borrow().resize.map(|(g, _, _)| g);
                let cursor = if let Some(g) = resizing.or_else(|| self.hit(x, y).is_none().then(|| self.edge(x, y)).flatten())
                {
                    match g {
                        Grip::N | Grip::S => IDC_SIZENS,
                        Grip::E | Grip::W => IDC_SIZEWE,
                        Grip::NW | Grip::SE => IDC_SIZENWSE,
                        _ => IDC_SIZENESW,
                    }
                } else if self.hit(x, y).is_some() {
                    IDC_HAND
                } else {
                    return None;
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                Some(LRESULT(1))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = (loword(lp.0), hiword(lp.0));
                if let Some(b) = self.hit(x, y) {
                    self.st.borrow_mut().press = Some(b);
                    unsafe { SetCapture(hwnd) };
                } else if let Some(g) = self.edge(x, y) {
                    self.sync_pos();
                    let start = {
                        let mut st = self.st.borrow_mut();
                        st.resize = st.view.map(|v| (g, v, cursor_pos()));
                        st.resize.is_some()
                    };
                    if start {
                        unsafe { SetCapture(hwnd) };
                    }
                } else {
                    // Kéo ở bất kỳ đâu ngoài nút: để Windows di chuyển cửa sổ.
                    let (cx, cy) = cursor_pos();
                    let pt = ((cy as u16 as isize) << 16) | (cx as u16 as isize);
                    unsafe {
                        let _ = ReleaseCapture();
                        SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(pt)));
                    }
                    // Vòng lặp di chuyển của Windows trả về khi thả chuột.
                    self.save_box();
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if self.resize_end() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    return Some(LRESULT(0));
                }
                let up = self.hit(loword(lp.0), hiword(lp.0));
                let press = self.st.borrow_mut().press.take();
                unsafe {
                    let _ = ReleaseCapture();
                }
                if let Some(b) = press.filter(|b| Some(*b) == up) {
                    self.click(b);
                }
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                self.st.borrow_mut().press = None;
                self.resize_end();
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = hiword(wp.0 as isize);
                let steps = {
                    let mut st = self.st.borrow_mut();
                    st.wheel += delta;
                    let s = st.wheel / 120;
                    st.wheel %= 120;
                    s
                };
                if steps != 0 {
                    if wp.0 & MK_CONTROL != 0 {
                        self.change_opacity(steps);
                    } else {
                        // lParam của WM_MOUSEWHEEL là toạ độ màn hình.
                        self.zoom(steps, None, (loword(lp.0), hiword(lp.0)));
                    }
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                let vk = wp.0 as u16;
                let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
                match vk {
                    v if v == VK_ESCAPE.0 => self.close(),
                    0x43 if ctrl => {
                        let path = self.st.borrow().path.clone();
                        emit(Action::PreviewCopy(path));
                    }
                    v if v == VK_OEM_PLUS.0 || v == VK_ADD.0 => self.zoom(1, None, self.center()),
                    v if v == VK_OEM_MINUS.0 || v == VK_SUBTRACT.0 => self.zoom(-1, None, self.center()),
                    v if v == 0x30 || v == VK_NUMPAD0.0 => {
                        let z = self.st.borrow().home_zoom;
                        self.zoom(0, Some(z), self.center());
                    }
                    _ => return None,
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_PILL => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_PILL);
                }
                self.st.borrow_mut().pill = None;
                self.repaint();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_SAVE => {
                self.save_box();
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                // Ảnh giữ nguyên physical px; chỉ nút, nhãn, bo góc đổi cỡ theo DPI mới. Vẽ lại sau.
                unsafe { SetTimer(Some(hwnd), TIMER_DPI, 1, None) };
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_DPI => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_DPI);
                }
                self.sync_pos();
                if !self.rescale() {
                    self.close();
                } else {
                    self.paint();
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
