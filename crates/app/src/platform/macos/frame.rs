//! Viền vùng chọn (SPEC 5.2, 5.3). Cửa sổ = region nới ra `margin` mỗi cạnh; viền và handle nằm trong phần lề nên
//! không dính vào ảnh. Faded: cả cửa sổ bỏ qua chuột (`ignoresMouseEvents`).

use std::{cell::RefCell, rc::Rc};

use chup_core::{coords::Rect, model::FrameKey};
use chup_ui::frame::{self as ui_frame, Part};
use objc2::MainThreadMarker;

use super::{
    app::Action,
    gfx::{self, Surface},
    panel::{key, Cursor, Ev, Handler, Opts, Panel, LEVEL_UI},
    util::cursor_pos,
    win::emit,
};

struct Drag {
    part: Part,
    cursor: (i32, i32),
    outer: Rect,
}

#[derive(Default)]
struct State {
    outer: Rect,
    margin: i32,
    scale: f64,
    faded: bool,
    visible: bool,
    drag: Option<Drag>,
}

pub struct Frame {
    panel: Panel,
    st: RefCell<State>,
}

impl Frame {
    pub fn create(mtm: MainThreadMarker) -> Rc<Frame> {
        // Làm key window được (không activate app): nhận Enter / Esc sau khi chọn vùng (chup-48i). Bấm viền frame cũng
        // làm nó thành key lại.
        let panel = Panel::new(mtm, Opts { key: true, level: LEVEL_UI, activating: false });
        let f = Rc::new(Frame { panel, st: RefCell::new(State::default()) });
        let weak: std::rc::Weak<dyn Handler> = Rc::downgrade(&f) as _;
        f.panel.set_handler(weak);
        f
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
        self.panel.set_ignores_mouse(faded);
        self.render();
        self.panel.show(false);
    }

    /// Nhận bàn phím (Enter chụp, Esc đóng vùng): làm key window, không activate app. Bấm sang app khác là mất.
    pub fn take_keys(&self) {
        if self.st.borrow().visible {
            self.panel.show(true);
        }
    }

    /// Tự kiểm tra (`CHUP_SELFTEST`): bơm sự kiện như thật.
    pub fn inject(&self, ev: Ev) {
        Handler::event(self, ev);
    }

    /// Tự kiểm tra (`CHUP_SELFTEST`): lấy ảnh đang hiện.
    pub fn snapshot(&self) -> Option<image::RgbaImage> {
        self.panel.snapshot()
    }

    pub fn set_faded(&self, faded: bool) {
        self.st.borrow_mut().faded = faded;
        self.panel.set_ignores_mouse(faded);
        self.render();
    }

    pub fn hide(&self) {
        self.panel.hide();
        let mut st = self.st.borrow_mut();
        st.visible = false;
        st.drag = None;
    }

    fn render(&self) {
        let (outer, m, scale, faded) = {
            let st = self.st.borrow();
            (st.outer, st.margin, st.scale, st.faded)
        };
        let (w, h) = (outer.w as i32, outer.h as i32);
        let Some(mut s) = Surface::new(w, h) else { return };
        ui_frame::paint(&mut s.px(), (w, h), m, scale, faded, &gfx::theme(), true);
        self.panel.present(&s, outer);
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
        if resized {
            self.render();
        } else {
            self.panel.set_rect(next);
        }
        emit(Action::FrameMoved(next));
    }
}

impl Handler for Frame {
    fn event(&self, ev: Ev) {
        match ev {
            Ev::Down { p, .. } => {
                if let Some(part) = self.hit(p.0, p.1) {
                    let outer = self.st.borrow().outer;
                    self.st.borrow_mut().drag = Some(Drag { part, cursor: cursor_pos(), outer });
                }
            }
            Ev::Drag { .. } => {
                if self.st.borrow().drag.is_some() {
                    self.drag_to(cursor_pos());
                }
            }
            Ev::Up { .. } => {
                self.st.borrow_mut().drag = None;
            }
            Ev::Focus(on) => emit(Action::FrameFocus(on)),
            Ev::Key { code: key::RETURN | key::KEYPAD_ENTER, .. } => emit(Action::FrameKey(FrameKey::Enter)),
            Ev::Key { code: key::ESCAPE, .. } => emit(Action::FrameKey(FrameKey::Escape)),
            _ => {}
        }
    }

    fn cursor(&self, p: (i32, i32)) -> Cursor {
        let part = self.st.borrow().drag.as_ref().map(|d| d.part).or_else(|| self.hit(p.0, p.1));
        match part {
            Some(Part::Move) => Cursor::Move,
            Some(Part::N | Part::S) => Cursor::ResizeNS,
            Some(Part::E | Part::W) => Cursor::ResizeEW,
            Some(Part::NW | Part::SE) => Cursor::ResizeNWSE,
            Some(Part::NE | Part::SW) => Cursor::ResizeNESW,
            None => Cursor::Arrow,
        }
    }
}
