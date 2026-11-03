//! Overlay chọn vùng (SPEC 5.1): mỗi màn hình 1 layered window, toạ độ physical px.
//!
//! - Surface (DIB cỡ màn hình) chỉ cấp khi overlay hiện, ẩn là giải phóng ngay.
//! - Khi kéo chỉ vẽ lại và đẩy phần thay đổi (vùng chọn cũ ∪ mới, nhãn kích thước).
//! - Vùng chọn có alpha 1/255 thay vì 0: pixel alpha 0 của layered window không nhận chuột.
//! - Tự động chọn vùng (SPEC 16, `auto.rs`): rê chuột là tô khung dưới con trỏ, click (≤ 4 px) là chọn khung đó.

mod auto;
mod freeze;

use std::cell::{Cell, RefCell};

use chup_core::{
    autoselect::is_click,
    coords::{MonitorInfo, Rect},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_UP},
        WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, LoadCursorW, SetCursor, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOPMOST, IDC_CROSS,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOW, SW_SHOWNA, WM_CAPTURECHANGED, WM_KEYDOWN,
            WM_KEYUP, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP,
            WM_SETCURSOR, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
            WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

use super::{
    app::Action,
    gfx::{pill, Color, PillPainter, Surface},
    util::{cursor_pos, hiword, loword},
    win::{self, emit, Handler},
};

const DIM: Color = Color::rgba(0, 0, 0, 0.4);
/// Gần như trong suốt nhưng vẫn nhận chuột.
const HOLE_PX: u32 = 0x0100_0000;

struct Sprite {
    s: Surface,
    at: Rect,
}

#[derive(Default)]
struct State {
    surface: Option<Surface>,
    /// Chỗ nhấn chuột trái (Live). Chưa di quá 4 px thì vẫn là click (chọn khung đang tô).
    start: Option<(i32, i32)>,
    /// Đã di quá 4 px từ `start`: đang kéo chọn vùng tự do, không tô khung tự động.
    dragging: bool,
    sel: Option<Rect>,
    hint: Option<Sprite>,
    /// Dòng gợi ý đang giữ là bản có tự động chọn vùng.
    hint_auto: bool,
    /// Nhãn "W × H": vẽ lại vào cùng 1 bộ đệm, `label_at` là chỗ đặt trên overlay.
    label: Option<PillPainter>,
    label_at: Option<Rect>,
    label_text: String,
    /// Có giá trị khi overlay đang ở chế độ Freeze (SPEC 14).
    frozen: Option<freeze::Frozen>,
    /// Tự động chọn vùng (SPEC 16); `None` khi tắt trong Cài đặt.
    auto: Option<auto::Auto>,
}

pub struct Overlay {
    pub hwnd: HWND,
    pub mon: MonitorInfo,
    st: RefCell<State>,
    /// Đang ở chế độ Freeze. Để ngoài `st`: wndproc đọc cờ này ở mọi message, kể cả message
    /// gửi đồng bộ khi đang giữ `borrow_mut` (ví dụ WM_WINDOWPOS* lúc `present` đổi vị trí cửa sổ).
    frozen_mode: Cell<bool>,
}

thread_local! {
    static CLASS: Vec<u16> = win::register_class::<Overlay>("ChupOverlay");
}

impl Overlay {
    pub fn create(mon: MonitorInfo) -> windows::core::Result<std::rc::Rc<Overlay>> {
        CLASS.with(|cls| {
            let r = mon.rect;
            win::create(
                win::CreateParams {
                    class: cls,
                    title: "Chup",
                    style: WS_POPUP,
                    ex_style: WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                    x: r.x,
                    y: r.y,
                    w: r.w as i32,
                    h: r.h as i32,
                    parent: None,
                },
                |hwnd| Overlay { hwnd, mon, st: RefCell::new(State::default()), frozen_mode: Cell::new(false) },
            )
        })
    }

    fn s(&self, dip: f64) -> i32 {
        (dip * self.mon.scale).round() as i32
    }

    /// Hiện overlay; `focus` = overlay dưới con trỏ (nhận phím Esc).
    /// DIB và dòng gợi ý được giữ lại giữa các lần chọn vùng liên tiếp; `release` mới giải phóng.
    pub fn show(&self, focus: bool, auto: bool) {
        let (w, h) = (self.mon.rect.w as i32, self.mon.rect.h as i32);
        let t0 = std::time::Instant::now();
        {
            let mut st = self.st.borrow_mut();
            if st.surface.is_none() {
                st.surface = Surface::new(w, h);
            }
            if st.surface.is_none() {
                log::error!("Không cấp được bộ nhớ cho overlay {w}×{h}");
                return;
            }
        }
        let t_alloc = t0.elapsed();
        if self.st.borrow().hint.is_none() || self.st.borrow().hint_auto != auto {
            let k = self.mon.scale as f32;
            let white = Color::rgba(255, 255, 255, 1.0);
            let text = if auto { chup_core::i18n::t().hint_select_auto } else { chup_core::i18n::t().hint_select };
            self.st.borrow_mut().hint_auto = auto;
            let hint = pill(text, 13.0 * k, 12.0 * k, 6.0 * k, 999.0, Color::rgba(0, 0, 0, 0.7), white).map(|s| {
                let at = Rect::new((w - s.w) / 2, self.s(24.0), s.w as u32, s.h as u32);
                Sprite { s, at }
            });
            self.st.borrow_mut().hint = hint;
        }
        let t_hint = t0.elapsed();
        {
            self.frozen_mode.set(false);
            let mut st = self.st.borrow_mut();
            st.frozen = None;
            st.auto = None;
            st.start = None;
            st.dragging = false;
            st.sel = None;
            st.label_at = None;
            st.label_text.clear();
            paint(&mut st, Rect::new(0, 0, w as u32, h as u32));
        }
        let t_paint = t0.elapsed();
        if let Some(s) = self.st.borrow().surface.as_ref() {
            s.present(self.hwnd, (self.mon.rect.x, self.mon.rect.y), 255, None);
        }
        let t_present = t0.elapsed();
        self.raise(focus);
        log::debug!(
            "overlay {} {w}×{h}: cấp {:?}, gợi ý {:?}, tô {:?}, đẩy {:?}, tổng {:?}",
            self.mon.key,
            t_alloc,
            t_hint - t_alloc,
            t_paint - t_hint,
            t_present - t_paint,
            t0.elapsed()
        );
    }

    /// Hiện cửa sổ trên cùng; `focus` = lấy focus để nhận phím.
    fn raise(&self, focus: bool) {
        unsafe {
            let _ = ShowWindow(self.hwnd, if focus { SW_SHOW } else { SW_SHOWNA });
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            if focus {
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(Some(self.hwnd));
                if GetForegroundWindow() != self.hwnd {
                    // Hẹn giờ Freeze: Chup không có input gần nhất nên Windows chặn SetForegroundWindow.
                    // Gắn tạm hàng đợi input với thread đang foreground rồi thử lại.
                    let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
                    let me = GetCurrentThreadId();
                    if fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool() {
                        let _ = SetForegroundWindow(self.hwnd);
                        let _ = SetFocus(Some(self.hwnd));
                        let _ = AttachThreadInput(me, fg_thread, false);
                    }
                    if GetForegroundWindow() != self.hwnd {
                        log::warn!("overlay {}: không lấy được focus, Esc/Enter có thể không tới", self.mon.key);
                    }
                }
            }
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            if windows::Win32::UI::Input::KeyboardAndMouse::GetCapture() == self.hwnd {
                let _ = ReleaseCapture();
            }
        }
        self.frozen_mode.set(false);
        let mut st = self.st.borrow_mut();
        st.start = None;
        st.dragging = false;
        st.sel = None;
        st.auto = None;
        // Hết phiên Freeze: bỏ ảnh và DIB ngay (4K ~66 MB mỗi màn hình), không đợi trim.
        if st.frozen.take().is_some() {
            st.surface = None;
            st.label_at = None;
            st.label_text.clear();
        }
    }

    /// Giải phóng DIB (khoảng 30 MB ở 4K) và bộ đệm chữ. Gọi khi app rảnh.
    pub fn release(&self) {
        if unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.hwnd) }.as_bool() {
            return;
        }
        // Giữ lại sprite gợi ý (vài trăm KB): tạo lại cần Direct2D, chậm khi vừa trim xong.
        let mut st = self.st.borrow_mut();
        let (hint, hint_auto) = (st.hint.take(), st.hint_auto);
        *st = State { hint, hint_auto, ..State::default() };
    }

    /// Vẽ nhãn "W × H" (physical px) cho `sel`, trả về chỗ đặt: dưới vùng chọn, sát đáy thì lên trên.
    fn place_label(&self, st: &mut State, sel: Rect) -> Option<Rect> {
        let (w, h) = (self.mon.rect.w as i32, self.mon.rect.h as i32);
        let k = self.mon.scale as f32;
        if st.label.is_none() {
            st.label = PillPainter::new(12.0 * k, 6.0 * k, 2.0 * k, 4.0 * k, (self.s(160.0), self.s(30.0)));
        }
        let text = format!("{} × {}", sel.w, sel.h);
        let mut size = st.label_at.map(|a| (a.w as i32, a.h as i32));
        if text != st.label_text || size.is_none() {
            if let Some(p) = st.label.as_mut() {
                size = Some(p.paint(&text, Color::rgba(0, 0, 0, 0.75), Color::rgba(255, 255, 255, 1.0)));
            }
            st.label_text = text;
        }
        let (lw, lh) = size?;
        let (gap, max_x) = (self.s(6.0), w - self.s(110.0));
        let below = sel.bottom() + gap;
        let ly = if below + lh > h { sel.y - gap - lh } else { below };
        Some(Rect::new(sel.x.min(max_x).max(0), ly.max(2), lw as u32, lh as u32))
    }

    fn on_drag(&self, x: i32, y: i32) {
        let (w, h) = (self.mon.rect.w as i32, self.mon.rect.h as i32);
        let (x, y) = (x.clamp(0, w), y.clamp(0, h));
        let mut st = self.st.borrow_mut();
        let Some(start) = st.start else { return };
        let sel = Rect::from_points(start, (x, y));
        let mut dirty = st.sel.map(|r| r.expand(2)).unwrap_or_default().union(&sel.expand(2));
        if let Some(at) = st.label_at {
            dirty = dirty.union(&at);
        }

        st.label_at = self.place_label(&mut st, sel);
        if let Some(at) = st.label_at {
            dirty = dirty.union(&at);
        }
        st.sel = Some(sel);
        paint(&mut st, dirty);
        if let Some(s) = st.surface.as_ref() {
            s.present(self.hwnd, (self.mon.rect.x, self.mon.rect.y), 255, Some(dirty));
        }
    }
}

/// Vẽ lại phần `dirty` của overlay: lớp tối, lỗ của vùng chọn, viền trắng, nhãn, gợi ý.
fn paint(st: &mut State, dirty: Rect) {
    if st.frozen.is_some() {
        return freeze::paint_frozen(st, dirty);
    }
    let hl = auto::highlight(st).filter(|_| !st.dragging);
    let State { surface, sel, hint, label, label_at, start, .. } = st;
    let Some(s) = surface.as_mut() else { return };
    s.fill(dirty, DIM.px());
    // Khung tự động: bên trong sáng như vùng đang kéo.
    if let Some(inside) = hl.and_then(|h| h.intersect(&dirty)) {
        s.fill(inside, HOLE_PX);
    }
    if let Some(sel) = sel {
        if let Some(hole) = sel.intersect(&dirty) {
            s.fill(hole, HOLE_PX);
        }
        s.outline_outside(*sel, 1, Color::rgba(255, 255, 255, 0.95).px());
    }
    if start.is_none() {
        if let Some(h) = hint.as_mut() {
            if h.at.intersect(&dirty).is_some() {
                s.blend_from(&mut h.s, h.at.x, h.at.y);
            }
        }
    }
    if let (Some(p), Some(at)) = (label.as_mut(), label_at) {
        s.blend_part(&mut p.surface, (at.w as i32, at.h as i32), at.x, at.y);
    }
    if hl.is_some() {
        auto::paint_auto_edges(st);
    }
}

/// Bước cuộn chuột: 1 nấc (120) = 1 cấp; chuột cuộn mịn (< 120) vẫn tính 1 cấp theo chiều cuộn.
fn wheel_steps(wp: WPARAM) -> i32 {
    let d = (wp.0 >> 16) as u16 as i16 as i32;
    if d.abs() >= 120 {
        d / 120
    } else {
        d.signum()
    }
}

impl Handler for Overlay {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        // Đổi cấp khung (SPEC 16.2 bước 3) và Shift (bước 5): cuộn chuột và phím chỉ tới overlay đang có focus,
        // nên gửi qua App để tới overlay đang chứa con trỏ.
        match msg {
            WM_MOUSEWHEEL => {
                emit(Action::AutoLevel(wheel_steps(wp)));
                return Some(LRESULT(0));
            }
            WM_KEYDOWN if wp.0 as u16 == VK_UP.0 => {
                emit(Action::AutoLevel(1));
                return Some(LRESULT(0));
            }
            WM_KEYDOWN if wp.0 as u16 == VK_DOWN.0 => {
                emit(Action::AutoLevel(-1));
                return Some(LRESULT(0));
            }
            WM_KEYDOWN | WM_KEYUP if wp.0 as u16 == VK_SHIFT.0 => {
                emit(Action::AutoLevel(0));
                return Some(LRESULT(0));
            }
            _ => {}
        }
        if self.frozen_mode.get() {
            return self.handle_frozen(hwnd, msg, wp, lp);
        }
        match msg {
            WM_SETCURSOR => {
                unsafe { SetCursor(LoadCursorW(None, IDC_CROSS).ok()) };
                Some(LRESULT(1))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = (loword(lp.0), hiword(lp.0));
                let dirty = {
                    let mut st = self.st.borrow_mut();
                    st.start = Some((x, y));
                    st.dragging = false;
                    st.hint.as_ref().map(|h| h.at)
                };
                unsafe { SetCapture(hwnd) };
                // Ẩn dòng gợi ý khi nhấn chuột.
                if let Some(d) = dirty {
                    let mut st = self.st.borrow_mut();
                    paint(&mut st, d);
                    if let Some(s) = st.surface.as_ref() {
                        s.present(hwnd, (self.mon.rect.x, self.mon.rect.y), 255, Some(d));
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let p = (loword(lp.0), hiword(lp.0));
                let (start, dragging) = {
                    let st = self.st.borrow();
                    (st.start, st.dragging)
                };
                match start {
                    // Nhấn rồi di chưa quá 4 px: vẫn là click.
                    Some(s) if !dragging && is_click(s, p) => {}
                    Some(_) => {
                        if !dragging {
                            self.st.borrow_mut().dragging = true;
                            self.auto_hover(p, false);
                        }
                        self.on_drag(p.0, p.1);
                    }
                    None => self.auto_hover(p, true),
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let sel = {
                    let mut st = self.st.borrow_mut();
                    let had = st.start.take().is_some();
                    let dragging = std::mem::take(&mut st.dragging);
                    let sel = st.sel.take();
                    match (had, dragging) {
                        (true, true) => sel,
                        // Click: lấy khung đang tô (SPEC 16.2 bước 4).
                        (true, false) => st.auto.as_ref().and_then(auto::Auto::shown),
                        _ => None,
                    }
                };
                unsafe {
                    let _ = ReleaseCapture();
                }
                match sel {
                    Some(r) => emit(Action::RegionSelected(r.offset(self.mon.rect.x, self.mon.rect.y))),
                    None => emit(Action::CancelSelection),
                }
                Some(LRESULT(0))
            }
            // Mất capture giữa lúc nhấn / kéo (Alt+Tab, Win...): bỏ lần kéo này, overlay vẫn mở để chọn lại.
            WM_CAPTURECHANGED if lp.0 != hwnd.0 as isize => {
                let dirty = {
                    let mut st = self.st.borrow_mut();
                    if st.start.take().is_none() {
                        return Some(LRESULT(0));
                    }
                    st.dragging = false;
                    let mut d = st.sel.take().map(|r| r.expand(2)).unwrap_or_default();
                    if let Some(at) = st.label_at.take() {
                        d = d.union(&at);
                    }
                    d
                };
                if !dirty.is_empty() {
                    let mut st = self.st.borrow_mut();
                    paint(&mut st, dirty);
                    if let Some(s) = st.surface.as_ref() {
                        s.present(hwnd, (self.mon.rect.x, self.mon.rect.y), 255, Some(dirty));
                    }
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                emit(Action::CancelSelection);
                Some(LRESULT(0))
            }
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
                emit(Action::CancelSelection);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
