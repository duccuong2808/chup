//! Viền vùng chọn (SPEC 5.2, 5.3). Cửa sổ = region nới ra `margin` mỗi cạnh; mọi thứ nhìn thấy
//! nằm trong phần lề nên không bao giờ dính vào ảnh. Phần bên trong region có alpha 0 nên
//! chuột đi xuyên qua. Kéo/resize tự làm bằng SetCapture (không dùng vòng lặp modal của OS).

use std::cell::RefCell;

use chup_core::coords::Rect;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture},
        WindowsAndMessaging::{
            GetWindowLongPtrW, LoadCursorW, SetCursor, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE,
            HTCLIENT, HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE,
            IDC_SIZEWE, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE,
            WM_CAPTURECHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST,
            WM_SETCURSOR, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
            WS_POPUP,
        },
    },
};

use super::{
    app::Action,
    gfx::{Color, Surface, Theme},
    util::{cursor_pos, hiword, loword},
    win::{self, emit, Handler},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

struct Drag {
    part: Part,
    cursor: (i32, i32),
    outer: Rect,
}

#[derive(Default)]
struct State {
    surface: Option<Surface>,
    outer: Rect,
    margin: i32,
    scale: f64,
    faded: bool,
    visible: bool,
    drag: Option<Drag>,
}

pub struct Frame {
    pub hwnd: HWND,
    st: RefCell<State>,
}

thread_local! {
    static CLASS: Vec<u16> = win::register_class::<Frame>("ChupFrame");
}

impl Frame {
    pub fn create() -> windows::core::Result<std::rc::Rc<Frame>> {
        CLASS.with(|cls| {
            win::create(
                win::CreateParams {
                    class: cls,
                    title: "Chup",
                    style: WS_POPUP,
                    ex_style: WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1,
                    parent: None,
                },
                |hwnd| Frame { hwnd, st: RefCell::new(State::default()) },
            )
        })
    }

    pub fn show(&self, outer: Rect, margin: i32, scale: f64, faded: bool) {
        {
            let mut st = self.st.borrow_mut();
            st.outer = outer;
            st.margin = margin;
            st.scale = scale;
            st.faded = faded;
            st.visible = true;
            st.drag = None;
        }
        self.set_click_through(faded);
        self.render();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    pub fn set_faded(&self, faded: bool) {
        self.st.borrow_mut().faded = faded;
        self.set_click_through(faded);
        self.render();
    }

    pub fn hide(&self) {
        unsafe {
            if GetCapture() == self.hwnd {
                let _ = ReleaseCapture();
            }
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        let mut st = self.st.borrow_mut();
        st.visible = false;
        st.drag = None;
        st.surface = None;
    }

    /// Faded: cả cửa sổ click-through (WS_EX_TRANSPARENT).
    fn set_click_through(&self, on: bool) {
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let t = WS_EX_TRANSPARENT.0 as isize;
            let next = if on { ex | t } else { ex & !t };
            if next != ex {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, next);
            }
        }
    }

    fn sz(&self, scale: f64, dip: f64) -> i32 {
        (dip * scale).round().max(1.0) as i32
    }

    /// Vẽ lại và đẩy lên ở `st.outer`.
    fn render(&self) {
        let mut st = self.st.borrow_mut();
        let (w, h) = (st.outer.w as i32, st.outer.h as i32);
        // Cấp dư để resize không phải cấp lại DIB mỗi lần chuột di.
        let need = st.surface.as_ref().map(|s| s.w < w || s.h < h || s.w > w * 2 + 512 || s.h > h * 2 + 512);
        if need.unwrap_or(true) {
            st.surface = Surface::new(w + 256, h + 256);
        }
        let (m, scale, faded, outer) = (st.margin, st.scale, st.faded, st.outer);
        let theme = Theme::current();
        let bw = self.sz(scale, 2.0);
        let hs = self.sz(scale, 8.0);
        let hb = self.sz(scale, 1.5);
        let off = m - self.sz(scale, 3.0) - hs;
        let Some(s) = st.surface.as_mut() else { return };

        let all = Rect::new(0, 0, s.w as u32, s.h as u32);
        s.fill(all, 0);
        let region = Rect::new(m, m, (w - 2 * m).max(0) as u32, (h - 2 * m).max(0) as u32);
        let alpha = if faded { 0.3 } else { 1.0 };
        if !faded {
            // Lề nhận chuột (để kéo), gần như trong suốt.
            let ring = Rect::new(0, 0, w as u32, h as u32);
            s.fill(ring, 0x0100_0000);
            s.fill(region, 0);
        }
        // Viền màu nhấn nằm ngay ngoài region, viền trắng mảnh ngoài nữa.
        let band = region.expand(bw);
        s.outline_outside(band, 1, Color::rgba(255, 255, 255, 0.9 * alpha).px());
        s.outline_outside(region, bw, theme.accent.with_alpha(alpha).px());

        if !faded {
            for (hx, hy) in handle_origins(w, h, off, hs) {
                let r = Rect::new(hx, hy, hs as u32, hs as u32);
                s.fill(r, theme.accent.px());
                s.fill(r.expand(-hb), Color::rgba(255, 255, 255, 1.0).px());
            }
        }
        present_sized(s, self.hwnd, outer);
    }

    fn hit(&self, lx: i32, ly: i32) -> Option<Part> {
        let st = self.st.borrow();
        if st.faded || !st.visible {
            return None;
        }
        let (w, h, m) = (st.outer.w as i32, st.outer.h as i32, st.margin);
        let hs = self.sz(st.scale, 8.0);
        let off = m - self.sz(st.scale, 3.0) - hs;
        let pad = self.sz(st.scale, 3.0);
        let parts = [Part::NW, Part::N, Part::NE, Part::E, Part::SE, Part::S, Part::SW, Part::W];
        for (p, (hx, hy)) in parts.iter().zip(handle_origins(w, h, off, hs)) {
            if Rect::new(hx - pad, hy - pad, (hs + 2 * pad) as u32, (hs + 2 * pad) as u32).contains_point(lx, ly) {
                return Some(*p);
            }
        }
        let inner = Rect::new(m, m, (w - 2 * m).max(0) as u32, (h - 2 * m).max(0) as u32);
        if inner.contains_point(lx, ly) {
            return None;
        }
        Rect::new(0, 0, w as u32, h as u32).contains_point(lx, ly).then_some(Part::Move)
    }

    fn drag_to(&self, cur: (i32, i32)) {
        let next = {
            let st = self.st.borrow();
            let Some(d) = st.drag.as_ref() else { return };
            let (dx, dy) = (cur.0 - d.cursor.0, cur.1 - d.cursor.1);
            let o = d.outer;
            let min = 2 * st.margin + 8;
            let (mut l, mut t, mut r, mut b) = (o.x, o.y, o.right(), o.bottom());
            match d.part {
                Part::Move => {
                    l += dx;
                    r += dx;
                    t += dy;
                    b += dy;
                }
                p => {
                    if matches!(p, Part::W | Part::NW | Part::SW) {
                        l = (l + dx).min(r - min);
                    }
                    if matches!(p, Part::E | Part::NE | Part::SE) {
                        r = (r + dx).max(l + min);
                    }
                    if matches!(p, Part::N | Part::NW | Part::NE) {
                        t = (t + dy).min(b - min);
                    }
                    if matches!(p, Part::S | Part::SW | Part::SE) {
                        b = (b + dy).max(t + min);
                    }
                }
            }
            Rect::new(l, t, (r - l) as u32, (b - t) as u32)
        };
        let resized = {
            let mut st = self.st.borrow_mut();
            let resized = (next.w, next.h) != (st.outer.w, st.outer.h);
            st.outer = next;
            resized
        };
        if resized {
            self.render();
        } else if let Some(s) = self.st.borrow().surface.as_ref() {
            // Chỉ dời: đẩy lại bitmap cũ ở vị trí mới.
            present_sized(s, self.hwnd, next);
        }
        emit(Action::FrameMoved(next));
    }
}

/// 8 handle theo thứ tự NW, N, NE, E, SE, S, SW, W (toạ độ góc trên-trái trong cửa sổ).
fn handle_origins(w: i32, h: i32, off: i32, hs: i32) -> [(i32, i32); 8] {
    let (cx, cy) = ((w - hs) / 2, (h - hs) / 2);
    let (r, b) = (w - off - hs, h - off - hs);
    [(off, off), (cx, off), (r, off), (r, cy), (r, b), (cx, b), (off, b), (off, cy)]
}

/// Surface có thể lớn hơn cửa sổ (cấp dư): chỉ đẩy phần đúng kích thước cửa sổ.
fn present_sized(s: &Surface, hwnd: HWND, outer: Rect) {
    s.present_part(hwnd, (outer.x, outer.y), (outer.w as i32, outer.h as i32), 255);
}

impl Handler for Frame {
    fn handle(&self, hwnd: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_NCHITTEST => {
                let o = self.st.borrow().outer;
                let (x, y) = (loword(lp.0) - o.x, hiword(lp.0) - o.y);
                Some(LRESULT(if self.hit(x, y).is_some() { HTCLIENT as isize } else { HTTRANSPARENT as isize }))
            }
            WM_SETCURSOR => {
                let (cx, cy) = cursor_pos();
                let o = self.st.borrow().outer;
                let part = self.st.borrow().drag.as_ref().map(|d| d.part).or_else(|| self.hit(cx - o.x, cy - o.y));
                let id = match part {
                    Some(Part::Move) => IDC_SIZEALL,
                    Some(Part::N | Part::S) => IDC_SIZENS,
                    Some(Part::E | Part::W) => IDC_SIZEWE,
                    Some(Part::NW | Part::SE) => IDC_SIZENWSE,
                    Some(Part::NE | Part::SW) => IDC_SIZENESW,
                    None => IDC_ARROW,
                };
                unsafe { SetCursor(LoadCursorW(None, id).ok()) };
                Some(LRESULT(1))
            }
            WM_LBUTTONDOWN => {
                let (lx, ly) = (loword(lp.0), hiword(lp.0));
                if let Some(part) = self.hit(lx, ly) {
                    let outer = self.st.borrow().outer;
                    self.st.borrow_mut().drag = Some(Drag { part, cursor: cursor_pos(), outer });
                    unsafe { SetCapture(hwnd) };
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                if self.st.borrow().drag.is_some() {
                    self.drag_to(cursor_pos());
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if self.st.borrow_mut().drag.take().is_some() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                self.st.borrow_mut().drag = None;
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
