//! Overlay chọn vùng (SPEC 5.1): mỗi màn hình 1 layered window, toạ độ physical px.
//!
//! - Surface (DIB cỡ màn hình) chỉ cấp khi overlay hiện, ẩn là giải phóng ngay.
//! - Khi kéo chỉ vẽ lại và đẩy phần thay đổi (vùng chọn cũ ∪ mới, nhãn kích thước).
//! - Vùng chọn có alpha 1/255 thay vì 0: pixel alpha 0 của layered window không nhận chuột.

mod freeze;

use std::cell::RefCell;

use chup_core::coords::{MonitorInfo, Rect};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN},
        WindowsAndMessaging::{
            LoadCursorW, SetCursor, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOPMOST, IDC_CROSS,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOW, SW_SHOWNA, WM_KEYDOWN, WM_LBUTTONDBLCLK,
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP, WM_SETCURSOR, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
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
const HINT: &str = "Kéo để chọn vùng · Esc hoặc chuột phải để huỷ";

struct Sprite {
    s: Surface,
    at: Rect,
}

#[derive(Default)]
struct State {
    surface: Option<Surface>,
    start: Option<(i32, i32)>,
    sel: Option<Rect>,
    hint: Option<Sprite>,
    /// Nhãn "W × H": vẽ lại vào cùng 1 bộ đệm, `label_at` là chỗ đặt trên overlay.
    label: Option<PillPainter>,
    label_at: Option<Rect>,
    label_text: String,
    /// Có giá trị khi overlay đang ở chế độ Freeze (SPEC 14).
    frozen: Option<freeze::Frozen>,
}

pub struct Overlay {
    pub hwnd: HWND,
    pub mon: MonitorInfo,
    st: RefCell<State>,
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
                |hwnd| Overlay { hwnd, mon, st: RefCell::new(State::default()) },
            )
        })
    }

    fn s(&self, dip: f64) -> i32 {
        (dip * self.mon.scale).round() as i32
    }

    /// Hiện overlay; `focus` = overlay dưới con trỏ (nhận phím Esc).
    /// DIB và dòng gợi ý được giữ lại giữa các lần chọn vùng liên tiếp; `release` mới giải phóng.
    pub fn show(&self, focus: bool) {
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
        if self.st.borrow().hint.is_none() {
            let k = self.mon.scale as f32;
            let white = Color::rgba(255, 255, 255, 1.0);
            let hint = pill(HINT, 13.0 * k, 12.0 * k, 6.0 * k, 999.0, Color::rgba(0, 0, 0, 0.7), white).map(|s| {
                let at = Rect::new((w - s.w) / 2, self.s(24.0), s.w as u32, s.h as u32);
                Sprite { s, at }
            });
            self.st.borrow_mut().hint = hint;
        }
        let t_hint = t0.elapsed();
        {
            let mut st = self.st.borrow_mut();
            st.frozen = None;
            st.start = None;
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
        let mut st = self.st.borrow_mut();
        st.start = None;
        st.sel = None;
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
        let hint = st.hint.take();
        *st = State { hint, ..State::default() };
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
    let State { surface, sel, hint, label, label_at, start, .. } = st;
    let Some(s) = surface.as_mut() else { return };
    s.fill(dirty, DIM.px());
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
}

impl Handler for Overlay {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if self.st.borrow().frozen.is_some() {
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
                    st.hint.as_ref().map(|h| h.at)
                };
                unsafe { SetCapture(hwnd) };
                // Ẩn dòng gợi ý khi bắt đầu kéo.
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
                if self.st.borrow().start.is_some() {
                    self.on_drag(loword(lp.0), hiword(lp.0));
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let sel = {
                    let mut st = self.st.borrow_mut();
                    let had = st.start.take().is_some();
                    had.then(|| st.sel.take()).flatten()
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
