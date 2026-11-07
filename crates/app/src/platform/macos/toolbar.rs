//! Toolbar cạnh vùng chọn: Chụp / Mờ / Chụp text / Huỷ (SPEC 5.1, 5.2, 15.2). Không bao giờ lấy focus.
//! Faded: chỉ còn icon, mờ (alpha của cửa sổ), rõ lại khi chuột vào.

use std::{cell::RefCell, rc::Rc};

use chup_core::coords::Rect;
use chup_ui::{
    toolbar::{live_items, paint_bar, LiveBtn as Btn, FADED_ALPHA, LIVE_BUTTONS},
    Canvas as _,
};
use objc2::MainThreadMarker;

use super::{
    app::Action,
    gfx::{self, Align, Canvas, Color, Font, Surface},
    panel::{Cursor, Ev, Handler, Opts, Panel, LEVEL_UI},
    win::emit,
};

#[derive(Default)]
struct State {
    rect: Rect,
    scale: f64,
    faded: bool,
    visible: bool,
    hover: Option<Btn>,
    pressed: Option<Btn>,
    inside: bool,
    buttons: Vec<(Btn, Rect)>,
}

pub struct Toolbar {
    panel: Panel,
    st: RefCell<State>,
}

impl Toolbar {
    pub fn create(mtm: MainThreadMarker) -> Rc<Toolbar> {
        let panel = Panel::new(mtm, Opts { key: false, level: LEVEL_UI, activating: false });
        let t = Rc::new(Toolbar { panel, st: RefCell::new(State::default()) });
        let weak: std::rc::Weak<dyn Handler> = Rc::downgrade(&t) as _;
        t.panel.set_handler(weak);
        t
    }

    /// Đặt toolbar tại `rect` (physical) và hiện, không lấy focus.
    pub fn show(&self, rect: Rect, scale: f64, faded: bool) {
        let was_visible = {
            let mut st = self.st.borrow_mut();
            st.rect = rect;
            st.scale = scale;
            st.faded = faded;
            std::mem::replace(&mut st.visible, true)
        };
        self.render();
        if !was_visible {
            self.panel.show(false);
        }
    }

    /// Tự kiểm tra (`CHUP_SELFTEST`): bơm sự kiện như thật và lấy ảnh đang hiện.
    pub fn inject(&self, ev: super::panel::Ev) {
        super::panel::Handler::event(self, ev);
    }

    pub fn snapshot(&self) -> Option<image::RgbaImage> {
        self.panel.snapshot()
    }

    pub fn hide(&self) {
        self.panel.hide();
        let mut st = self.st.borrow_mut();
        st.visible = false;
        st.inside = false;
        st.hover = None;
        st.pressed = None;
    }

    /// Ẩn tạm lúc chụp (fallback) rồi hiện lại đúng chỗ cũ.
    pub fn set_hidden_for_capture(&self, hidden: bool) {
        if !self.st.borrow().visible {
            return;
        }
        if hidden {
            self.panel.window.orderOut(None);
        } else {
            self.panel.show(false);
        }
    }

    fn render(&self) {
        let (rect, k, faded, hover, pressed, inside) = {
            let st = self.st.borrow();
            if !st.visible {
                return;
            }
            (st.rect, st.scale as f32, st.faded, st.hover, st.pressed, st.inside)
        };
        let (w, h) = (rect.w as i32, rect.h as i32);
        let Some(s) = Surface::new(w, h) else { return };
        let Some(c) = Canvas::new(&s) else { return };
        let font = Font::new(13.0 * k, false, Align::Leading, false);
        let items = live_items(faded);
        let idx = |b: Option<Btn>| b.and_then(|b| LIVE_BUTTONS.iter().position(|x| *x == b));
        c.begin(Color::CLEAR);
        let rects = paint_bar(&c, font.as_ref(), &gfx::theme(), k, (w, h), &items, faded, idx(hover), idx(pressed));
        c.end();
        self.st.borrow_mut().buttons = LIVE_BUTTONS.into_iter().zip(rects).collect();
        let alpha = if faded && !inside { FADED_ALPHA } else { 255 };
        self.panel.set_alpha(alpha as f64 / 255.0);
        self.panel.present(&s, rect);
    }

    fn hit(&self, x: i32, y: i32) -> Option<Btn> {
        self.st.borrow().buttons.iter().find(|(_, r)| r.contains_point(x, y)).map(|(b, _)| *b)
    }
}

impl Handler for Toolbar {
    fn event(&self, ev: Ev) {
        match ev {
            Ev::Move { p } | Ev::Drag { p } => {
                let h = self.hit(p.0, p.1);
                let changed = {
                    let mut st = self.st.borrow_mut();
                    let changed = st.hover != h || !st.inside;
                    st.hover = h;
                    st.inside = true;
                    changed
                };
                if changed {
                    self.render();
                }
            }
            Ev::Leave => {
                {
                    let mut st = self.st.borrow_mut();
                    st.inside = false;
                    st.hover = None;
                    st.pressed = None;
                }
                self.render();
            }
            Ev::Down { p, .. } => {
                self.st.borrow_mut().pressed = self.hit(p.0, p.1);
                self.render();
            }
            Ev::Up { p } => {
                let up = self.hit(p.0, p.1);
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
            }
            _ => {}
        }
    }

    fn cursor(&self, p: (i32, i32)) -> Cursor {
        if self.hit(p.0, p.1).is_some() {
            Cursor::Hand
        } else {
            Cursor::Arrow
        }
    }
}
