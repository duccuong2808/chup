//! Overlay chọn vùng (SPEC 5.1): mỗi màn hình 1 layered window, toạ độ physical px.
//!
//! - Surface (DIB cỡ màn hình) chỉ cấp khi overlay hiện, ẩn là giải phóng ngay.
//! - Khi kéo chỉ vẽ lại và đẩy phần thay đổi (vùng chọn cũ ∪ mới, nhãn kích thước).
//! - Vùng chọn có alpha 1/255 thay vì 0: pixel alpha 0 của layered window không nhận chuột.

use std::cell::RefCell;

use chup_core::coords::{MonitorInfo, Rect};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE},
        WindowsAndMessaging::{
            LoadCursorW, SetCursor, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOPMOST, IDC_CROSS,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOW, SW_SHOWNA, WM_KEYDOWN, WM_LBUTTONDOWN,
            WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP, WM_SETCURSOR, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
            WS_POPUP,
        },
    },
};

use super::{
    app::Action,
    gfx::{pill, Color, Surface},
    util::{hiword, loword},
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
    label: Option<Sprite>,
    label_text: String,
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
    pub fn show(&self, focus: bool) {
        let (w, h) = (self.mon.rect.w as i32, self.mon.rect.h as i32);
        let t0 = std::time::Instant::now();
        let Some(surface) = Surface::new(w, h) else {
            log::error!("Không cấp được bộ nhớ cho overlay {w}×{h}");
            return;
        };
        let scale = self.mon.scale as f32;
        let hint = pill(
            HINT,
            13.0 * scale,
            12.0 * scale,
            6.0 * scale,
            999.0,
            Color::rgba(0, 0, 0, 0.7),
            Color::rgba(255, 255, 255, 1.0),
        )
        .map(|s| {
            let at = Rect::new((w - s.w) / 2, self.s(24.0), s.w as u32, s.h as u32);
            Sprite { s, at }
        });
        {
            let mut st = self.st.borrow_mut();
            *st = State { surface: Some(surface), hint, ..State::default() };
            paint(&mut st, Rect::new(0, 0, w as u32, h as u32));
            if let Some(s) = st.surface.as_ref() {
                s.present(self.hwnd, (self.mon.rect.x, self.mon.rect.y), 255, None);
            }
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, if focus { SW_SHOW } else { SW_SHOWNA });
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            if focus {
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(Some(self.hwnd));
            }
        }
        log::debug!("overlay {} hiện sau {:?}", self.mon.key, t0.elapsed());
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        // Giải phóng DIB (vài chục MB ở 4K) ngay khi ẩn.
        *self.st.borrow_mut() = State::default();
    }

    fn on_drag(&self, x: i32, y: i32) {
        let (w, h) = (self.mon.rect.w as i32, self.mon.rect.h as i32);
        let (x, y) = (x.clamp(0, w), y.clamp(0, h));
        let mut st = self.st.borrow_mut();
        let Some(start) = st.start else { return };
        let sel = Rect::from_points(start, (x, y));
        let mut dirty = st.sel.map(|r| r.expand(2)).unwrap_or_default().union(&sel.expand(2));
        if let Some(l) = &st.label {
            dirty = dirty.union(&l.at);
        }

        // Nhãn "W × H" (physical px), dưới vùng chọn hoặc lên trên nếu sát đáy.
        let text = format!("{} × {}", sel.w, sel.h);
        if text != st.label_text {
            let k = self.mon.scale as f32;
            st.label = pill(&text, 12.0 * k, 6.0 * k, 2.0 * k, 4.0 * k, Color::rgba(0, 0, 0, 0.75), Color::rgba(255, 255, 255, 1.0))
                .map(|s| Sprite { at: Rect::new(0, 0, s.w as u32, s.h as u32), s });
            st.label_text = text;
        }
        let (gap, max_x) = (self.s(6.0), w - self.s(110.0));
        if let Some(l) = st.label.as_mut() {
            let below = sel.bottom() + gap;
            let ly = if below + l.at.h as i32 > h { sel.y - gap - l.at.h as i32 } else { below };
            l.at = Rect::new(sel.x.min(max_x).max(0), ly.max(2), l.at.w, l.at.h);
            dirty = dirty.union(&l.at);
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
    let State { surface, sel, hint, label, start, .. } = st;
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
    if let Some(l) = label.as_mut() {
        s.blend_from(&mut l.s, l.at.x, l.at.y);
    }
}

impl Handler for Overlay {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
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
