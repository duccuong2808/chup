//! Viền vùng chọn (SPEC 5.2, 5.3). Cửa sổ = region nới ra `margin` mỗi cạnh; mọi thứ nhìn thấy
//! nằm trong phần lề nên không bao giờ dính vào ảnh. Phần bên trong region có alpha 0 nên
//! chuột đi xuyên qua. Kéo/resize tự làm bằng SetCapture (không dùng vòng lặp modal của OS).

use std::cell::RefCell;

use chup_core::coords::Rect;
use chup_ui::frame::{self as ui_frame, Part};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture},
        WindowsAndMessaging::{
            GetWindowLongPtrW, LoadCursorW, SetCursor, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE,
            HTCLIENT, HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE,
            IDC_SIZEWE, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE,
            WM_CAPTURECHANGED, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST,
            WM_SETCURSOR, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
            WS_POPUP,
        },
    },
};

use super::{
    app::Action,
    gfx::{self, Surface},
    util::{cursor_pos, hiword, loword},
    win::{self, emit, Handler},
};

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

    /// Vẽ lại và đẩy lên ở `st.outer`. Ngoài lúc kéo/resize thì không giữ DIB:
    /// DWM đã có bản sao của layered window, giữ thêm bản của mình chỉ tốn RAM.
    fn render(&self) {
        let surface = self.paint();
        let (outer, dragging) = {
            let st = self.st.borrow();
            (st.outer, st.drag.is_some())
        };
        if let Some(s) = surface {
            present_sized(&s, self.hwnd, outer);
            if dragging {
                self.st.borrow_mut().surface = Some(s);
            }
        }
    }

    /// Vẽ vào surface (lấy ra khỏi state để không giữ borrow lúc gọi Win32).
    fn paint(&self) -> Option<Surface> {
        let mut st = self.st.borrow_mut();
        let (w, h) = (st.outer.w as i32, st.outer.h as i32);
        // Cấp dư để resize không phải cấp lại DIB mỗi lần chuột di.
        let need = st.surface.as_ref().map(|s| s.w < w || s.h < h || s.w > w * 2 + 512 || s.h > h * 2 + 512);
        if need.unwrap_or(true) {
            st.surface = Surface::new(w + 256, h + 256);
        }
        let (m, scale, faded) = (st.margin, st.scale, st.faded);
        let mut surface = st.surface.take()?;
        drop(st);
        ui_frame::paint(&mut surface.px(), (w, h), m, scale, faded, &gfx::theme(), true);
        Some(surface)
    }

    fn hit(&self, lx: i32, ly: i32) -> Option<Part> {
        let st = self.st.borrow();
        if !st.visible {
            return None;
        }
        ui_frame::hit((st.outer.w as i32, st.outer.h as i32), st.margin, st.scale, st.faded, lx, ly)
    }

    fn drag_to(&self, cur: (i32, i32)) {
        let next = {
            let st = self.st.borrow();
            let Some(d) = st.drag.as_ref() else { return };
            ui_frame::drag(d.part, d.outer, cur.0 - d.cursor.0, cur.1 - d.cursor.1, st.margin)
        };
        let resized = {
            let mut st = self.st.borrow_mut();
            let resized = (next.w, next.h) != (st.outer.w, st.outer.h);
            st.outer = next;
            resized
        };
        let kept = if resized { None } else { self.st.borrow_mut().surface.take() };
        match kept {
            // Chỉ dời: đẩy lại bitmap cũ ở vị trí mới.
            Some(s) => {
                present_sized(&s, self.hwnd, next);
                self.st.borrow_mut().surface = Some(s);
            }
            None => self.render(),
        }
        emit(Action::FrameMoved(next));
    }
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
            // CS_DBLCLKS: nắm lại khung ngay sau khi vừa thả tới dưới dạng DBLCLK (chup-t8m).
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
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
                // Kéo xong: không cần giữ DIB nữa.
                self.st.borrow_mut().surface = None;
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                let mut st = self.st.borrow_mut();
                st.drag = None;
                st.surface = None;
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
