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
    preview::{self as ui_preview, Btn, EDGE, PILL_MS, SPIN_MS, TEXT_PAD},
    Canvas as _,
};
use image::RgbaImage;
use objc2::{define_class, msg_send, rc::Retained, runtime::AnyObject, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSMenu, NSMenuItem};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSObject, NSPoint, NSString};
use objc2_quartz_core::{kCAFilterTrilinear, CALayer, CATransaction};

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
    /// File đang upload (SPEC 19): nút Upload của đúng file đó quay vòng bận.
    uploading: Option<PathBuf>,
    /// Vòng "bận" của nút [T]: bước quay, lúc bắt đầu OCR (nhãn "lần đầu có thể lâu"), timer quay.
    spin: u32,
    busy_since: Option<std::time::Instant>,
    spin_timer: Option<Timer>,
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

/// Cây layer của Preview, nằm trên layer gốc của view (layer gốc chứa bóng đổ và nền, xem `rescale`):
/// `clip` (khung ảnh, bo góc) chứa `img` (cả ảnh gốc, GPU co giãn), rồi `top` (viền, chữ chọn, nút, nhãn).
/// Phóng to và dời ảnh chỉ đổi khung của `img`: GPU vẽ lại, CPU không đụng tới pixel nào (mượt như app Preview).
struct Stage {
    /// Hệ toạ độ y hướng xuống, gốc ở góc trên-trái cửa sổ (đơn vị point).
    root: Retained<CALayer>,
    clip: Retained<CALayer>,
    img: Retained<CALayer>,
    top: Retained<CALayer>,
}

impl Stage {
    fn new(parent: &CALayer) -> Stage {
        let root = CALayer::new();
        // Lật sao cho layer con có y hướng xuống trên màn hình, bất kể layer của view đã lật hay chưa.
        root.setGeometryFlipped(!parent.contentsAreFlipped());
        let clip = CALayer::new();
        clip.setMasksToBounds(true);
        let img = CALayer::new();
        // Thu nhỏ nhiều (ảnh 4K vừa khung) dùng mipmap: không răng cưa.
        img.setMinificationFilter(unsafe { kCAFilterTrilinear });
        clip.addSublayer(&img);
        root.addSublayer(&clip);
        let top = CALayer::new();
        root.addSublayer(&top);
        parent.addSublayer(&root);
        Stage { root, clip, img, top }
    }

    fn clear(&self) {
        unsafe {
            self.img.setContents(None);
            self.top.setContents(None);
        }
    }
}

/// Đổi layer trong `f` không có hiệu ứng chuyển động, đưa lên màn hình cùng lúc.
fn no_anim(f: impl FnOnce()) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    f();
    CATransaction::commit();
}

fn cg_rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}

pub struct Preview {
    mtm: MainThreadMarker,
    panel: Panel,
    stage: RefCell<Option<Stage>>,
    open: Cell<bool>,
    st: RefCell<State>,
    menu_target: Retained<MenuTarget>,
    me: RefCell<Weak<Preview>>,
}

impl Preview {
    pub fn create(mtm: MainThreadMarker) -> Rc<Preview> {
        let panel = Panel::new(mtm, Opts { key: true, level: LEVEL_FLOATING, activating: true });
        let t = MenuTarget::alloc(mtm).set_ivars(MenuIvars { owner: RefCell::new(Weak::new()) });
        let menu_target: Retained<MenuTarget> = unsafe { msg_send![super(t), init] };
        let p = Rc::new(Preview {
            mtm,
            panel,
            stage: RefCell::new(None),
            open: Cell::new(false),
            st: RefCell::new(State::default()),
            menu_target,
            me: RefCell::new(Weak::new()),
        });
        *p.me.borrow_mut() = Rc::downgrade(&p);
        *p.menu_target.ivars().owner.borrow_mut() = Rc::downgrade(&p);
        let weak: Weak<dyn Handler> = Rc::downgrade(&p) as _;
        p.panel.set_handler(weak);
        p.panel.enable_pinch();
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
        if !self.rebuild() {
            self.close();
            emit(Action::Toast(chup_core::model::Toast::error(chup_core::i18n::t().err_preview_memory)));
            return;
        }
        self.apply_z(topmost);
        self.panel.show(true);
        self.panel.activate_app();
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
        if let Some(stage) = self.stage.borrow().as_ref() {
            no_anim(|| stage.clear());
        }
        {
            let mut st = self.st.borrow_mut();
            let pending = None;
            *st = State { pending, ..State::default() };
        }
        if was_open {
            self.panel.release_app();
            emit(Action::PreviewClosed);
        }
    }

    fn apply_z(&self, topmost: bool) {
        self.panel.set_level(if topmost { LEVEL_FLOATING } else { LEVEL_NORMAL });
    }

    /// Cỡ cửa sổ đổi (mở, kéo mép): vẽ lại nền, đặt lại ảnh và lớp trên. `false` = không cấp được bộ nhớ.
    fn rebuild(&self) -> bool {
        let mut ok = false;
        no_anim(|| {
            ok = self.rescale();
            if ok {
                self.place_image();
                self.paint();
            }
        });
        ok
    }

    /// Vẽ lớp nền (bóng đổ, nền tối) theo cỡ cửa sổ, đưa cửa sổ tới `view.rect`. Ảnh nạp lên GPU một lần.
    fn rescale(&self) -> bool {
        let k = self.scale();
        let theme = gfx::theme();
        let (base, pos, new_image) = {
            let mut st = self.st.borrow_mut();
            let st = &mut *st;
            let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return false };
            let m = ui_preview::margin(k);
            st.margin = m;
            let (w, h) = (v.rect.w as i32 + 2 * m, v.rect.h as i32 + 2 * m);
            let Some(base) = Surface::new(w, h) else { return false };
            let Some(c) = Canvas::new(&base) else { return false };
            let new_image = if st.image.is_none() {
                st.image = c.image(img);
                st.image.clone()
            } else {
                None
            };
            if st.image.is_none() {
                return false;
            }
            c.begin(Color::CLEAR);
            // grab: vùng ảnh ở lớp nền trong suốt (ảnh nằm ở layer riêng), phủ alpha 1/255 cho chắc nhận chuột.
            ui_preview::paint_back(&c, &theme, v, img.dimensions(), m, k, true);
            c.end();
            (base, v.rect.expand(m), new_image)
        };
        self.panel.present(&base, pos);
        let mut stage = self.stage.borrow_mut();
        if stage.is_none() {
            let Some(layer) = self.panel.view.layer() else { return false };
            *stage = Some(Stage::new(&layer));
        }
        let Some(stage) = stage.as_ref() else { return false };
        stage.root.setFrame(cg_rect(0.0, 0.0, pos.w as f64 / k as f64, pos.h as f64 / k as f64));
        if let Some(image) = new_image {
            let obj: &AnyObject = unsafe { &*(&*image as *const CGImage).cast::<AnyObject>() };
            unsafe { stage.img.setContents(Some(obj)) };
        }
        true
    }

    /// Đặt khung cắt và ảnh theo `view` (zoom, dời ảnh). Chỉ đổi khung layer: GPU co giãn ảnh.
    fn place_image(&self) {
        let k = self.scale() as f64;
        let st = self.st.borrow();
        let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return };
        let stage = self.stage.borrow();
        let Some(stage) = stage.as_ref() else { return };
        let (iw, ih) = img.dimensions();
        let p = ui_preview::image_place(v, (iw, ih), st.margin, k as f32);
        let (cx, cy) = (p.clip.left as f64, p.clip.top as f64);
        let (cw, ch) = ((p.clip.right - p.clip.left) as f64, (p.clip.bottom - p.clip.top) as f64);
        no_anim(|| {
            stage.clip.setFrame(cg_rect(cx / k, cy / k, cw / k, ch / k));
            stage.clip.setCornerRadius(p.radius as f64 / k);
            let s = p.scale as f64;
            stage.img.setFrame(cg_rect(
                (p.origin.0 as f64 - cx) / k,
                (p.origin.1 as f64 - cy) / k,
                iw as f64 * s / k,
                ih as f64 * s / k,
            ));
        });
    }

    /// Vẽ lớp trên: viền, vùng chọn chữ, toolbar, nhãn. Không đụng tới ảnh.
    fn paint(&self) {
        let k = self.scale();
        let top = {
            let mut st = self.st.borrow_mut();
            let st = &mut *st;
            let Some(v) = st.view else { return };
            let m = st.margin;
            let (w, h) = (v.rect.w as f32, v.rect.h as f32);
            st.hits = ui_preview::bar_hits(w, k);
            let sel: Vec<_> = match (st.text.as_ref(), st.sel, st.img.as_ref()) {
                (Some(t), Some(sel), Some(img)) => ui_preview::sel_rects(t, sel, v, img.dimensions(), m, k),
                _ => vec![],
            };
            let (sw, sh) = (v.rect.w as i32 + 2 * m, v.rect.h as i32 + 2 * m);
            let Some(out) = Surface::new(sw, sh) else { return };
            if let Some(c) = Canvas::new(&out) {
                let theme = gfx::theme();
                let busy = (st.ocr_busy.as_ref() == Some(&st.path)).then_some(st.spin);
                let busy_text = busy.and(st.busy_since).map(|t0| ui_preview::busy_label(t0.elapsed().as_millis()));
                let label = st.pill.as_deref().or(busy_text);
                let pill_font = label.and_then(|_| Font::from_spec(ui_preview::pill_font(k)));
                let top = ui_preview::Top {
                    k,
                    m,
                    size: (w, h),
                    sel: &sel,
                    bar: st.hovering,
                    hover: st.hover,
                    busy,
                    uploading: (st.uploading.as_ref() == Some(&st.path)).then_some(st.spin),
                    topmost: st.topmost,
                    pill: label.zip(pill_font.as_ref()),
                };
                c.begin(Color::CLEAR);
                ui_preview::paint_border(&c, &theme, v, m, k);
                ui_preview::paint_top(&c, &theme, &top);
                c.end();
            }
            self.panel.set_alpha(preview::alpha(st.opacity) as f64 / 255.0);
            out.image((sw, sh))
        };
        let stage = self.stage.borrow();
        let Some(stage) = stage.as_ref() else { return };
        no_anim(|| {
            stage.top.setFrame(stage.root.bounds());
            stage.top.setContentsScale(k as f64);
            let obj = top.as_ref().map(|i| unsafe { &*(&**i as *const CGImage).cast::<AnyObject>() });
            unsafe { stage.top.setContents(obj) };
        });
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

    /// Kéo mép xong: báo App để nhớ tỉ lệ so với cỡ tự tính.
    fn save_size(&self) {
        self.sync_pos();
        let st = self.st.borrow();
        if let (Some(v), Some(img)) = (st.view, st.img.as_ref()) {
            emit(Action::PreviewResized { rect: v.rect, img: img.dimensions(), zoom: v.zoom });
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
        self.set_zoom(|v, dims| match steps {
            Some(n) => preview::magnify_at(v, dims, n, anchor),
            None => preview::unmagnify(v),
        });
    }

    /// Đổi zoom, hiện nhãn % rồi vẽ lại. Cửa sổ giữ nguyên cỡ và chỗ.
    fn set_zoom(&self, f: impl FnOnce(View, (u32, u32)) -> View) {
        let scale = {
            let mut st = self.st.borrow_mut();
            let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return };
            let nv = f(v, img.dimensions());
            st.view = Some(nv);
            nv.scale()
        };
        self.show_pill(format!("{}%", (scale * 100.0).round()));
        self.redraw();
    }

    /// Zoom / dời ảnh đổi: đặt lại ảnh (GPU) và vẽ lớp trên. Cỡ cửa sổ không đổi.
    fn redraw(&self) {
        self.sync_pos();
        no_anim(|| {
            self.place_image();
            self.paint();
        });
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
        if !self.rebuild() {
            self.close();
        }
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

    /// File đang được OCR (`None` = không OCR gì): nút [T] của đúng file đó quay vòng "bận", kèm nhãn
    /// "Đang nhận diện chữ…" (chup-27e: lần đầu Vision chuẩn bị model gần 1 phút).
    pub fn set_ocr_busy(&self, busy: Option<PathBuf>) {
        let changed = {
            let mut st = self.st.borrow_mut();
            let changed = st.ocr_busy != busy;
            if changed {
                st.busy_since = busy.as_ref().map(|_| std::time::Instant::now());
                st.spin = 0;
                if busy.is_none() && st.uploading.is_none() {
                    st.spin_timer = None;
                }
            }
            st.ocr_busy = busy;
            changed
        };
        if !changed {
            return;
        }
        if self.st.borrow().ocr_busy.is_some() {
            self.spin_later();
        }
        if self.open.get() {
            self.paint();
        }
    }

    /// File đang upload (`None` = không upload gì): nút Upload của đúng file đó quay vòng "bận".
    pub fn set_uploading(&self, path: Option<PathBuf>) {
        let (changed, start) = {
            let mut st = self.st.borrow_mut();
            let changed = st.uploading != path;
            // Vòng quay đang chạy cho OCR thì dùng chung, không hẹn thêm.
            let start = changed && path.is_some() && st.spin_timer.is_none();
            st.uploading = path;
            (changed, start)
        };
        if start {
            self.spin_later();
        }
        if changed && self.open.get() {
            self.paint();
        }
    }

    /// Hẹn bước quay tiếp theo của vòng "bận". Dừng khi hết bận.
    fn spin_later(&self) {
        let me = self.me.borrow().clone();
        let t = win::after(SPIN_MS, move || {
            let Some(p) = me.upgrade() else { return };
            let busy = {
                let mut st = p.st.borrow_mut();
                st.spin = st.spin.wrapping_add(1);
                st.ocr_busy.is_some() || st.uploading.is_some()
            };
            if busy {
                if p.open.get() {
                    p.paint();
                }
                p.spin_later();
            }
        });
        self.st.borrow_mut().spin_timer = Some(t);
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

    /// Bấm nút toolbar; `wp`: điểm bấm (toạ độ cửa sổ, px) để mở menu Upload ngay đó.
    fn click(&self, b: Btn, wp: (i32, i32)) {
        let path = self.st.borrow().path.clone();
        match b {
            Btn::Ocr => emit(Action::PreviewOcr(path)),
            Btn::Pin => self.toggle_pin(),
            Btn::Upload => {
                let k = self.panel.window.backingScaleFactor();
                let at = NSPoint::new(wp.0 as f64 / k, wp.1 as f64 / k);
                super::share::popup(&self.panel.view, at, self.mtm, move |kind| {
                    emit(Action::PreviewUpload(path.clone(), kind))
                });
            }
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
            ("", 0, true),
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
        let path = self.st.borrow().path.clone();
        super::share::add_items(&menu, self.mtm, move |kind| emit(Action::PreviewUpload(path.clone(), kind)));
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
            // Chỉ nhớ khung khi cửa sổ thật sự đổi chỗ: bấm suông không được ghi đè khung người dùng đã chọn.
            let moved = st.moving.take().is_some_and(|(r, _)| r != self.panel.rect());
            // Bấm vào mép mà không kéo: không phải chọn cỡ.
            let view = st.view;
            let resized = st.resize.take().is_some_and(|(_, s, _)| view.is_some_and(|v| (v.rect.w, v.rect.h) != (s.rect.w, s.rect.h)));
            (resized, st.pan.take().is_some(), moved)
        };
        if resized {
            self.save_size();
            return;
        }
        if moved {
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
            self.click(b, wp);
        }
    }

    /// Chuột có nấc: phóng to/thu nhỏ như Windows. Trackpad (cuộn mịn): dời ảnh đang phóng to theo 2 ngón, như
    /// app Preview của macOS (SPEC 17). Ctrl + cuộn: đổi opacity ở cả hai.
    fn on_scroll(&self, wp: (i32, i32), dy: f64, lines: bool, content: (f64, f64), mods: Mods) {
        if !lines && !mods.ctrl {
            self.scroll_pan(content);
            return;
        }
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

    fn scroll_pan(&self, (dx, dy): (f64, f64)) {
        if (dx == 0.0 && dy == 0.0) || self.st.borrow().pan.is_some() {
            return;
        }
        let changed = {
            let mut st = self.st.borrow_mut();
            let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return };
            let nv = preview::pan_by(v, img.dimensions(), dx, dy);
            st.view = Some(nv);
            nv.off != v.off
        };
        if changed {
            self.redraw();
        }
    }

    /// Chụm/xoè 2 ngón: phóng to quanh con trỏ như lăn chuột (SPEC 17). `delta` cộng dồn được nên dùng `exp`.
    fn on_pinch(&self, wp: (i32, i32), delta: f64) {
        if self.st.borrow().pan.is_some() {
            return;
        }
        self.sync_pos();
        let anchor = self.local(wp);
        self.set_zoom(|v, dims| preview::zoom_at(v, dims, delta.exp(), anchor));
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
            Ev::Scroll { p, dy, lines, content, mods } => self.on_scroll(p, dy, lines, content, mods),
            Ev::Magnify { p, delta } => self.on_pinch(p, delta),
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

    /// Ghép cả cây layer (nền, ảnh, lớp trên) như trên màn hình.
    pub fn snapshot(&self) -> Option<RgbaImage> {
        self.panel.snapshot_layers()
    }
}
