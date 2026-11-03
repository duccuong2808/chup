//! Toolbar cạnh vùng chọn: Chụp / Mờ / Chụp text / Huỷ (SPEC 5.1, 5.2, 15.2). Không bao giờ lấy focus.
//! Faded: chỉ còn icon, mờ 45% (SourceConstantAlpha), rõ lại khi hover.

use std::cell::RefCell;

use chup_core::{coords::Rect, icons::Icon};
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
    gfx::{rectf, Align, Canvas, Color, Font, Surface, Theme},
    util::{hiword, loword},
    win::{self, emit, Handler},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Capture,
    Fade,
    /// Chụp vùng rồi OCR, copy chữ (không lưu file).
    Text,
    Close,
}

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
        let theme = Theme::current();
        let font = Font::new(13.0 * k, false, Align::Leading, false);
        let faded = st.faded;
        let items = [
            BarButton { icon: Icon::Camera, label: chup_core::i18n::t().btn_capture, primary: true, active: false },
            BarButton { icon: Icon::Fade, label: chup_core::i18n::t().btn_fade, primary: false, active: faded },
            BarButton { icon: Icon::Text, label: "", primary: false, active: false },
            BarButton { icon: Icon::Close, label: "", primary: false, active: false },
        ];
        const BTNS: [Btn; 4] = [Btn::Capture, Btn::Fade, Btn::Text, Btn::Close];
        let idx = |b: Option<Btn>| b.and_then(|b| BTNS.iter().position(|x| *x == b));
        let (hover, pressed, inside) = (idx(st.hover), idx(st.pressed), st.inside);
        let Some(c) = st.canvas.as_ref() else { return };

        c.begin(Color::rgba(0, 0, 0, 0.0));
        let rects = paint_bar(c, font.as_ref(), &theme, k, (w, h), &items, faded, hover, pressed);
        let buttons: Vec<(Btn, Rect)> = BTNS.into_iter().zip(rects).collect();
        c.end();
        st.buttons = buttons;
        let alpha = if faded && !inside { 115 } else { 255 };
        if let Some(s) = st.surface.as_ref() {
            s.present(self.hwnd, (st.rect.x, st.rect.y), alpha, None);
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Btn> {
        self.st.borrow().buttons.iter().find(|(_, r)| r.contains_point(x, y)).map(|(b, _)| *b)
    }
}

/// 1 nút trên thanh. `primary`: nền màu nhấn. `active`: icon màu nhấn (nút Mờ khi đang Faded).
#[derive(Clone, Copy)]
pub struct BarButton {
    pub icon: Icon,
    pub label: &'static str,
    pub primary: bool,
    pub active: bool,
}

fn button_width(font: Option<&Font>, k: f32, b: &BarButton, icons_only: bool) -> f32 {
    if icons_only || b.label.is_empty() {
        28.0 * k
    } else {
        let tw = font.map(|f| f.measure(b.label, 1000.0).0).unwrap_or(30.0 * k);
        (10.0 + 16.0 + 6.0) * k + tw + 10.0 * k
    }
}

/// Chiều rộng thanh (physical) cho bộ nút `items`.
pub fn bar_width(font: Option<&Font>, k: f32, items: &[BarButton], icons_only: bool) -> f32 {
    4.0 * k * (items.len() as f32 + 1.0) + items.iter().map(|b| button_width(font, k, b, icons_only)).sum::<f32>()
}

/// Vẽ thanh nút căn phải trong khung `size` (giữa `Canvas::begin` và `end`). Trả về rect từng nút.
/// Bố cục (DIP): thanh cao 36, nút cao 28, đệm 4, cách mép phải 2. Dùng chung cho toolbar Live và Freeze.
#[allow(clippy::too_many_arguments)]
pub fn paint_bar(
    c: &Canvas,
    font: Option<&Font>,
    theme: &Theme,
    k: f32,
    size: (i32, i32),
    items: &[BarButton],
    icons_only: bool,
    hover: Option<usize>,
    pressed: Option<usize>,
) -> Vec<Rect> {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let (bh, bh_btn, pad) = (36.0 * k, 28.0 * k, 4.0 * k);
    let bar_w = bar_width(font, k, items, icons_only);
    let (bar_x, bar_y) = (w - bar_w - 2.0 * k, (h - bh) / 2.0);
    let by = bar_y + (bh - bh_btn) / 2.0;
    let mut x = bar_x + pad;
    let mut rects = Vec::with_capacity(items.len());
    for b in items {
        let bw = button_width(font, k, b, icons_only);
        rects.push(Rect::new(x as i32, by as i32, bw as u32, bh_btn as u32));
        x += bw + pad;
    }

    let bar = rectf(bar_x, bar_y, bar_w, bh);
    c.shadow(bar, 10.0 * k, 3.0 * k, 1.0 * k, theme.shadow);
    c.fill_round(bar, 10.0 * k, theme.surface);
    c.stroke_round(bar, 10.0 * k, 1.0, theme.line);
    for (i, (b, r)) in items.iter().zip(&rects).enumerate() {
        let rf = rectf(r.x as f32, r.y as f32, r.w as f32, r.h as f32);
        let down = if pressed == Some(i) { 1.0 * k } else { 0.0 };
        let hovered = hover == Some(i);
        let (bg, fg) = if b.primary {
            (Some(if hovered { theme.accent_strong } else { theme.accent }), Color::rgba(255, 255, 255, 1.0))
        } else if b.active {
            (hovered.then_some(theme.surface2), theme.accent)
        } else {
            (hovered.then_some(theme.surface2), theme.text)
        };
        if let Some(bg) = bg {
            c.fill_round(rf, 6.0 * k, bg);
        }
        let icon_sz = 16.0 * k;
        let iy = rf.top + (r.h as f32 - icon_sz) / 2.0 + down;
        if icons_only || b.label.is_empty() {
            c.icon(b.icon, rf.left + (r.w as f32 - icon_sz) / 2.0, iy, icon_sz, fg);
        } else {
            c.icon(b.icon, rf.left + 10.0 * k, iy, icon_sz, fg);
            if let Some(f) = font {
                let tx = rf.left + (10.0 + 16.0 + 6.0) * k;
                c.text(b.label, f, rectf(tx, rf.top + down, rf.right - tx, rf.bottom - rf.top), fg);
            }
        }
    }
    rects
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
