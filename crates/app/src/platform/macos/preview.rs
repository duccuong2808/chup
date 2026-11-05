//! Cửa sổ Preview (SPEC 17): ảnh to khi bấm thumbnail. Port từ `windows/preview.rs`, vẽ bằng `chup_ui::preview`.
//! Một cửa sổ duy nhất, tạo lúc khởi động; ảnh, surface và ảnh CoreGraphics chỉ tồn tại khi đang mở.
//!
//! Cửa sổ rộng hơn `view.rect` một lề `margin` mỗi phía để vẽ bóng đổ. Toạ độ "cục bộ" = so với góc trên-trái
//! của `view.rect` (toạ độ trong cửa sổ trừ lề).

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    sync::Arc,
};

use chup_core::{
    coords::{monitor_at, Rect},
    ocr::{Pos, TextLayout},
    preview::{self, View},
    selection::Grip,
};
use chup_ui::{
    preview::{self as ui_preview, Btn, EDGE, PILL_MS, TEXT_PAD},
    Canvas as _,
};
use image::RgbaImage;
use objc2::{define_class, msg_send, rc::Retained, runtime::AnyObject, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSMenu, NSMenuItem};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSObject, NSPoint, NSString};

use super::{
    app::Action,
    gfx::{self, Canvas, Color, Font, Surface},
    panel::{key, Cursor, Ev, Handler, Mods, Opts, Panel, LEVEL_FLOATING, LEVEL_NORMAL},
    util::{self, cursor_pos},
    win::{self, emit, Timer},
};

const CMD_COPY_TEXT: isize = 1;
const CMD_SELECT_ALL: isize = 2;
const CMD_COPY_IMAGE: isize = 3;
/// Trackpad: bao nhiêu point cuộn thì đổi 1 nấc phóng to.
const SCROLL_STEP_PT: f64 = 24.0;

#[derive(Default)]
struct State {
    pending: Option<PathBuf>,
    path: PathBuf,
    img: Option<RgbaImage>,
    view: Option<View>,
    margin: i32,
    work: Rect,
    topmost: bool,
    opacity: u8,
    cfg: Option<(bool, u8)>,
    base: Option<Surface>,
    image: Option<CFRetained<CGImage>>,
    hover: Option<Btn>,
    hovering: bool,
    press: Option<Btn>,
    pill: Option<String>,
    pill_timer: Option<Timer>,
    hits: Vec<(Btn, Rect)>,
    scroll: f64,
    resize: Option<(Grip, View, (i32, i32))>,
    pan: Option<(View, (i32, i32))>,
    /// Đang kéo nền để di chuyển cửa sổ: rect cửa sổ lúc bấm, con trỏ lúc bấm.
    moving: Option<(Rect, (i32, i32))>,
    ocr_busy: Option<PathBuf>,
    text: Option<Arc<TextLayout>>,
    sel: Option<(Pos, Pos)>,
    text_press: Option<(Pos, (i32, i32))>,
    selecting: bool,
}

impl State {
    fn clear_text(&mut self) {
        self.text = None;
        self.sel = None;
        self.text_press = None;
        self.selecting = false;
    }
}

pub struct MenuIvars {
    owner: RefCell<Weak<Preview>>,
}

define_class!(
    // SAFETY: NSObject không có yêu cầu riêng khi kế thừa; không cài Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ChupPreviewMenuTarget"]
    #[ivars = MenuIvars]
    pub struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(onMenu:))]
        fn on_menu(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let owner = self.ivars().owner.borrow().upgrade();
            if let Some(p) = owner {
                p.on_menu(tag);
            }
        }
    }
);

pub struct Preview {
    mtm: MainThreadMarker,
    panel: Panel,
    open: Cell<bool>,
    st: RefCell<State>,
    menu_target: Retained<MenuTarget>,
    me: RefCell<Weak<Preview>>,
}

impl Preview {
    pub fn create(mtm: MainThreadMarker) -> Rc<Preview> {
        let panel = Panel::new(mtm, Opts { key: true, level: LEVEL_FLOATING });
        let t = MenuTarget::alloc(mtm).set_ivars(MenuIvars { owner: RefCell::new(Weak::new()) });
        let menu_target: Retained<MenuTarget> = unsafe { msg_send![super(t), init] };
        let p = Rc::new(Preview {
            mtm,
            panel,
            open: Cell::new(false),
            st: RefCell::new(State::default()),
            menu_target,
            me: RefCell::new(Weak::new()),
        });
        *p.me.borrow_mut() = Rc::downgrade(&p);
        *p.menu_target.ivars().owner.borrow_mut() = Rc::downgrade(&p);
        let weak: Weak<dyn Handler> = Rc::downgrade(&p) as _;
        p.panel.set_handler(weak);
        p
    }

    pub fn is_visible(&self) -> bool {
        self.open.get()
    }

    pub fn request(&self, path: PathBuf) {
        self.st.borrow_mut().pending = Some(path);
    }

    pub fn is_pending(&self, path: &PathBuf) -> bool {
        self.st.borrow().pending.as_ref() == Some(path)
    }

    pub fn cancel_pending(&self) {
        self.st.borrow_mut().pending = None;
    }

    fn scale(&self) -> f32 {
        let r = self.st.borrow().view.map(|v| v.rect).unwrap_or_default();
        let (cx, cy) = r.center();
        util::layout().scale_at_px(cx, cy) as f32
    }

    /// Hiện ảnh vừa giải mã ở `view`. Đang mở ảnh khác thì thay.
    pub fn show(&self, path: PathBuf, img: RgbaImage, view: View, work: Rect, topmost: bool, opacity: u8) {
        {
            let mut st = self.st.borrow_mut();
            st.pending = None;
            st.view = Some(view);
            st.work = work;
            st.resize = None;
            st.pan = None;
            st.moving = None;
            st.path = path;
            st.img = Some(img);
            st.image = None;
            st.topmost = topmost;
            st.opacity = preview::clamp_opacity(opacity);
            st.cfg = Some((topmost, opacity));
            st.pill = None;
            st.clear_text();
        }
        self.open.set(true);
        if !self.rescale() {
            self.close();
            emit(Action::Toast(chup_core::model::Toast::error(chup_core::i18n::t().err_preview_memory)));
            return;
        }
        self.paint();
        self.apply_z(topmost);
        self.panel.show(true);
    }

    pub fn apply_config(&self, topmost: bool, opacity: u8) {
        let (z, repaint) = {
            let mut st = self.st.borrow_mut();
            let Some((old_top, old_op)) = st.cfg else { return };
            st.cfg = Some((topmost, opacity));
            let z = (old_top != topmost).then_some(topmost);
            if let Some(t) = z {
                st.topmost = t;
            }
            let repaint = old_op != opacity;
            if repaint {
                st.opacity = preview::clamp_opacity(opacity);
            }
            (z, repaint)
        };
        if let Some(t) = z {
            self.apply_z(t);
        }
        if z.is_some() || repaint {
            self.paint();
        }
    }

    pub fn close(&self) {
        let was_open = self.open.replace(false);
        self.panel.hide();
        {
            let mut st = self.st.borrow_mut();
            let pending = None;
            *st = State { pending, ..State::default() };
        }
        if was_open {
            emit(Action::PreviewClosed);
        }
    }

    fn apply_z(&self, topmost: bool) {
        self.panel.set_level(if topmost { LEVEL_FLOATING } else { LEVEL_NORMAL });
    }

    /// Vẽ lại lớp nền theo view. `false` = không cấp được bộ nhớ.
    fn rescale(&self) -> bool {
        let k = self.scale();
        let theme = gfx::theme();
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return false };
        let m = ui_preview::margin(k);
        st.margin = m;
        let (w, h) = (v.rect.w as i32 + 2 * m, v.rect.h as i32 + 2 * m);
        let Some(base) = Surface::new(w, h) else { return false };
        let Some(c) = Canvas::new(&base) else { return false };
        if st.image.is_none() {
            st.image = c.image(img);
        }
        let Some(image) = st.image.as_ref() else { return false };
        c.begin(Color::CLEAR);
        ui_preview::paint_base(&c, &theme, v, img.dimensions(), image, m, k, false);
        c.end();
        st.base = Some(base);
        true
    }

    /// Chép lớp nền, vẽ vùng chọn chữ, toolbar, nhãn, rồi đưa lên cửa sổ.
    fn paint(&self) {
        let k = self.scale();
        let out = {
            let mut st = self.st.borrow_mut();
            let st = &mut *st;
            let Some(v) = st.view else { return };
            let Some(base) = st.base.as_ref() else { return };
            let m = st.margin;
            let (w, h) = (v.rect.w as f32, v.rect.h as f32);
            st.hits = ui_preview::bar_hits(w, k);
            let sel: Vec<_> = match (st.text.as_ref(), st.sel, st.img.as_ref()) {
                (Some(t), Some(sel), Some(img)) => ui_preview::sel_rects(t, sel, v, img.dimensions(), m, k),
                _ => vec![],
            };
            let Some(mut out) = Surface::new(base.w, base.h) else { return };
            out.pixels().copy_from_slice(base.data());
            if let Some(c) = Canvas::new(&out) {
                let pill_font = st.pill.as_ref().and_then(|_| Font::from_spec(ui_preview::pill_font(k)));
                let top = ui_preview::Top {
                    k,
                    m,
                    size: (w, h),
                    sel: &sel,
                    bar: st.hovering,
                    hover: st.hover,
                    busy: st.ocr_busy.as_ref() == Some(&st.path),
                    topmost: st.topmost,
                    pill: st.pill.as_deref().zip(pill_font.as_ref()),
                };
                c.begin_over();
                ui_preview::paint_top(&c, &gfx::theme(), &top);
                c.end();
            }
            let pos = v.rect.expand(m);
            self.panel.set_alpha(preview::alpha(st.opacity) as f64 / 255.0);
            (out, pos)
        };
        self.panel.present(&out.0, out.1);
    }

    /// Cửa sổ có thể đã bị kéo đi: đọc lại vị trí.
    fn sync_pos(&self) {
        let r = self.panel.rect();
        let mut st = self.st.borrow_mut();
        let m = st.margin;
        if let Some(v) = st.view.as_mut() {
            v.rect.x = r.x + m;
            v.rect.y = r.y + m;
        }
    }

    fn local(&self, p: (i32, i32)) -> (i32, i32) {
        let m = self.st.borrow().margin;
        (p.0 - m, p.1 - m)
    }

    fn save_box(&self) {
        self.sync_pos();
        if let Some(r) = self.st.borrow().view.map(|v| v.rect) {
            emit(Action::PreviewBox(r));
        }
    }

    fn edge(&self, x: i32, y: i32) -> Option<Grip> {
        let band = (EDGE * self.scale()).round() as i32;
        let st = self.st.borrow();
        let v = st.view?;
        preview::edge_hit((v.rect.w, v.rect.h), (x, y), band, st.margin)
    }

    fn hit(&self, x: i32, y: i32) -> Option<Btn> {
        self.st.borrow().hits.iter().find(|(_, r)| r.contains_point(x, y)).map(|(b, _)| *b)
    }

    fn show_pill(&self, text: String) {
        let me = self.me.borrow().clone();
        let t = win::after(PILL_MS, move || {
            if let Some(p) = me.upgrade() {
                p.st.borrow_mut().pill = None;
                p.paint();
            }
        });
        let mut st = self.st.borrow_mut();
        st.pill = Some(text);
        st.pill_timer = Some(t);
    }

    fn current_work(&self) -> Rect {
        let st = self.st.borrow();
        let Some(v) = st.view else { return st.work };
        let (cx, cy) = v.rect.center();
        monitor_at(&util::layout().monitors(), cx, cy).map(|m| m.work).unwrap_or(st.work)
    }

    fn magnify(&self, steps: Option<i32>, anchor: (i32, i32)) {
        let scale = {
            let mut st = self.st.borrow_mut();
            let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return };
            let nv = match steps {
                Some(n) => preview::magnify_at(v, img.dimensions(), n, anchor),
                None => preview::unmagnify(v),
            };
            st.view = Some(nv);
            nv.scale()
        };
        self.show_pill(format!("{}%", (scale * 100.0).round()));
        self.redraw();
    }

    fn redraw(&self) {
        self.sync_pos();
        if !self.rescale() {
            self.close();
            return;
        }
        self.paint();
    }

    fn local_center(&self) -> (i32, i32) {
        self.st.borrow().view.map(|v| (v.rect.w as i32 / 2, v.rect.h as i32 / 2)).unwrap_or_default()
    }

    fn resize_move(&self) {
        let work = self.current_work();
        let min = ui_preview::min_size(self.scale() as f64);
        {
            let mut st = self.st.borrow_mut();
            let (Some((grip, start, from)), Some(img)) = (st.resize, st.img.as_ref()) else { return };
            let nv = preview::drag_edge(start, img.dimensions(), grip, from, cursor_pos(), work, min);
            if Some(nv) == st.view {
                return;
            }
            st.view = Some(nv);
        }
        if !self.rescale() {
            self.close();
            return;
        }
        self.paint();
    }

    fn pan_move(&self) {
        {
            let mut st = self.st.borrow_mut();
            let (Some((start, from)), Some(img)) = (st.pan, st.img.as_ref()) else { return };
            let (cx, cy) = cursor_pos();
            let nv = preview::pan(start, img.dimensions(), cx - from.0, cy - from.1);
            if Some(nv) == st.view {
                return;
            }
            st.view = Some(nv);
        }
        self.redraw();
    }

    fn change_opacity(&self, steps: i32) {
        let op = {
            let mut st = self.st.borrow_mut();
            st.opacity = preview::step_opacity(st.opacity, steps);
            st.opacity
        };
        self.show_pill(format!("Opacity {op}%"));
        self.paint();
    }

    fn toggle_pin(&self) {
        let top = {
            let mut st = self.st.borrow_mut();
            st.topmost = !st.topmost;
            st.topmost
        };
        self.apply_z(top);
        self.paint();
    }

    pub fn set_ocr_busy(&self, busy: Option<PathBuf>) {
        let changed = {
            let mut st = self.st.borrow_mut();
            let changed = st.ocr_busy != busy;
            st.ocr_busy = busy;
            changed
        };
        if changed && self.open.get() {
            self.paint();
        }
    }

    pub fn set_text(&self, path: &Path, text: Arc<TextLayout>) {
        let had_sel = {
            let mut st = self.st.borrow_mut();
            if !self.open.get() || st.path != path {
                return;
            }
            let had = st.sel.is_some();
            st.clear_text();
            st.text = Some(text);
            had
        };
        if had_sel {
            self.paint();
        }
    }

    fn to_image(&self, x: i32, y: i32) -> Option<(Arc<TextLayout>, (i32, i32), i32)> {
        let k = self.scale();
        let st = self.st.borrow();
        let (t, v, img) = (st.text.as_ref()?, st.view?, st.img.as_ref()?);
        if t.is_empty() {
            return None;
        }
        let (ix, iy) = v.to_image(img.dimensions(), (x as f64, y as f64));
        let p = (ix.floor() as i32, iy.floor() as i32);
        let pad = (TEXT_PAD as f64 * k as f64 / v.scale().max(1e-6)).ceil() as i32;
        Some((t.clone(), p, pad))
    }

    /// Vị trí chữ dưới con trỏ. Giữ ⌥ thì không tính là chữ (kéo để di chuyển).
    fn text_at(&self, x: i32, y: i32) -> Option<Pos> {
        if Mods::now().alt {
            return None;
        }
        let (t, p, pad) = self.to_image(x, y)?;
        t.hit(p, pad)
    }

    fn can_pan(&self) -> bool {
        let st = self.st.borrow();
        st.pan.is_some() || (st.view.is_some_and(|v| v.magnified()) && !Mods::now().alt)
    }

    fn drag_select(&self, x: i32, y: i32) {
        let changed = {
            let Some((t, p, _)) = self.to_image(x, y) else { return };
            let Some(focus) = t.nearest(p) else { return };
            let k = self.scale();
            let mut st = self.st.borrow_mut();
            let Some((anchor, start)) = st.text_press else { return };
            if !st.selecting {
                let d = (3.0 * k) as i32;
                if (x - start.0).abs() <= d && (y - start.1).abs() <= d {
                    return;
                }
                st.selecting = true;
            }
            st.sel.replace((anchor, focus)) != Some((anchor, focus))
        };
        if changed {
            self.paint();
        }
    }

    fn set_sel(&self, sel: Option<(Pos, Pos)>) {
        let changed = std::mem::replace(&mut self.st.borrow_mut().sel, sel) != sel;
        if changed {
            self.paint();
        }
    }

    fn select_all(&self) {
        let all = self.st.borrow().text.as_ref().and_then(|t| t.all());
        if all.is_some() {
            self.set_sel(all);
        }
    }

    fn selected_text(&self) -> Option<String> {
        let st = self.st.borrow();
        let (t, (a, b)) = (st.text.as_ref()?, st.sel?);
        Some(t.text(a, b)).filter(|s| !s.is_empty())
    }

    fn copy(&self) {
        match self.selected_text() {
            Some(text) => emit(Action::CopyText(text)),
            None => emit(Action::PreviewCopy(self.st.borrow().path.clone())),
        }
    }

    pub fn image_of(&self, path: &Path) -> Option<RgbaImage> {
        let st = self.st.borrow();
        st.img.as_ref().filter(|_| self.open.get() && st.path == path).cloned()
    }

    fn click(&self, b: Btn) {
        let path = self.st.borrow().path.clone();
        match b {
            Btn::Ocr => emit(Action::PreviewOcr(path)),
            Btn::Pin => self.toggle_pin(),
            Btn::Copy => emit(Action::PreviewCopy(path)),
            Btn::Folder => emit(Action::PreviewReveal(path)),
            Btn::Close => self.close(),
        }
    }

    fn on_menu(&self, tag: isize) {
        match tag {
            CMD_COPY_TEXT => self.copy(),
            CMD_SELECT_ALL => self.select_all(),
            CMD_COPY_IMAGE => emit(Action::PreviewCopy(self.st.borrow().path.clone())),
            _ => {}
        }
    }

    fn context_menu(&self, p: (i32, i32)) {
        let t = chup_core::i18n::t();
        let has_sel = self.selected_text().is_some();
        let has_text = self.st.borrow().text.as_ref().is_some_and(|t| !t.is_empty());
        let menu = NSMenu::new(self.mtm);
        menu.setAutoenablesItems(false);
        for (title, tag, enabled) in [
            (t.menu_copy_text, CMD_COPY_TEXT, has_sel),
            (t.menu_select_all_text, CMD_SELECT_ALL, has_text),
            ("", 0, true),
            (t.menu_copy_image, CMD_COPY_IMAGE, true),
        ] {
            let item = if tag == 0 {
                NSMenuItem::separatorItem(self.mtm)
            } else {
                let i = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(self.mtm),
                        &NSString::from_str(title),
                        Some(sel!(onMenu:)),
                        &NSString::from_str(""),
                    )
                };
                unsafe { i.setTarget(Some(self.menu_target.as_ref())) };
                i.setTag(tag);
                i.setEnabled(enabled);
                i
            };
            menu.addItem(&item);
        }
        let k = self.panel.window.backingScaleFactor();
        let at = NSPoint::new(p.0 as f64 / k, p.1 as f64 / k);
        // Không giữ borrow: menu chạy vòng lặp riêng.
        menu.popUpMenuPositioningItem_atLocation_inView(None, at, Some(&self.panel.view));
    }

    fn on_down(&self, wp: (i32, i32), clicks: isize, mods: Mods) {
        let (x, y) = self.local(wp);
        if let Some(b) = self.hit(x, y) {
            self.st.borrow_mut().press = Some(b);
            return;
        }
        if let Some(g) = self.edge(x, y) {
            self.sync_pos();
            let mut st = self.st.borrow_mut();
            st.resize = st.view.map(|v| (g, v, cursor_pos()));
            return;
        }
        if let Some(p) = self.text_at(x, y) {
            let text = self.st.borrow().text.clone();
            match (clicks, text) {
                (2, Some(t)) => self.set_sel(Some(t.word_range(p))),
                (3.., Some(t)) => self.set_sel(Some(t.row_range(p))),
                _ => {
                    let extend = {
                        let mut st = self.st.borrow_mut();
                        match st.sel {
                            Some((anchor, _)) if mods.shift => {
                                st.text_press = Some((anchor, (x, y)));
                                st.selecting = true;
                                st.sel = Some((anchor, p));
                                true
                            }
                            _ => {
                                st.text_press = Some((p, (x, y)));
                                st.selecting = false;
                                false
                            }
                        }
                    };
                    if extend {
                        self.paint();
                    }
                }
            }
            return;
        }
        self.set_sel(None);
        if self.can_pan() {
            let mut st = self.st.borrow_mut();
            st.pan = st.view.map(|v| (v, cursor_pos()));
            return;
        }
        // Kéo ở chỗ khác: di chuyển cửa sổ.
        self.st.borrow_mut().moving = Some((self.panel.rect(), cursor_pos()));
    }

    fn on_drag(&self, wp: (i32, i32)) {
        if self.st.borrow().resize.is_some() {
            return self.resize_move();
        }
        if self.st.borrow().pan.is_some() {
            return self.pan_move();
        }
        if self.st.borrow().text_press.is_some() {
            let (x, y) = self.local(wp);
            return self.drag_select(x, y);
        }
        let moving = self.st.borrow().moving;
        if let Some((r, from)) = moving {
            let (cx, cy) = cursor_pos();
            self.panel.set_rect(r.offset(cx - from.0, cy - from.1));
        }
    }

    fn on_up(&self, wp: (i32, i32)) {
        let text_click = {
            let mut st = self.st.borrow_mut();
            st.text_press.take().map(|_| !std::mem::take(&mut st.selecting))
        };
        if let Some(click) = text_click {
            if click {
                self.set_sel(None);
            }
            return;
        }
        let (resized, panned, moved) = {
            let mut st = self.st.borrow_mut();
            (st.resize.take().is_some(), st.pan.take().is_some(), st.moving.take().is_some())
        };
        if resized || moved {
            self.save_box();
            return;
        }
        if panned {
            return;
        }
        let (x, y) = self.local(wp);
        let up = self.hit(x, y);
        let press = self.st.borrow_mut().press.take();
        if let Some(b) = press.filter(|b| Some(*b) == up) {
            self.click(b);
        }
    }

    fn on_scroll(&self, wp: (i32, i32), dy: f64, lines: bool, mods: Mods) {
        let steps = {
            let mut st = self.st.borrow_mut();
            if lines {
                st.scroll = 0.0;
                dy.signum() as i32 * (dy.abs().round().max(1.0) as i32)
            } else {
                st.scroll += dy;
                let s = (st.scroll / SCROLL_STEP_PT).trunc();
                st.scroll -= s * SCROLL_STEP_PT;
                s as i32
            }
        };
        if steps == 0 {
            return;
        }
        if mods.ctrl {
            self.change_opacity(steps);
        } else if self.st.borrow().pan.is_none() {
            self.sync_pos();
            self.magnify(Some(steps), self.local(wp));
        }
    }

    fn on_key(&self, code: u16, mods: Mods) {
        match code {
            key::ESCAPE => {
                let pressing = {
                    let mut st = self.st.borrow_mut();
                    st.selecting = false;
                    st.text_press.take().is_some()
                };
                if pressing || self.selected_text().is_some() {
                    self.set_sel(None);
                } else {
                    self.close();
                }
            }
            key::C if mods.cmd => self.copy(),
            key::A if mods.cmd => self.select_all(),
            key::W if mods.cmd => self.close(),
            key::EQUAL | key::KEYPAD_PLUS => self.magnify(Some(1), self.local_center()),
            key::MINUS | key::KEYPAD_MINUS => self.magnify(Some(-1), self.local_center()),
            key::ZERO | key::KEYPAD_0 => self.magnify(None, self.local_center()),
            _ => {}
        }
    }
}

impl Handler for Preview {
    fn event(&self, ev: Ev) {
        if !self.open.get() {
            return;
        }
        match ev {
            Ev::Enter | Ev::Move { .. } => {
                let p = match ev {
                    Ev::Move { p } => Some(self.local(p)),
                    _ => None,
                };
                let h = p.and_then(|(x, y)| self.hit(x, y));
                let changed = {
                    let mut st = self.st.borrow_mut();
                    let entered = !std::mem::replace(&mut st.hovering, true);
                    let moved = std::mem::replace(&mut st.hover, h) != h;
                    entered || moved
                };
                if changed {
                    self.paint();
                }
            }
            Ev::Leave => {
                {
                    let mut st = self.st.borrow_mut();
                    st.hovering = false;
                    st.hover = None;
                }
                self.paint();
            }
            Ev::Down { p, clicks, mods } => self.on_down(p, clicks, mods),
            Ev::Drag { p } => self.on_drag(p),
            Ev::Up { p } => self.on_up(p),
            Ev::RightDown { p } => self.context_menu(p),
            Ev::Scroll { p, dy, lines, mods } => self.on_scroll(p, dy, lines, mods),
            Ev::Key { code, mods } => self.on_key(code, mods),
            _ => {}
        }
    }

    fn cursor(&self, p: (i32, i32)) -> Cursor {
        let (x, y) = self.local(p);
        let resizing = self.st.borrow().resize.map(|(g, _, _)| g);
        if let Some(g) = resizing.or_else(|| self.hit(x, y).is_none().then(|| self.edge(x, y)).flatten()) {
            return match g {
                Grip::N | Grip::S => Cursor::ResizeNS,
                Grip::E | Grip::W => Cursor::ResizeEW,
                Grip::NW | Grip::SE => Cursor::ResizeNWSE,
                _ => Cursor::ResizeNESW,
            };
        }
        if self.hit(x, y).is_some() {
            Cursor::Hand
        } else if self.st.borrow().text_press.is_some() || self.text_at(x, y).is_some() {
            Cursor::IBeam
        } else if self.can_pan() {
            Cursor::Move
        } else {
            Cursor::Arrow
        }
    }
}

impl Preview {
    /// Tự kiểm tra: bơm sự kiện, lấy ảnh đang hiện.
    pub fn inject(&self, ev: Ev) {
        Handler::event(self, ev);
    }

    pub fn snapshot(&self) -> Option<RgbaImage> {
        self.panel.snapshot()
    }
}
