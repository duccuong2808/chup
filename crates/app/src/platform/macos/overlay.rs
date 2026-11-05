//! Overlay chọn vùng (SPEC 5.1) và Freeze (SPEC 14.3): mỗi màn hình một panel phủ cả màn hình, trên menu bar và
//! Dock. Logic giống vỏ Windows (`windows/overlay.rs`, `overlay/freeze.rs`); vẽ bằng `chup_ui::overlay`.
//!
//! - Surface cỡ màn hình chỉ cấp khi overlay hiện; ẩn Freeze là bỏ ngay (ảnh + DIB vài chục MB).
//! - Freeze: ảnh đóng băng 1:1, vùng chọn có 8 handle, toolbar [Lưu][Copy][Chữ][✕] vẽ trong overlay.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use chup_core::{
    autoselect::is_click,
    coords::{place_toolbar, MonitorInfo, Rect},
    freeze,
    model::FreezeAction,
    selection::{self, Grip},
};
use chup_ui::{
    overlay::{
        self as ui_overlay, BAR_GAP_DIP, BAR_H_DIP, FAILED_DIM, HANDLE_DIP, HANDLE_HIT_DIP, HINT_BG, HINT_FONT_DIP,
        HINT_PAD, HINT_TOP_DIP, LABEL_BG, LABEL_FONT_DIP, LABEL_PAD, LABEL_RADIUS_DIP,
    },
    toolbar::{bar_width, freeze_items, paint_bar},
    Canvas as _,
};
use image::RgbaImage;
use objc2::MainThreadMarker;

use super::{
    app::Action,
    gfx::{self, pill, Align, Canvas, Color, Font, PillPainter, Surface},
    panel::{key, Cursor, Ev, Handler, Opts, Panel, LEVEL_OVERLAY},
    win::emit,
};

/// Pixel "lỗ" của vùng chọn: macOS tự quyết nhận chuột theo cửa sổ (overlay luôn nhận), không cần mẹo alpha 1/255.
const HOLE_PX: u32 = 0x0100_0000;

struct Sprite {
    s: Surface,
    at: Rect,
}

#[derive(Clone, Copy)]
enum Drag {
    Idle,
    New((i32, i32)),
    Adjust { grip: Grip, from: (i32, i32), rect0: Rect },
    Button(usize),
}

/// Hành động của từng nút toolbar Freeze; `None` = huỷ.
const ACTIONS: [Option<FreezeAction>; 4] = [Some(FreezeAction::Save), Some(FreezeAction::Copy), Some(FreezeAction::Ocr), None];

struct Bar {
    canvas: Canvas,
    surface: Surface,
    font: Option<Font>,
    k: f32,
    at: Rect,
    buttons: Vec<Rect>,
    hover: Option<usize>,
    pressed: Option<usize>,
}

impl Bar {
    fn new(k: f32) -> Option<Bar> {
        let font = Font::new(13.0 * k, false, Align::Leading, false);
        let w = (bar_width(font.as_ref(), k, &freeze_items(), false) + 4.0 * k).ceil() as i32;
        let h = (BAR_H_DIP as f32 * k).ceil() as i32;
        let surface = Surface::new(w, h)?;
        let canvas = Canvas::new(&surface)?;
        let mut b = Bar { canvas, surface, font, k, at: Rect::default(), buttons: vec![], hover: None, pressed: None };
        b.render();
        Some(b)
    }

    fn render(&mut self) {
        let size = (self.surface.w, self.surface.h);
        self.canvas.begin(Color::CLEAR);
        self.buttons =
            paint_bar(&self.canvas, self.font.as_ref(), &gfx::theme(), self.k, size, &freeze_items(), false, self.hover, self.pressed);
        self.canvas.end();
    }

    fn hit(&self, p: (i32, i32)) -> Option<usize> {
        let (x, y) = (p.0 - self.at.x, p.1 - self.at.y);
        self.buttons.iter().position(|r| r.contains_point(x, y))
    }
}

struct Frozen {
    img: Option<Arc<RgbaImage>>,
    sel: Option<Rect>,
    drag: Drag,
    hint: Option<Sprite>,
    bar: Option<Bar>,
    handle: i32,
    ocr_only: bool,
}

impl Frozen {
    fn default_action(&self) -> FreezeAction {
        if self.ocr_only {
            FreezeAction::Ocr
        } else {
            FreezeAction::Save
        }
    }
    fn bar_visible(&self) -> bool {
        self.sel.is_some() && matches!(self.drag, Drag::Idle | Drag::Button(_))
    }
    fn bar_hit(&self, p: (i32, i32)) -> Option<usize> {
        self.bar.as_ref().filter(|_| self.bar_visible()).and_then(|b| b.hit(p))
    }
    fn hint_visible(&self) -> bool {
        self.sel.is_none() && matches!(self.drag, Drag::Idle)
    }
    fn reset_bar_state(&mut self) {
        if let Some(b) = self.bar.as_mut().filter(|b| b.hover.is_some() || b.pressed.is_some()) {
            b.hover = None;
            b.pressed = None;
            b.render();
        }
    }
}

enum Change {
    None,
    Bar,
    Sel,
}

#[derive(Default)]
struct State {
    surface: Option<Surface>,
    start: Option<(i32, i32)>,
    dragging: bool,
    sel: Option<Rect>,
    hint: Option<Sprite>,
    label: Option<PillPainter>,
    label_at: Option<Rect>,
    label_text: String,
    frozen: Option<Frozen>,
}

pub struct Overlay {
    pub mon: MonitorInfo,
    panel: Panel,
    st: RefCell<State>,
    frozen_mode: Cell<bool>,
}

impl Overlay {
    pub fn create(mtm: MainThreadMarker, mon: MonitorInfo) -> Rc<Overlay> {
        let panel = Panel::new(mtm, Opts { key: true, level: LEVEL_OVERLAY });
        panel.set_rect(mon.rect);
        let o = Rc::new(Overlay { mon, panel, st: RefCell::new(State::default()), frozen_mode: Cell::new(false) });
        let weak: std::rc::Weak<dyn Handler> = Rc::downgrade(&o) as _;
        o.panel.set_handler(weak);
        o
    }

    fn s(&self, dip: f64) -> i32 {
        (dip * self.mon.scale).round() as i32
    }

    fn size(&self) -> (i32, i32) {
        (self.mon.rect.w as i32, self.mon.rect.h as i32)
    }

    fn hint_sprite(&self, text: &str) -> Option<Sprite> {
        let k = self.mon.scale as f32;
        pill(text, HINT_FONT_DIP * k, HINT_PAD.0 * k, HINT_PAD.1 * k, 999.0, HINT_BG, Color::WHITE).map(|s| {
            let at = ui_overlay::hint_rect((s.w, s.h), self.size().0, self.s(HINT_TOP_DIP));
            Sprite { s, at }
        })
    }

    fn present(&self) {
        let st = self.st.borrow();
        if let Some(s) = st.surface.as_ref() {
            self.panel.present(s, self.mon.rect);
        }
    }

    /// Hiện overlay chọn vùng; `focus` = overlay dưới con trỏ (nhận phím Esc).
    pub fn show(&self, focus: bool) {
        let (w, h) = self.size();
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
        if self.st.borrow().hint.is_none() {
            let hint = self.hint_sprite(chup_core::i18n::t().hint_select);
            self.st.borrow_mut().hint = hint;
        }
        self.frozen_mode.set(false);
        {
            let mut st = self.st.borrow_mut();
            st.frozen = None;
            st.start = None;
            st.dragging = false;
            st.sel = None;
            st.label_at = None;
            st.label_text.clear();
            paint(&mut st, Rect::new(0, 0, w as u32, h as u32));
        }
        self.panel.set_rect(self.mon.rect);
        self.present();
        self.panel.show(focus);
        log::debug!("overlay {} {w}×{h}: hiện mất {:?}", self.mon.key, t0.elapsed());
    }

    /// Tự kiểm tra (`CHUP_SELFTEST`): bơm sự kiện như thật và lấy ảnh đang hiện.
    pub fn inject(&self, ev: super::panel::Ev) {
        super::panel::Handler::event(self, ev);
    }

    pub fn snapshot(&self) -> Option<image::RgbaImage> {
        self.panel.snapshot()
    }

    pub fn take_focus(&self) {
        self.panel.show(true);
    }

    pub fn hide(&self) {
        self.panel.hide();
        self.frozen_mode.set(false);
        let mut st = self.st.borrow_mut();
        st.start = None;
        st.dragging = false;
        st.sel = None;
        if st.frozen.take().is_some() {
            st.surface = None;
            st.label_at = None;
            st.label_text.clear();
        }
    }

    /// Giải phóng bộ đệm cỡ màn hình khi rảnh.
    pub fn release(&self) {
        if self.panel.is_visible() {
            return;
        }
        let mut st = self.st.borrow_mut();
        let hint = st.hint.take();
        *st = State { hint, ..State::default() };
    }

    fn label_painter(&self, max_w: f64) -> Option<PillPainter> {
        let k = self.mon.scale as f32;
        PillPainter::new(LABEL_FONT_DIP * k, LABEL_PAD.0 * k, LABEL_PAD.1 * k, LABEL_RADIUS_DIP * k, (self.s(max_w), self.s(30.0)))
    }

    fn place_label(&self, st: &mut State, sel: Rect) -> Option<Rect> {
        if st.label.is_none() {
            st.label = self.label_painter(160.0);
        }
        let text = format!("{} × {}", sel.w, sel.h);
        let mut size = st.label_at.map(|a| (a.w as i32, a.h as i32));
        if text != st.label_text || size.is_none() {
            if let Some(p) = st.label.as_mut() {
                size = Some(p.paint(&text, LABEL_BG, Color::WHITE));
            }
            st.label_text = text;
        }
        Some(ui_overlay::label_rect(sel, size?, self.size(), self.s(6.0), self.s(110.0)))
    }

    fn on_drag(&self, x: i32, y: i32) {
        let (w, h) = self.size();
        let (x, y) = (x.clamp(0, w), y.clamp(0, h));
        {
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
        }
        self.present();
    }

    fn live_event(&self, ev: Ev) {
        match ev {
            Ev::Down { p, .. } => {
                let dirty = {
                    let mut st = self.st.borrow_mut();
                    st.start = Some(p);
                    st.dragging = false;
                    st.hint.as_ref().map(|h| h.at)
                };
                if let Some(d) = dirty {
                    paint(&mut self.st.borrow_mut(), d);
                    self.present();
                }
            }
            Ev::Drag { p } => {
                let (start, dragging) = {
                    let st = self.st.borrow();
                    (st.start, st.dragging)
                };
                match start {
                    Some(s) if !dragging && is_click(s, p) => {}
                    Some(_) => {
                        self.st.borrow_mut().dragging = true;
                        self.on_drag(p.0, p.1);
                    }
                    None => {}
                }
            }
            Ev::Up { .. } => {
                let sel = {
                    let mut st = self.st.borrow_mut();
                    let had = st.start.take().is_some();
                    let dragging = std::mem::take(&mut st.dragging);
                    let sel = st.sel.take();
                    if had && dragging {
                        sel
                    } else {
                        None
                    }
                };
                match sel {
                    Some(r) => emit(Action::RegionSelected(r.offset(self.mon.rect.x, self.mon.rect.y))),
                    None => emit(Action::CancelSelection),
                }
            }
            Ev::RightUp { .. } => emit(Action::CancelSelection),
            Ev::Key { code: key::ESCAPE, .. } => emit(Action::CancelSelection),
            _ => {}
        }
    }

    // ---------- Freeze ----------

    /// Hiện ở chế độ Freeze. `img = None`: màn hình này chụp hỏng. `ocr_only`: "Chụp text".
    pub fn show_frozen(&self, focus: bool, img: Option<RgbaImage>, ocr_only: bool) -> bool {
        let (w, h) = self.size();
        let k = self.mon.scale as f32;
        let t = chup_core::i18n::t();
        let text = match (img.is_some(), ocr_only) {
            (false, _) => t.hint_freeze_failed,
            (true, true) => t.hint_ocr,
            (true, false) => t.hint_frozen,
        };
        let hint = self.hint_sprite(text);
        let bar = if ocr_only { None } else { Bar::new(k) };
        {
            let mut st = self.st.borrow_mut();
            if st.surface.is_none() {
                st.surface = Surface::new(w, h);
            }
            if st.surface.is_none() {
                log::error!("Không cấp được bộ nhớ cho overlay Freeze {w}×{h}");
                return false;
            }
            st.start = None;
            st.dragging = false;
            st.sel = None;
            st.label_at = None;
            st.label_text.clear();
            st.frozen = Some(Frozen {
                img: img.map(Arc::new),
                sel: None,
                drag: Drag::Idle,
                hint,
                bar,
                handle: self.s(HANDLE_DIP),
                ocr_only,
            });
            self.frozen_mode.set(true);
            paint(&mut st, Rect::new(0, 0, w as u32, h as u32));
        }
        self.panel.set_rect(self.mon.rect);
        self.present();
        self.panel.show(focus);
        true
    }

    pub fn take_frozen(&self) -> Option<Arc<RgbaImage>> {
        self.st.borrow_mut().frozen.as_mut()?.img.take()
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen_mode.get()
    }

    pub fn clear_frozen_selection(&self) {
        self.frozen_update(|f| {
            if f.sel.take().is_none() {
                return Change::None;
            }
            f.drag = Drag::Idle;
            f.reset_bar_state();
            Change::Sel
        });
    }

    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.mon.rect.w, self.mon.rect.h)
    }

    fn frozen_update(&self, f: impl FnOnce(&mut Frozen) -> Change) {
        {
            let mut st = self.st.borrow_mut();
            let before = extent(&st);
            let Some(fr) = st.frozen.as_mut() else { return };
            let dirty = match f(fr) {
                Change::None => return,
                Change::Bar => match fr.bar.as_ref() {
                    Some(b) => b.at,
                    None => return,
                },
                Change::Sel => {
                    let sel = fr.sel;
                    let dragging = matches!(fr.drag, Drag::New(_) | Drag::Adjust { .. });
                    let gap = fr.handle / 2 + self.s(BAR_GAP_DIP);
                    st.label_at = match sel {
                        Some(s) if dragging => self.place_label(&mut st, s),
                        _ => None,
                    };
                    let bounds = self.bounds();
                    if let (Some(s), Some(b)) = (sel, st.frozen.as_mut().and_then(|f| f.bar.as_mut())) {
                        let size = (b.surface.w as u32, b.surface.h as u32);
                        let (x, y) = place_toolbar(&s, size, &bounds, gap);
                        b.at = Rect::new(x, y, size.0, size.1);
                    }
                    before.union(&extent(&st))
                }
            };
            paint(&mut st, dirty);
        }
        self.present();
    }

    fn frozen_down(&self, p: (i32, i32), dbl: bool) {
        let hit_px = self.s(HANDLE_HIT_DIP);
        let mut started_new = false;
        let mut save = false;
        self.frozen_update(|f| {
            if f.img.is_none() {
                return Change::None;
            }
            if let Some(i) = f.bar_hit(p) {
                f.drag = Drag::Button(i);
                if let Some(b) = f.bar.as_mut() {
                    b.pressed = Some(i);
                    b.render();
                }
                return Change::Bar;
            }
            match f.sel.and_then(|s| selection::hit(s, p, hit_px)) {
                Some(Grip::Move) if dbl => {
                    save = true;
                    Change::None
                }
                Some(grip) => {
                    f.drag = Drag::Adjust { grip, from: p, rect0: f.sel.unwrap_or_default() };
                    f.reset_bar_state();
                    Change::Sel
                }
                None => {
                    f.drag = Drag::New(p);
                    f.sel = None;
                    f.reset_bar_state();
                    started_new = true;
                    Change::Sel
                }
            }
        });
        if started_new {
            emit(Action::FreezeSelecting(self.mon.key.clone()));
        }
        if save {
            let action = self.st.borrow().frozen.as_ref().map(Frozen::default_action);
            self.frozen_confirm(action);
        }
    }

    fn frozen_move(&self, p: (i32, i32)) {
        let b = self.bounds();
        let p = (p.0.clamp(0, b.right()), p.1.clamp(0, b.bottom()));
        self.frozen_update(|f| match f.drag {
            Drag::New(start) if f.sel.is_none() && is_click(start, p) => Change::None,
            Drag::New(start) => {
                let r = Rect::from_points(start, p);
                f.sel = (!r.is_empty()).then_some(r);
                Change::Sel
            }
            Drag::Adjust { grip, from, rect0 } => {
                f.sel = Some(selection::apply(rect0, grip, from, p, b));
                Change::Sel
            }
            Drag::Idle => {
                if !f.bar_visible() {
                    return Change::None;
                }
                let h = f.bar_hit(p);
                match f.bar.as_mut() {
                    Some(bar) if bar.hover != h => {
                        bar.hover = h;
                        bar.render();
                        Change::Bar
                    }
                    _ => Change::None,
                }
            }
            Drag::Button(_) => Change::None,
        });
    }

    fn frozen_end(&self, p: (i32, i32)) {
        let mut clicked = None;
        let mut auto_ocr = false;
        self.frozen_update(|f| match std::mem::replace(&mut f.drag, Drag::Idle) {
            Drag::Button(i) => {
                if f.bar_hit(p) == Some(i) {
                    clicked = Some(i);
                }
                if let Some(b) = f.bar.as_mut() {
                    b.pressed = None;
                    b.render();
                }
                Change::Bar
            }
            Drag::New(_) => {
                if f.sel.is_some_and(|s| s.w < 4 || s.h < 4) {
                    f.sel = None;
                }
                auto_ocr = f.ocr_only && f.sel.is_some();
                Change::Sel
            }
            Drag::Adjust { .. } => Change::Sel,
            Drag::Idle => Change::None,
        });
        if let Some(i) = clicked {
            self.frozen_confirm(ACTIONS[i]);
        } else if auto_ocr {
            self.frozen_confirm(Some(FreezeAction::Ocr));
        }
    }

    fn frozen_confirm(&self, action: Option<FreezeAction>) {
        let sel = self.st.borrow().frozen.as_ref().and_then(|f| f.sel);
        match (action, sel) {
            (None, _) => emit(Action::FreezeCancel),
            (Some(action), Some(r)) => {
                emit(Action::FreezeConfirm { rect: r.offset(self.mon.rect.x, self.mon.rect.y), action })
            }
            (Some(_), None) => {}
        }
    }

    fn frozen_cursor(&self, p: (i32, i32)) -> Cursor {
        let st = self.st.borrow();
        let Some(f) = st.frozen.as_ref() else { return Cursor::Cross };
        if f.bar_hit(p).is_some() {
            return Cursor::Hand;
        }
        match f.sel.and_then(|s| selection::hit(s, p, self.s(HANDLE_HIT_DIP))) {
            Some(Grip::Move) => Cursor::Move,
            Some(Grip::N | Grip::S) => Cursor::ResizeNS,
            Some(Grip::E | Grip::W) => Cursor::ResizeEW,
            Some(Grip::NW | Grip::SE) => Cursor::ResizeNWSE,
            Some(Grip::NE | Grip::SW) => Cursor::ResizeNESW,
            None => Cursor::Cross,
        }
    }

    fn frozen_event(&self, ev: Ev) {
        match ev {
            Ev::Down { p, clicks, .. } => self.frozen_down(p, clicks == 2),
            Ev::Drag { p } | Ev::Move { p } => self.frozen_move(p),
            Ev::Up { p } => self.frozen_end(p),
            Ev::RightUp { .. } => emit(Action::FreezeCancel),
            Ev::Key { code: key::ESCAPE, .. } => emit(Action::FreezeCancel),
            Ev::Key { code: key::RETURN | key::KEYPAD_ENTER, .. } => {
                let action = self.st.borrow().frozen.as_ref().map(Frozen::default_action);
                self.frozen_confirm(action);
            }
            _ => {}
        }
    }
}

impl Handler for Overlay {
    fn event(&self, ev: Ev) {
        if self.frozen_mode.get() {
            self.frozen_event(ev)
        } else {
            self.live_event(ev)
        }
    }

    fn cursor(&self, p: (i32, i32)) -> Cursor {
        if self.frozen_mode.get() {
            self.frozen_cursor(p)
        } else {
            Cursor::Cross
        }
    }
}

/// Phần overlay Freeze có thể đổi theo vùng chọn: vùng kèm handle, nhãn, toolbar, gợi ý.
fn extent(st: &State) -> Rect {
    let Some(f) = st.frozen.as_ref() else { return Rect::default() };
    let mut r = f.sel.map(|s| s.expand(f.handle + 2)).unwrap_or_default();
    let bar = f.bar.as_ref().filter(|_| f.bar_visible()).map(|b| b.at);
    let hint = f.hint.as_ref().filter(|_| f.hint_visible()).map(|h| h.at);
    for o in [st.label_at, bar, hint].into_iter().flatten() {
        r = r.union(&o);
    }
    r
}

/// Vẽ lại phần `dirty`.
fn paint(st: &mut State, dirty: Rect) {
    if st.frozen.is_some() {
        return paint_frozen(st, dirty);
    }
    let State { surface, sel, hint, label, label_at, start, .. } = st;
    let Some(s) = surface.as_mut() else { return };
    ui_overlay::paint_live(&mut s.px(), dirty, None, *sel, HOLE_PX);
    if start.is_none() {
        if let Some(h) = hint.as_ref() {
            if h.at.intersect(&dirty).is_some() {
                s.blend_from(&h.s, h.at.x, h.at.y, dirty);
            }
        }
    }
    if let (Some(p), Some(at)) = (label.as_ref(), label_at) {
        s.blend_part(&p.surface, (at.w as i32, at.h as i32), at.x, at.y, dirty);
    }
}

fn paint_frozen(st: &mut State, dirty: Rect) {
    let State { surface, frozen, label, label_at, .. } = st;
    let (Some(s), Some(f)) = (surface.as_mut(), frozen.as_mut()) else { return };
    let sw = s.w as usize;
    match &f.img {
        Some(img) => freeze::compose(img, s.pixels(), sw, dirty, f.sel),
        None => s.fill(dirty, FAILED_DIM.px()),
    }
    if let Some(sel) = f.sel {
        ui_overlay::paint_frozen_sel(&mut s.px(), dirty, sel, f.handle, !matches!(f.drag, Drag::New(_)));
    }
    if f.hint_visible() {
        if let Some(h) = f.hint.as_ref() {
            if h.at.intersect(&dirty).is_some() {
                s.blend_from(&h.s, h.at.x, h.at.y, dirty);
            }
        }
    }
    if let (Some(p), Some(at)) = (label.as_ref(), label_at.as_ref()) {
        s.blend_part(&p.surface, (at.w as i32, at.h as i32), at.x, at.y, dirty);
    }
    let show_bar = f.bar_visible();
    if let Some(b) = f.bar.as_ref().filter(|_| show_bar) {
        if b.at.intersect(&dirty).is_some() {
            s.blend_from(&b.surface, b.at.x, b.at.y, dirty);
        }
    }
}
