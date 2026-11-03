//! Chế độ Freeze của overlay (SPEC 14.3): hiện ảnh đóng băng 1:1, chọn vùng, chỉnh bằng 8 handle,
//! toolbar [✓ Lưu][Copy][T Chữ][✕] vẽ ngay trong overlay (không phải cửa sổ riêng).
//! Chế độ "Chụp text" (SPEC 15.2): không có toolbar, thả chuột là OCR luôn.
//! Hình học nằm ở `chup_core::selection`, pixel ở `chup_core::freeze`.

use std::sync::Arc;

use chup_core::{
    autoselect::is_click,
    coords::{place_toolbar, Rect},
    freeze,
    icons::Icon,
    model::FreezeAction,
    selection::{self, Grip},
};
use image::RgbaImage;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
            WindowsAndMessaging::{
                LoadCursorW, SetCursor, WM_CAPTURECHANGED, IDC_CROSS, IDC_HAND, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE,
                IDC_SIZEWE,
            },
        },
    },
};

use super::{
    super::{
        app::Action,
        gfx::{pill, Align, Canvas, Color, Font, Surface, Theme},
        toolbar::{bar_width, paint_bar, BarButton},
        win::emit,
    },
    auto, cursor_pos, hiword, loword, Overlay, Sprite, State, VK_ESCAPE, VK_RETURN, WM_KEYDOWN, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP, WM_SETCURSOR,
};

const HINT_FROZEN: &str = "Ảnh đã đóng băng · Kéo để chọn vùng · Enter để lưu · Esc để huỷ";
const HINT_FROZEN_AUTO: &str = "Ảnh đã đóng băng · Click chọn khung hoặc kéo chọn vùng · Enter để lưu · Esc để huỷ";
const HINT_FAILED: &str = "Không chụp được màn hình này · Esc để huỷ";
const HINT_OCR: &str = "Chụp text · Kéo để chọn vùng có chữ · Esc để huỷ";
/// Handle vẽ 8 DIP, vùng bắt chuột 14 DIP cho dễ trúng.
const HANDLE_DIP: f64 = 8.0;
const HANDLE_HIT_DIP: f64 = 14.0;
const BAR_H_DIP: f64 = 44.0;
const BAR_GAP_DIP: f64 = 6.0;
const WHITE: u32 = 0xFFFF_FFFF;
const DARK: u32 = 0xFF20_2020;

const BUTTONS: [BarButton; 4] = [
    BarButton { icon: Icon::Check, label: "Lưu", primary: true, active: false },
    BarButton { icon: Icon::Copy, label: "Copy", primary: false, active: false },
    BarButton { icon: Icon::Text, label: "Chữ", primary: false, active: false },
    BarButton { icon: Icon::Close, label: "", primary: false, active: false },
];
/// Hành động của từng nút; `None` = huỷ.
const ACTIONS: [Option<FreezeAction>; 4] =
    [Some(FreezeAction::Save), Some(FreezeAction::Copy), Some(FreezeAction::Ocr), None];

#[derive(Clone, Copy)]
enum Drag {
    Idle,
    /// Đang kéo chọn vùng mới từ điểm này.
    New((i32, i32)),
    /// Đang di chuyển hoặc resize: luôn tính từ vùng lúc nhấn chuột.
    Adjust { grip: Grip, from: (i32, i32), rect0: Rect },
    /// Đang nhấn nút toolbar thứ i.
    Button(usize),
}

pub(super) struct Frozen {
    /// `None`: màn hình này chụp hỏng, chỉ phủ tối kèm thông báo, không cho chọn.
    /// `Arc`: worker tự chọn vùng đọc chung để nhận khung từ ảnh.
    img: Option<Arc<RgbaImage>>,
    sel: Option<Rect>,
    drag: Drag,
    hint: Option<Sprite>,
    bar: Option<Bar>,
    /// Cạnh handle (physical px).
    handle: i32,
    /// "Chụp text": không toolbar, thả chuột / Enter / double-click là OCR.
    ocr_only: bool,
}

impl Frozen {
    /// Hành động mặc định (Enter, double-click).
    fn default_action(&self) -> FreezeAction {
        if self.ocr_only {
            FreezeAction::Ocr
        } else {
            FreezeAction::Save
        }
    }
}

/// Toolbar vẽ sẵn vào sprite riêng, khi cần mới blend vào DIB của overlay.
struct Bar {
    // Canvas giữ render target trên DC của surface: khai báo trước để được thả trước.
    canvas: Canvas,
    surface: Surface,
    font: Option<Font>,
    k: f32,
    /// Chỗ đặt trên overlay, tính lại mỗi khi vùng chọn đổi.
    at: Rect,
    /// Rect từng nút, toạ độ trong sprite.
    buttons: Vec<Rect>,
    hover: Option<usize>,
    pressed: Option<usize>,
}

impl Bar {
    fn new(k: f32) -> Option<Bar> {
        let font = Font::new(13.0 * k, false, Align::Leading, false);
        let w = (bar_width(font.as_ref(), k, &BUTTONS, false) + 4.0 * k).ceil() as i32;
        let h = (BAR_H_DIP as f32 * k).ceil() as i32;
        let surface = Surface::new(w, h)?;
        let canvas = Canvas::new(&surface)?;
        let mut b = Bar { canvas, surface, font, k, at: Rect::default(), buttons: vec![], hover: None, pressed: None };
        b.render();
        Some(b)
    }

    fn render(&mut self) {
        let theme = Theme::current();
        let size = (self.surface.w, self.surface.h);
        self.canvas.begin(Color::rgba(0, 0, 0, 0.0));
        self.buttons =
            paint_bar(&self.canvas, self.font.as_ref(), &theme, self.k, size, &BUTTONS, false, self.hover, self.pressed);
        self.canvas.end();
    }

    fn hit(&self, p: (i32, i32)) -> Option<usize> {
        let (x, y) = (p.0 - self.at.x, p.1 - self.at.y);
        self.buttons.iter().position(|r| r.contains_point(x, y))
    }
}

impl Frozen {
    /// Toolbar chỉ hiện khi đã có vùng và không kéo vùng.
    fn bar_visible(&self) -> bool {
        self.sel.is_some() && matches!(self.drag, Drag::Idle | Drag::Button(_))
    }

    fn bar_hit(&self, p: (i32, i32)) -> Option<usize> {
        self.bar.as_ref().filter(|_| self.bar_visible()).and_then(|b| b.hit(p))
    }

    /// Gợi ý chỉ hiện khi chưa có vùng và không kéo gì.
    fn hint_visible(&self) -> bool {
        self.sel.is_none() && matches!(self.drag, Drag::Idle)
    }

    /// Toolbar sắp ẩn (bắt đầu kéo, mất vùng chọn): bỏ trạng thái hover/nhấn cũ,
    /// để lần hiện sau không sáng nhầm nút.
    fn reset_bar_state(&mut self) {
        if let Some(b) = self.bar.as_mut().filter(|b| b.hover.is_some() || b.pressed.is_some()) {
            b.hover = None;
            b.pressed = None;
            b.render();
        }
    }
}

/// Phần cần vẽ lại sau một sự kiện.
enum Change {
    None,
    /// Chỉ toolbar đổi (hover, nhấn): vẽ lại đúng chỗ toolbar.
    Bar,
    /// Vùng chọn hoặc trạng thái kéo đổi: vẽ lại phần cũ ∪ phần mới.
    Sel,
}

/// Phần overlay có thể đổi theo vùng chọn: vùng kèm handle, nhãn, toolbar, gợi ý.
fn extent(st: &State) -> Rect {
    let Some(f) = st.frozen.as_ref() else { return Rect::default() };
    let mut r = f.sel.map(|s| s.expand(f.handle + 2)).unwrap_or_default();
    let bar = f.bar.as_ref().filter(|_| f.bar_visible()).map(|b| b.at);
    // Chỉ tính gợi ý khi nó đang hiện: nó nằm ở giữa mép trên, gộp vào lúc kéo vùng ở góc dưới
    // sẽ làm dirty phình ra gần nửa màn hình mỗi lần chuột di.
    let hint = f.hint.as_ref().filter(|_| f.hint_visible()).map(|h| h.at);
    for o in [st.label_at, bar, hint].into_iter().flatten() {
        r = r.union(&o);
    }
    r
}

impl Overlay {
    /// Hiện overlay ở chế độ Freeze. `img = None`: màn hình này chụp hỏng. `ocr_only`: "Chụp text".
    /// Trả về `false` nếu không hiện được (không cấp được DIB).
    pub fn show_frozen(&self, focus: bool, img: Option<RgbaImage>, ocr_only: bool, auto: bool) -> bool {
        let (w, h) = (self.mon.rect.w as i32, self.mon.rect.h as i32);
        let t0 = std::time::Instant::now();
        let k = self.mon.scale as f32;
        let text = match (img.is_some(), ocr_only) {
            (false, _) => HINT_FAILED,
            (true, true) => HINT_OCR,
            (true, false) if auto => HINT_FROZEN_AUTO,
            (true, false) => HINT_FROZEN,
        };
        let white = Color::rgba(255, 255, 255, 1.0);
        let hint = pill(text, 13.0 * k, 12.0 * k, 6.0 * k, 999.0, Color::rgba(0, 0, 0, 0.7), white).map(|s| {
            let at = Rect::new((w - s.w) / 2, self.s(24.0), s.w as u32, s.h as u32);
            Sprite { s, at }
        });
        let bar = if ocr_only { None } else { Bar::new(k) };
        let t_prep = t0.elapsed();
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
            st.auto = None;
            st.sel = None;
            st.label_at = None;
            st.label_text.clear();
            st.frozen =
                Some(Frozen {
                    img: img.map(Arc::new),
                    sel: None,
                    drag: Drag::Idle,
                    hint,
                    bar,
                    handle: self.s(HANDLE_DIP),
                    ocr_only,
                });
            self.frozen_mode.set(true);
            super::paint(&mut st, Rect::new(0, 0, w as u32, h as u32));
        }
        let t_paint = t0.elapsed();
        if let Some(s) = self.st.borrow().surface.as_ref() {
            s.present(self.hwnd, (self.mon.rect.x, self.mon.rect.y), 255, None);
        }
        self.raise(focus);
        log::debug!(
            "overlay freeze {} {w}×{h}: gợi ý + toolbar {t_prep:?}, tô {:?}, tổng {:?}",
            self.mon.key,
            t_paint - t_prep,
            t0.elapsed()
        );
        true
    }

    /// Lấy ảnh đóng băng ra để crop (Lưu/Copy). Overlay vẫn hiện tới khi `hide`.
    pub fn take_frozen(&self) -> Option<Arc<RgbaImage>> {
        self.st.borrow_mut().frozen.as_mut()?.img.take()
    }

    /// Ảnh đóng băng của màn hình này (dùng chung), cho worker tự chọn vùng.
    pub fn frozen_image(&self) -> Option<Arc<RgbaImage>> {
        self.st.borrow().frozen.as_ref()?.img.clone()
    }

    /// Có tô khung tự động ở `p` không: có ảnh, không đang kéo, không trong vùng chọn / trên handle / toolbar.
    pub(super) fn frozen_auto_allowed(&self, p: (i32, i32)) -> bool {
        let st = self.st.borrow();
        let Some(f) = st.frozen.as_ref() else { return false };
        f.img.is_some()
            && matches!(f.drag, Drag::Idle)
            && f.bar_hit(p).is_none()
            && f.sel.and_then(|s| selection::hit(s, p, self.s(HANDLE_HIT_DIP))).is_none()
    }

    /// Người dùng bắt đầu chọn ở màn hình khác: bỏ vùng chọn ở màn hình này.
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

    /// Chạy `f` trên state Freeze, đặt lại nhãn và toolbar, rồi vẽ và đẩy phần đã đổi.
    /// Di chuột chỉ để hover thì chỉ vẽ lại toolbar: compose cả vùng lớn mỗi lần chuột di sẽ giật ở 4K.
    fn frozen_update(&self, f: impl FnOnce(&mut Frozen) -> Change) {
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
                // Toolbar đặt trên toàn màn hình (được đè lên taskbar trong ảnh đóng băng).
                let bounds = self.bounds();
                if let (Some(s), Some(b)) = (sel, st.frozen.as_mut().and_then(|f| f.bar.as_mut())) {
                    let size = (b.surface.w as u32, b.surface.h as u32);
                    let (x, y) = place_toolbar(&s, size, &bounds, gap);
                    b.at = Rect::new(x, y, size.0, size.1);
                }
                before.union(&extent(&st))
            }
        };
        super::paint(&mut st, dirty);
        if let Some(s) = st.surface.as_ref() {
            s.present(self.hwnd, (self.mon.rect.x, self.mon.rect.y), 255, Some(dirty));
        }
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
                // Double-click trong vùng = Lưu (SPEC 14.3).
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
            emit(Action::FreezeSelecting(self.hwnd.0 as isize));
        }
        if save {
            let action = self.st.borrow().frozen.as_ref().map(Frozen::default_action);
            self.frozen_confirm(action);
        }
    }

    fn frozen_move(&self, p: (i32, i32)) {
        let b = self.bounds();
        let p = (p.0.clamp(0, b.right()), p.1.clamp(0, b.bottom()));
        // Sau khi xử lý: Some(true) = rê tự do, xét tô khung; Some(false) = bắt đầu kéo, bỏ khung tô.
        let mut hover = None;
        self.frozen_update(|f| match f.drag {
            // Nhấn rồi di chưa quá 4 px: vẫn là click (chọn khung đang tô).
            Drag::New(start) if f.sel.is_none() && is_click(start, p) => Change::None,
            Drag::New(start) => {
                hover = Some(false);
                let r = Rect::from_points(start, p);
                f.sel = (!r.is_empty()).then_some(r);
                Change::Sel
            }
            Drag::Adjust { grip, from, rect0 } => {
                f.sel = Some(selection::apply(rect0, grip, from, p, b));
                Change::Sel
            }
            Drag::Idle => {
                hover = Some(true);
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
        match hover {
            Some(true) => self.auto_hover(p, self.frozen_auto_allowed(p)),
            Some(false) => self.auto_hover(p, false),
            None => {}
        }
    }

    /// Kết thúc lần kéo. `click = false` khi mất capture (Alt+Tab, Win, UAC…): không bấm nút nào.
    fn frozen_end(&self, p: (i32, i32), click: bool) {
        let mut clicked = None;
        let mut auto_ocr = false;
        let picked = self.auto_frame();
        self.frozen_update(|f| match std::mem::replace(&mut f.drag, Drag::Idle) {
            Drag::Button(i) => {
                if click && f.bar_hit(p) == Some(i) {
                    clicked = Some(i);
                }
                if let Some(b) = f.bar.as_mut() {
                    b.pressed = None;
                    b.render();
                }
                Change::Bar
            }
            Drag::New(start) => {
                // Click (chưa di quá 4 px): lấy khung đang tô (SPEC 16.2 bước 4).
                if f.sel.is_none() && click && is_click(start, p) {
                    f.sel = picked;
                }
                // Giống Live: vùng nhỏ hơn 4×4 coi như chưa chọn.
                if f.sel.is_some_and(|s| s.w < 4 || s.h < 4) {
                    f.sel = None;
                }
                auto_ocr = click && f.ocr_only && f.sel.is_some();
                Change::Sel
            }
            Drag::Adjust { .. } => Change::Sel,
            Drag::Idle => Change::None,
        });
        if let Some(i) = clicked {
            self.frozen_confirm(ACTIONS[i]);
        } else if auto_ocr {
            self.frozen_confirm(Some(FreezeAction::Ocr));
        } else {
            // Vừa có vùng chọn mới: chuột giờ nằm trong vùng, thôi tô khung.
            self.auto_hover(p, self.frozen_auto_allowed(p));
        }
    }

    /// `None` = huỷ. Lưu/Copy chỉ chạy khi đã có vùng.
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

    fn frozen_cursor(&self, p: (i32, i32)) -> PCWSTR {
        let st = self.st.borrow();
        let Some(f) = st.frozen.as_ref() else { return IDC_CROSS };
        if f.bar_hit(p).is_some() {
            return IDC_HAND;
        }
        match f.sel.and_then(|s| selection::hit(s, p, self.s(HANDLE_HIT_DIP))) {
            Some(Grip::Move) => IDC_SIZEALL,
            Some(Grip::N | Grip::S) => IDC_SIZENS,
            Some(Grip::E | Grip::W) => IDC_SIZEWE,
            Some(Grip::NW | Grip::SE) => IDC_SIZENWSE,
            Some(Grip::NE | Grip::SW) => IDC_SIZENESW,
            None => IDC_CROSS,
        }
    }

    /// Xử lý message khi overlay đang ở chế độ Freeze.
    pub(super) fn handle_frozen(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let p = (loword(lp.0), hiword(lp.0));
        match msg {
            WM_SETCURSOR => {
                let (cx, cy) = cursor_pos();
                let id = self.frozen_cursor((cx - self.mon.rect.x, cy - self.mon.rect.y));
                unsafe { SetCursor(LoadCursorW(None, id).ok()) };
                Some(LRESULT(1))
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                self.frozen_down(p, msg == WM_LBUTTONDBLCLK);
                unsafe { SetCapture(hwnd) };
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                self.frozen_move(p);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                // Kết thúc kéo trước khi nhả capture: ReleaseCapture gửi WM_CAPTURECHANGED ngay.
                self.frozen_end(p, true);
                unsafe {
                    let _ = ReleaseCapture();
                }
                Some(LRESULT(0))
            }
            // Mất capture giữa lúc kéo (không có WM_LBUTTONUP): chốt vùng ở vị trí hiện tại,
            // không để vùng chọn tiếp tục chạy theo chuột khi đã nhả nút.
            WM_CAPTURECHANGED if lp.0 != hwnd.0 as isize => {
                let (cx, cy) = cursor_pos();
                self.frozen_end((cx - self.mon.rect.x, cy - self.mon.rect.y), false);
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                emit(Action::FreezeCancel);
                Some(LRESULT(0))
            }
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
                emit(Action::FreezeCancel);
                Some(LRESULT(0))
            }
            WM_KEYDOWN if wp.0 as u16 == VK_RETURN.0 => {
                let action = self.st.borrow().frozen.as_ref().map(Frozen::default_action);
                self.frozen_confirm(action);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

/// Vẽ phần `dirty` ở chế độ Freeze: ảnh (tối ngoài vùng), viền, handle, gợi ý, nhãn, toolbar.
pub(super) fn paint_frozen(st: &mut State, dirty: Rect) {
    let hl = auto::highlight(st);
    let State { surface, frozen, label, label_at, .. } = st;
    let (Some(s), Some(f)) = (surface.as_mut(), frozen.as_mut()) else { return };
    let sw = s.w as usize;
    match &f.img {
        Some(img) => {
            freeze::compose(img, s.pixels(), sw, dirty, f.sel);
            // Khung tự động: bên trong là ảnh gốc, không làm tối (vẽ trước viền, handle của vùng chọn).
            if let Some(h) = hl {
                if let Some(inside) = h.intersect(&dirty) {
                    freeze::compose(img, s.pixels(), sw, inside, Some(h));
                }
            }
        }
        None => s.fill(dirty, Color::rgba(0, 0, 0, 0.6).px()),
    }
    if let Some(sel) = f.sel {
        s.outline_outside(sel, 1, WHITE);
        if !matches!(f.drag, Drag::New(_)) {
            for (_, hr) in selection::handles(sel, f.handle) {
                if hr.intersect(&dirty).is_some() {
                    s.fill(hr.expand(1), DARK);
                    s.fill(hr, WHITE);
                }
            }
        }
    }
    if f.hint_visible() {
        if let Some(h) = f.hint.as_mut() {
            if h.at.intersect(&dirty).is_some() {
                s.blend_from(&mut h.s, h.at.x, h.at.y);
            }
        }
    }
    if let (Some(p), Some(at)) = (label.as_mut(), label_at.as_ref()) {
        s.blend_part(&mut p.surface, (at.w as i32, at.h as i32), at.x, at.y);
    }
    let show_bar = f.bar_visible();
    if let Some(b) = f.bar.as_mut().filter(|_| show_bar) {
        if b.at.intersect(&dirty).is_some() {
            s.blend_from(&mut b.surface, b.at.x, b.at.y);
        }
    }
    if hl.is_some() {
        auto::paint_auto_edges(st);
    }
}
