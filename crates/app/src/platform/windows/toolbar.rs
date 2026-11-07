//! Toolbar cạnh vùng chọn: Chụp / Mờ / Chụp text / Huỷ (SPEC 5.1, 5.2, 15.2). Không bao giờ lấy focus.
//! Faded: chỉ còn icon, mờ 45% (SourceConstantAlpha), rõ lại khi hover.

use std::cell::RefCell;

use chup_core::coords::Rect;
use chup_ui::{
    toolbar::{live_items, paint_bar, LiveBtn as Btn, FADED_ALPHA, LIVE_BUTTONS},
    Canvas as _,
};
use windows::Win32::{
    UI::Controls::WM_MOUSELEAVE,
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT},
        WindowsAndMessaging::{
            SetWindowPos, ShowWindow, HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
            SW_SHOWNOACTIVATE, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
            WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

use super::{
    app::Action,
    gfx::{self, Align, Canvas, Color, Font, Surface},
    util::{hiword, loword},
    win::{self, emit, Handler},
};

#[derive(Default)]
struct State {
    surface: Option<Surface>,
    canvas: Option<Canvas>,
    rect: Rect,
    scale: f64,
    faded: bool,
    visible: bool,
    hover: Option<Btn>,
    pressed: Option<Btn>,
    tracking: bool,
    /// Chuột đang nằm trong toolbar (Faded: hiện rõ).
    inside: bool,
    buttons: Vec<(Btn, Rect)>,
}

pub struct Toolbar {
    pub hwnd: HWND,
    st: RefCell<State>,
}

thread_local! {
    static CLASS: Vec<u16> = win::register_class::<Toolbar>("ChupToolbar");
}

impl Toolbar {
    pub fn create() -> windows::core::Result<std::rc::Rc<Toolbar>> {
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
                |hwnd| Toolbar { hwnd, st: RefCell::new(State::default()) },
            )
        })
    }

    /// Đặt toolbar tại `rect` (physical) và hiện, không lấy focus.
    pub fn show(&self, rect: Rect, scale: f64, faded: bool) {
        let was_visible = {
            let mut st = self.st.borrow_mut();
            let resized = (st.rect.w, st.rect.h) != (rect.w, rect.h) || st.scale != scale;
            if resized {
                st.canvas = None;
                st.surface = None;
            }
            st.rect = rect;
            st.scale = scale;
            st.faded = faded;
            std::mem::replace(&mut st.visible, true)
        };
        self.render();
        if !was_visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        let mut st = self.st.borrow_mut();
        st.visible = false;
        st.inside = false;
        st.tracking = false;
        st.hover = None;
        st.pressed = None;
        st.canvas = None;
        st.surface = None;
    }

    /// Ẩn tạm (fallback ẩn UI lúc chụp) rồi hiện lại đúng chỗ cũ.
    pub fn set_hidden_for_capture(&self, hidden: bool) {
        if !self.st.borrow().visible {
            return;
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, if hidden { SW_HIDE } else { SW_SHOWNOACTIVATE });
        }
    }

    fn render(&self) {
        let mut st = self.st.borrow_mut();
        if !st.visible {
            return;
        }
        let (w, h) = (st.rect.w as i32, st.rect.h as i32);
        if st.surface.is_none() {
            st.surface = Surface::new(w, h);
            st.canvas = st.surface.as_ref().and_then(Canvas::new);
        }
        let k = st.scale as f32;
        let theme = gfx::theme();
        let font = Font::new(13.0 * k, false, Align::Leading, false);
        let faded = st.faded;
        let items = live_items(faded);
        let idx = |b: Option<Btn>| b.and_then(|b| LIVE_BUTTONS.iter().position(|x| *x == b));
        let (hover, pressed, inside) = (idx(st.hover), idx(st.pressed), st.inside);
        let Some(c) = st.canvas.as_ref() else { return };

        c.begin(Color::CLEAR);
        let rects = paint_bar(c, font.as_ref(), &theme, k, (w, h), &items, faded, hover, pressed);
        let buttons: Vec<(Btn, Rect)> = LIVE_BUTTONS.into_iter().zip(rects).collect();
        c.end();
        st.buttons = buttons;
        let alpha = if faded && !inside { FADED_ALPHA } else { 255 };
        if let Some(s) = st.surface.as_ref() {
            s.present(self.hwnd, (st.rect.x, st.rect.y), alpha, None);
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Btn> {
        self.st.borrow().buttons.iter().find(|(_, r)| r.contains_point(x, y)).map(|(b, _)| *b)
    }
}

impl Handler for Toolbar {
    fn handle(&self, hwnd: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_MOUSEMOVE => {
                let h = self.hit(loword(lp.0), hiword(lp.0));
                let (changed, need_track) = {
                    let mut st = self.st.borrow_mut();
                    let changed = st.hover != h || !st.inside;
                    st.hover = h;
                    st.inside = true;
                    (changed, !std::mem::replace(&mut st.tracking, true))
                };
                if need_track {
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
                    self.render();
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                {
                    let mut st = self.st.borrow_mut();
                    st.tracking = false;
                    st.inside = false;
                    st.hover = None;
                    st.pressed = None;
                }
                self.render();
                Some(LRESULT(0))
            }
            // CS_DBLCLKS: lần bấm thứ hai nhanh tới dưới dạng DBLCLK, coi như một lần bấm (chup-t8m).
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                self.st.borrow_mut().pressed = self.hit(loword(lp.0), hiword(lp.0));
                self.render();
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let up = self.hit(loword(lp.0), hiword(lp.0));
                let down = self.st.borrow_mut().pressed.take();
                self.render();
                if let Some(b) = up.filter(|_| up == down) {
                    emit(match b {
                        Btn::Capture => Action::Capture,
                        Btn::Fade => Action::ToggleFade,
                        Btn::Text => Action::CaptureText,
                        Btn::Close => Action::CloseRegion,
                    });
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
