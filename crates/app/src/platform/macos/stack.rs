//! Stack thumbnail (SPEC 5.4). Không bao giờ lấy focus. Cửa sổ co giãn đúng theo nội dung, neo ở góc dưới của work
//! area. Kéo thẻ ≥ 4 px = kéo file PNG ra app khác; click = mở ảnh. Bố cục và vẽ ở `chup_ui::stack`.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use chup_core::{capture::Shot, coords::Rect, model::Toast};
use chup_ui::{
    stack::{self as ui_stack, Hit, StackFonts, StackState, ToastItem, DRAG_DIP, LEAVE_MS, MARGIN_DIP, MAX_TOASTS},
    Canvas as _,
};
use objc2::MainThreadMarker;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;

use super::{
    app::Action,
    gfx::{self, Canvas, Color, Font, Surface},
    panel::{Cursor, Ev, Handler, Opts, Panel, LEVEL_UI},
    util::cursor_pos,
    win::{self, emit, Timer},
};

struct Press {
    hit: Hit,
    at: (i32, i32),
}

#[derive(Default)]
struct State {
    shots: Vec<Shot>,
    edge_right: bool,
    scale: f64,
    work: Rect,
    toasts: Vec<ToastItem>,
    toast_timers: Vec<(u32, Timer)>,
    next_toast: u32,
    collapsed: bool,
    hovering: bool,
    hover: Option<Hit>,
    press: Option<Press>,
    visible: bool,
    hidden_for_capture: bool,
    images: HashMap<String, CFRetained<CGImage>>,
    hits: Vec<(Hit, Rect)>,
    leave: Option<Timer>,
}

pub struct StackWin {
    panel: Panel,
    st: RefCell<State>,
    me: RefCell<std::rc::Weak<StackWin>>,
}

impl StackWin {
    pub fn create(mtm: MainThreadMarker) -> Rc<StackWin> {
        let panel = Panel::new(mtm, Opts { key: false, level: LEVEL_UI });
        let s = Rc::new(StackWin {
            panel,
            st: RefCell::new(State { edge_right: true, scale: 1.0, ..Default::default() }),
            me: RefCell::new(std::rc::Weak::new()),
        });
        *s.me.borrow_mut() = Rc::downgrade(&s);
        let weak: std::rc::Weak<dyn Handler> = Rc::downgrade(&s) as _;
        s.panel.set_handler(weak);
        s
    }

    /// Danh sách shot hoặc vị trí đổi. `work` = work area của màn hình đặt stack.
    pub fn update(&self, shots: Vec<Shot>, edge_right: bool, work: Rect, scale: f64) {
        {
            let mut st = self.st.borrow_mut();
            if shots.iter().any(|s| !st.shots.iter().any(|o| o.id == s.id)) {
                st.collapsed = false;
            }
            st.images.retain(|id, _| shots.iter().any(|s| &s.id == id));
            st.shots = shots;
            st.edge_right = edge_right;
            st.work = work;
            st.scale = scale;
        }
        self.relayout();
    }

    /// Rect cửa sổ stack nếu đang hiện (Preview mở sát cạnh nó).
    pub fn window_rect(&self) -> Option<Rect> {
        let st = self.st.borrow();
        (st.visible && !st.hidden_for_capture).then(|| self.panel.rect())
    }

    pub fn toast(&self, t: Toast) {
        let id = {
            let mut st = self.st.borrow_mut();
            st.next_toast += 1;
            let id = st.next_toast;
            st.toasts.push(ToastItem { id, toast: t.clone() });
            if st.toasts.len() > MAX_TOASTS {
                let old = st.toasts.remove(0).id;
                st.toast_timers.retain(|(i, _)| *i != old);
            }
            id
        };
        let me = self.me.borrow().clone();
        let timer = win::after(ui_stack::toast_ms(&t), move || {
            if let Some(s) = me.upgrade() {
                s.remove_toast(id);
            }
        });
        self.st.borrow_mut().toast_timers.push((id, timer));
        self.relayout();
    }

    pub fn set_hidden_for_capture(&self, hidden: bool) {
        let visible = {
            let mut st = self.st.borrow_mut();
            st.hidden_for_capture = hidden;
            st.visible
        };
        if visible {
            if hidden {
                self.panel.window.orderOut(None);
            } else {
                self.panel.show(false);
            }
        }
    }

    /// Tự kiểm tra (`CHUP_SELFTEST`): bơm sự kiện như thật và lấy ảnh đang hiện.
    pub fn inject(&self, ev: super::panel::Ev) {
        super::panel::Handler::event(self, ev);
    }

    pub fn snapshot(&self) -> Option<image::RgbaImage> {
        self.panel.snapshot()
    }

    pub fn is_visible(&self) -> bool {
        self.st.borrow().visible
    }

    fn remove_toast(&self, id: u32) {
        let removed = {
            let mut st = self.st.borrow_mut();
            let before = st.toasts.len();
            st.toasts.retain(|t| t.id != id);
            st.toast_timers.retain(|(i, _)| *i != id);
            before != st.toasts.len()
        };
        if removed {
            self.relayout();
        }
    }

    fn relayout(&self) {
        let theme = gfx::theme();
        let mut st = self.st.borrow_mut();
        let k = st.scale as f32;
        let [body, small, btn] = ui_stack::font_specs(k).map(Font::from_spec);
        let fonts = StackFonts { body, small, btn };
        let layout = {
            let input = StackState {
                shots: &st.shots,
                toasts: &st.toasts,
                collapsed: st.collapsed,
                hovering: st.hovering,
                hover: st.hover,
                edge_right: st.edge_right,
                k,
            };
            ui_stack::layout(&input, &fonts)
        };
        let Some(layout) = layout else {
            let was = std::mem::replace(&mut st.visible, false);
            st.images.clear();
            st.hits.clear();
            drop(st);
            if was {
                self.panel.hide();
            }
            return;
        };
        let (w, h) = (layout.w, layout.h);
        let Some(s) = Surface::new(w, h) else { return };
        let Some(c) = Canvas::new(&s) else { return };
        let missing: Vec<usize> = (0..st.shots.len()).filter(|&i| !st.images.contains_key(&st.shots[i].id)).collect();
        for i in missing {
            if let Some(img) = c.image(&st.shots[i].thumb) {
                let id = st.shots[i].id.clone();
                st.images.insert(id, img);
            }
        }
        let hits = {
            let input = StackState {
                shots: &st.shots,
                toasts: &st.toasts,
                collapsed: st.collapsed,
                hovering: st.hovering,
                hover: st.hover,
                edge_right: st.edge_right,
                k,
            };
            c.begin(Color::CLEAR);
            let hits = ui_stack::paint(&c, &theme, &fonts, &input, &layout, |id| st.images.get(id));
            c.end();
            hits
        };
        let margin = (MARGIN_DIP * k).round() as i32;
        let (px, py) = chup_core::coords::place_stack((w as u32, h as u32), &st.work, st.edge_right, margin);
        st.hits = hits;
        let was_visible = std::mem::replace(&mut st.visible, true);
        let hidden = st.hidden_for_capture;
        drop(st);
        self.panel.present(&s, Rect::new(px, py, w as u32, h as u32));
        if !was_visible && !hidden {
            self.panel.show(false);
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        ui_stack::hit_at(&self.st.borrow().hits, x, y)
    }

    fn shot_id(&self, i: usize) -> Option<String> {
        self.st.borrow().shots.get(i).map(|s| s.id.clone())
    }

    fn begin_drag(&self, i: usize) {
        let Some(shot) = self.st.borrow().shots.get(i).cloned() else { return };
        if !shot.path.is_file() {
            emit(Action::ShotMissing(shot.id));
            return;
        }
        let image = drag::Image::File(shot.thumb_path.clone());
        let res = drag::start_drag(
            &self.panel.raw_handle(),
            drag::DragItem::Files(vec![shot.path.clone()]),
            image,
            |_, _| {},
            drag::Options::default(),
        );
        if let Err(e) = res {
            emit(Action::Toast(Toast::error(chup_core::i18n::fill(chup_core::i18n::t().err_drag, &[&e]))));
        }
    }

    fn set_hovering(&self, on: bool) {
        let changed = {
            let mut st = self.st.borrow_mut();
            st.leave = None;
            std::mem::replace(&mut st.hovering, on) != on
        };
        if changed {
            self.relayout();
        }
    }
}

impl Handler for StackWin {
    fn event(&self, ev: Ev) {
        match ev {
            Ev::Enter => self.set_hovering(true),
            Ev::Move { p } => {
                self.set_hovering(true);
                let h = self.hit(p.0, p.1);
                let changed = std::mem::replace(&mut self.st.borrow_mut().hover, h) != h;
                if changed {
                    self.relayout();
                }
            }
            Ev::Leave => {
                self.st.borrow_mut().hover = None;
                let me = self.me.borrow().clone();
                let t = win::after(LEAVE_MS, move || {
                    if let Some(s) = me.upgrade() {
                        s.set_hovering(false);
                    }
                });
                self.st.borrow_mut().leave = Some(t);
                self.relayout();
            }
            Ev::Down { p, .. } => {
                self.set_hovering(true);
                if let Some(hit) = self.hit(p.0, p.1) {
                    self.st.borrow_mut().press = Some(Press { hit, at: cursor_pos() });
                }
            }
            Ev::Drag { .. } => {
                let card = {
                    let st = self.st.borrow();
                    st.press.as_ref().and_then(|p| match p.hit {
                        Hit::Card(i) => {
                            let (cx, cy) = cursor_pos();
                            let d = (((cx - p.at.0).pow(2) + (cy - p.at.1).pow(2)) as f64).sqrt();
                            (d >= DRAG_DIP * st.scale).then_some(i)
                        }
                        _ => None,
                    })
                };
                if let Some(i) = card {
                    self.st.borrow_mut().press = None;
                    self.begin_drag(i);
                }
            }
            Ev::Up { p } => {
                let up = self.hit(p.0, p.1);
                let Some(press) = self.st.borrow_mut().press.take() else { return };
                if up != Some(press.hit) {
                    return;
                }
                match press.hit {
                    Hit::Card(i) => {
                        if let Some(id) = self.shot_id(i) {
                            emit(Action::ShotOpen(id));
                        }
                    }
                    Hit::Copy(i) => {
                        if let Some(id) = self.shot_id(i) {
                            emit(Action::ShotCopy(id));
                        }
                    }
                    Hit::Folder(i) => {
                        if let Some(id) = self.shot_id(i) {
                            emit(Action::ShotReveal(id));
                        }
                    }
                    Hit::Remove(i) => {
                        if let Some(id) = self.shot_id(i) {
                            emit(Action::StackRemove(id));
                        }
                    }
                    Hit::Clear => emit(Action::StackClear),
                    Hit::Collapse => {
                        self.st.borrow_mut().collapsed = true;
                        self.relayout();
                    }
                    Hit::Tab => {
                        self.st.borrow_mut().collapsed = false;
                        self.relayout();
                    }
                    Hit::Toast(id) => self.remove_toast(id),
                }
            }
            _ => {}
        }
    }

    fn cursor(&self, p: (i32, i32)) -> Cursor {
        match self.hit(p.0, p.1) {
            Some(Hit::Card(_)) | None => Cursor::Arrow,
            Some(_) => Cursor::Hand,
        }
    }
}
