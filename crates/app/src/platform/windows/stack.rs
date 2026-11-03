//! Stack thumbnail (SPEC 5.4). Không bao giờ lấy focus. Cửa sổ co giãn đúng theo nội dung,
//! neo ở góc dưới của work area. Kéo thẻ ≥ 4px = kéo file PNG ra app khác; click = mở ảnh.

use std::{cell::RefCell, collections::HashMap};

use chup_core::{
    capture::Shot,
    coords::Rect,
    icons::Icon,
    model::{Toast, ToastKind},
};
use windows::Win32::{
    UI::Controls::WM_MOUSELEAVE,
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Direct2D::ID2D1Bitmap,
    UI::{
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT},
        WindowsAndMessaging::{
            KillTimer, SetTimer, SetWindowPos, ShowWindow, HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE,
            SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WM_CAPTURECHANGED, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_TIMER, WS_EX_LAYERED, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

use super::{
    app::Action,
    gfx::{rectf, Align, Canvas, Color, Font, Surface, Theme},
    util::{cursor_pos, hiword, loword, HwndHandle},
    win::{self, emit, Handler},
};

// Bố cục (DIP), giống bản web cũ.
const WIDTH: f32 = 184.0;
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 6.0;
const GAP: f32 = 8.0;
const CARD_W: f32 = 160.0;
const TAB_W: f32 = 28.0;
const TAB_H: f32 = 56.0;
const TAB_BOTTOM: f32 = 6.0;
const HEADER_H: f32 = 24.0;

const TIMER_LEAVE: usize = 1;
/// wParam của WM_MOUSEMOVE: phím trái đang giữ.
const MK_LBUTTON: usize = 0x0001;
const TIMER_TOAST_BASE: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Card(usize),
    Copy(usize),
    Folder(usize),
    Remove(usize),
    Collapse,
    Clear,
    Tab,
    Toast(u32),
}

struct Press {
    hit: Hit,
    at: (i32, i32),
}

struct ToastItem {
    id: u32,
    toast: Toast,
}

#[derive(Default)]
struct State {
    shots: Vec<Shot>,
    edge_right: bool,
    scale: f64,
    work: Rect,
    toasts: Vec<ToastItem>,
    next_toast: u32,
    collapsed: bool,
    hovering: bool,
    hover: Option<Hit>,
    press: Option<Press>,
    tracking: bool,
    visible: bool,
    hidden_for_capture: bool,
    surface: Option<Surface>,
    canvas: Option<Canvas>,
    bitmaps: HashMap<String, ID2D1Bitmap>,
    hits: Vec<(Hit, Rect)>,
    size: (i32, i32),
}

pub struct StackWin {
    pub hwnd: HWND,
    st: RefCell<State>,
}

thread_local! {
    static CLASS: Vec<u16> = win::register_class::<StackWin>("ChupStack");
}

fn thumb_h(s: &Shot) -> f32 {
    (CARD_W * s.height as f32 / s.width.max(1) as f32).round().clamp(40.0, 120.0)
}

impl StackWin {
    pub fn create() -> windows::core::Result<std::rc::Rc<StackWin>> {
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
                |hwnd| StackWin { hwnd, st: RefCell::new(State { edge_right: true, scale: 1.0, ..Default::default() }) },
            )
        })
    }

    /// Danh sách shot hoặc vị trí đổi. `work` = work area của màn hình đặt stack.
    pub fn update(&self, shots: Vec<Shot>, edge_right: bool, work: Rect, scale: f64) {
        {
            let mut st = self.st.borrow_mut();
            // Có ảnh mới thì tự mở rộng nếu đang thu gọn.
            if shots.iter().any(|s| !st.shots.iter().any(|o| o.id == s.id)) {
                st.collapsed = false;
            }
            if st.scale != scale {
                st.surface = None;
                st.canvas = None;
                st.bitmaps.clear();
            }
            st.bitmaps.retain(|id, _| shots.iter().any(|s| &s.id == id));
            st.shots = shots;
            st.edge_right = edge_right;
            st.work = work;
            st.scale = scale;
        }
        self.relayout();
    }

    /// Rect cửa sổ stack nếu đang hiện (Preview mở sát cạnh nó).
    pub fn window_rect(&self) -> Option<Rect> {
        if !self.st.borrow().visible || self.st.borrow().hidden_for_capture {
            return None;
        }
        let mut r = windows::Win32::Foundation::RECT::default();
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(self.hwnd, &mut r) }.ok()?;
        Some(super::util::rect_of(&r))
    }

    pub fn toast(&self, t: Toast) {
        let id = {
            let mut st = self.st.borrow_mut();
            st.next_toast += 1;
            let id = st.next_toast;
            st.toasts.push(ToastItem { id, toast: t.clone() });
            if st.toasts.len() > 3 {
                st.toasts.remove(0);
            }
            id
        };
        let ms = if t.kind == ToastKind::Error { 7000 } else { 2500 };
        unsafe { SetTimer(Some(self.hwnd), TIMER_TOAST_BASE + id as usize, ms, None) };
        self.relayout();
    }

    pub fn set_hidden_for_capture(&self, hidden: bool) {
        let visible = {
            let mut st = self.st.borrow_mut();
            st.hidden_for_capture = hidden;
            st.visible
        };
        if visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, if hidden { SW_HIDE } else { SW_SHOWNOACTIVATE });
            }
        }
    }

    pub fn is_visible(&self) -> bool {
        self.st.borrow().visible
    }

    fn remove_toast(&self, id: u32) {
        let removed = {
            let mut st = self.st.borrow_mut();
            let before = st.toasts.len();
            st.toasts.retain(|t| t.id != id);
            before != st.toasts.len()
        };
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_TOAST_BASE + id as usize);
        }
        if removed {
            self.relayout();
        }
    }

    /// Tính bố cục, vẽ, đặt kích thước và vị trí cửa sổ.
    fn relayout(&self) {
        let theme = Theme::current();
        let mut st = self.st.borrow_mut();
        let k = st.scale as f32;
        let has = !st.shots.is_empty();
        let show_list = has && !st.collapsed;
        let show_tab = has && st.collapsed;
        let show_header = st.hovering && show_list;
        let wrap_visible = show_list || !st.toasts.is_empty();

        if !wrap_visible && !show_tab {
            if st.visible {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
            }
            st.visible = false;
            st.canvas = None;
            st.surface = None;
            st.bitmaps.clear();
            st.hits.clear();
            return;
        }

        let font = Font::new(13.0 * k, false, Align::Leading, true);
        let small = Font::new(11.0 * k, false, Align::Center, false);
        let btn_font = Font::new(12.0 * k, false, Align::Center, false);

        // Chiều cao từng phần tử trong "wrap" (trên xuống).
        enum Item {
            Header,
            Toast(usize, f32),
            Card(usize, f32),
        }
        let mut items = vec![];
        if show_header {
            items.push(Item::Header);
        }
        for (i, t) in st.toasts.iter().enumerate() {
            let th = font.as_ref().map(|f| f.measure(&t.toast.message, (CARD_W - 23.0) * k).1).unwrap_or(16.0 * k);
            items.push(Item::Toast(i, th + 16.0 * k));
        }
        if show_list {
            for (i, s) in st.shots.iter().enumerate() {
                items.push(Item::Card(i, thumb_h(s) * k));
            }
        }
        let item_h = |it: &Item| match it {
            Item::Header => HEADER_H * k,
            Item::Toast(_, h) | Item::Card(_, h) => *h,
        };
        let wrap_h = if wrap_visible {
            PAD_Y * 2.0 * k + items.iter().map(item_h).sum::<f32>() + GAP * k * (items.len().saturating_sub(1)) as f32
        } else {
            0.0
        };
        let tab_h = if show_tab { (TAB_H + TAB_BOTTOM) * k } else { 0.0 };
        let total_w = if wrap_visible { WIDTH * k } else { TAB_W * k }.ceil() as i32;
        let total_h = (wrap_h + tab_h).ceil() as i32;

        // Surface cấp dư theo chiều cao để hover header/toast không phải cấp lại.
        let need_new = st.surface.as_ref().map(|s| s.w < total_w || s.h < total_h).unwrap_or(true);
        if need_new {
            st.canvas = None;
            st.bitmaps.clear();
            st.surface = Surface::new(total_w, total_h + (160.0 * k) as i32);
            st.canvas = st.surface.as_ref().and_then(Canvas::new);
        }

        // Bitmap thumbnail (gắn với render target).
        let missing: Vec<usize> =
            (0..st.shots.len()).filter(|&i| !st.bitmaps.contains_key(&st.shots[i].id)).collect();
        for i in missing {
            let bmp = st.canvas.as_ref().and_then(|c| c.bitmap(&st.shots[i].thumb));
            if let Some(b) = bmp {
                let id = st.shots[i].id.clone();
                st.bitmaps.insert(id, b);
            }
        }

        let (hover, edge_right) = (st.hover, st.edge_right);
        let mut hits: Vec<(Hit, Rect)> = vec![];
        let to_rect = |r: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F| {
            Rect::new(r.left as i32, r.top as i32, (r.right - r.left) as u32, (r.bottom - r.top) as u32)
        };
        let Some(c) = st.canvas.as_ref() else { return };
        c.begin(Color::rgba(0, 0, 0, 0.0));

        let mut y = PAD_Y * k;
        let x = PAD_X * k;
        let cw = CARD_W * k;
        for it in &items {
            let h = item_h(it);
            match it {
                Item::Header => {
                    let b = 24.0 * k;
                    let collapse = rectf(x, y, b, b);
                    let clear_w = btn_font.as_ref().map(|f| f.measure(chup_core::i18n::t().stack_clear_all, 1000.0).0).unwrap_or(40.0) + 16.0 * k;
                    let clear = rectf(x + cw - clear_w, y, clear_w, b);
                    for (r, hit) in [(collapse, Hit::Collapse), (clear, Hit::Clear)] {
                        c.shadow(r, 6.0 * k, 2.0 * k, 1.0 * k, theme.shadow);
                        let bg = if hover == Some(hit) { theme.surface2 } else { theme.surface };
                        c.fill_round(r, 6.0 * k, bg);
                        c.stroke_round(r, 6.0 * k, 1.0, theme.line);
                        hits.push((hit, to_rect(r)));
                    }
                    let chevron = if edge_right { Icon::ChevronRight } else { Icon::ChevronLeft };
                    c.icon(chevron, collapse.left + 5.0 * k, collapse.top + 5.0 * k, 14.0 * k, theme.text);
                    if let Some(f) = btn_font.as_ref() {
                        c.text(chup_core::i18n::t().stack_clear_all, f, clear, theme.text);
                    }
                }
                Item::Toast(i, _) => {
                    let t = &st.toasts[*i];
                    let r = rectf(x, y, cw, h);
                    c.shadow(r, 6.0 * k, 3.0 * k, 2.0 * k, theme.shadow);
                    c.fill_round(r, 6.0 * k, theme.surface);
                    c.stroke_round(r, 6.0 * k, 1.0, theme.line);
                    let bar = if t.toast.kind == ToastKind::Error { theme.danger } else { theme.accent };
                    c.fill_round(rectf(x, y, 3.0 * k, h), 1.5 * k, bar);
                    if let Some(f) = font.as_ref() {
                        c.text(&t.toast.message, f, rectf(x + 13.0 * k, y + 8.0 * k, cw - 23.0 * k, h - 16.0 * k), theme.text);
                    }
                    hits.push((Hit::Toast(t.id), to_rect(r)));
                }
                Item::Card(i, _) => {
                    let s = &st.shots[*i];
                    let r = rectf(x, y, cw, h);
                    let rad = 6.0 * k;
                    c.shadow(r, rad, 3.0 * k, 2.0 * k, theme.shadow);
                    c.fill_round(r, rad, theme.surface2);
                    if let Some(b) = st.bitmaps.get(&s.id) {
                        c.image_cover(b, r, rad);
                    }
                    c.stroke_round(r, rad, 1.0, theme.line);
                    let card_hovered =
                        matches!(hover, Some(Hit::Card(j) | Hit::Copy(j) | Hit::Folder(j) | Hit::Remove(j)) if j == *i);
                    let b = 24.0 * k;
                    let btns = [
                        (Hit::Copy(*i), Icon::Copy),
                        (Hit::Folder(*i), Icon::Folder),
                        (Hit::Remove(*i), Icon::Close),
                    ];
                    for (n, (hit, icon)) in btns.iter().enumerate() {
                        let bx = r.right - 4.0 * k - b * (3 - n) as f32 - 2.0 * k * (2 - n) as f32;
                        let br = rectf(bx, r.top + 4.0 * k, b, b);
                        if card_hovered {
                            let a = if hover == Some(*hit) { 0.85 } else { 0.6 };
                            c.fill_round(br, 6.0 * k, Color::rgba(0, 0, 0, a));
                            c.icon(*icon, br.left + 5.0 * k, br.top + 5.0 * k, 14.0 * k, Color::rgba(255, 255, 255, 1.0));
                        }
                        hits.push((*hit, to_rect(br)));
                    }
                    if card_hovered {
                        if let Some(f) = small.as_ref() {
                            let text = format!("{} × {}", s.width, s.height);
                            let (tw, th) = f.measure(&text, 1000.0);
                            let mr = rectf(r.left + 4.0 * k, r.bottom - 4.0 * k - th - 2.0 * k, tw + 10.0 * k, th + 2.0 * k);
                            c.fill_round(mr, 4.0 * k, Color::rgba(0, 0, 0, 0.55));
                            c.text(&text, f, mr, Color::rgba(255, 255, 255, 1.0));
                        }
                    }
                    hits.push((Hit::Card(*i), to_rect(r)));
                }
            }
            y += h + GAP * k;
        }

        if show_tab {
            let tx = if edge_right { total_w as f32 - TAB_W * k } else { 0.0 };
            let ty = total_h as f32 - (TAB_H + TAB_BOTTOM) * k;
            let r = rectf(tx, ty, TAB_W * k, TAB_H * k);
            c.shadow(r, 6.0 * k, 3.0 * k, 2.0 * k, theme.shadow);
            let bg = if hover == Some(Hit::Tab) { theme.surface2 } else { theme.surface };
            c.fill_round(r, 6.0 * k, bg);
            c.stroke_round(r, 6.0 * k, 1.0, theme.line);
            let chevron = if edge_right { Icon::ChevronLeft } else { Icon::ChevronRight };
            c.icon(chevron, tx + 7.0 * k, ty + 12.0 * k, 14.0 * k, theme.text);
            if let Some(f) = small.as_ref() {
                c.text(&st.shots.len().to_string(), f, rectf(tx, ty + 28.0 * k, TAB_W * k, 16.0 * k), theme.text);
            }
            hits.push((Hit::Tab, to_rect(r)));
        }
        c.end();

        // Neo góc dưới của work area (SPEC 5.4).
        let margin = (12.0 * k).round() as i32;
        let (px, py) = chup_core::coords::place_stack((total_w as u32, total_h as u32), &st.work, st.edge_right, margin);
        if let Some(s) = st.surface.as_ref() {
            s.present_part(self.hwnd, (px, py), (total_w, total_h), 255);
        }
        st.hits = hits;
        st.size = (total_w, total_h);
        let was_visible = std::mem::replace(&mut st.visible, true);
        let hidden = st.hidden_for_capture;
        drop(st);
        if !was_visible && !hidden {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    /// Hit theo vị trí con trỏ thật (toạ độ màn hình) và bố cục hiện tại: dùng lúc nhấn chuột, phòng khi bố cục vừa đổi
    /// (hover hiện header, cửa sổ lớn lên) sau khi Windows đã tính toạ độ của message.
    fn hit_cursor(&self) -> Option<Hit> {
        let mut r = windows::Win32::Foundation::RECT::default();
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(self.hwnd, &mut r) }.ok()?;
        let (cx, cy) = cursor_pos();
        self.hit(cx - r.left, cy - r.top)
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        // Nút trên thẻ được thêm trước thẻ nên tìm thấy trước.
        self.st.borrow().hits.iter().find(|(_, r)| r.contains_point(x, y)).map(|(h, _)| *h)
    }

    fn shot_id(&self, i: usize) -> Option<String> {
        self.st.borrow().shots.get(i).map(|s| s.id.clone())
    }

    fn begin_drag(&self, i: usize) {
        let Some(shot) = self.st.borrow().shots.get(i).cloned() else { return };
        if !shot.path.is_file() {
            // File bị xoá bên ngoài: App bỏ khỏi stack và báo (SPEC 7).
            emit(Action::ShotMissing(shot.id));
            return;
        }
        let image = drag::Image::File(shot.thumb_path.clone());
        let res = drag::start_drag(
            &HwndHandle(self.hwnd),
            drag::DragItem::Files(vec![shot.path.clone()]),
            image,
            |_, _| {},
            drag::Options::default(),
        );
        if let Err(e) = res {
            emit(Action::Toast(Toast::error(chup_core::i18n::fill(chup_core::i18n::t().err_drag, &[&e]))));
        }
    }
}

impl Handler for StackWin {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_MOUSEMOVE => {
                let (x, y) = (loword(lp.0), hiword(lp.0));
                let (enter, start_track) = {
                    let mut st = self.st.borrow_mut();
                    let enter = !st.hovering;
                    st.hovering = true;
                    (enter, !std::mem::replace(&mut st.tracking, true))
                };
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_LEAVE);
                }
                if start_track {
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
                // Còn nhớ lần nhấn mà phím trái đã nhả (mất WM_LBUTTONUP): bỏ, không để lần rê sau tự kéo file (chup-t8m).
                if wp.0 & MK_LBUTTON == 0 && self.st.borrow().press.is_some() {
                    log::debug!("stack: bỏ lần nhấn treo (phím trái đã nhả)");
                    self.st.borrow_mut().press = None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                // Kéo thẻ đủ xa thì bắt đầu kéo thả file.
                let drag_card = {
                    let st = self.st.borrow();
                    st.press.as_ref().and_then(|p| match p.hit {
                        Hit::Card(i) => {
                            let (cx, cy) = cursor_pos();
                            let d = (((cx - p.at.0).pow(2) + (cy - p.at.1).pow(2)) as f64).sqrt();
                            (d >= 4.0 * st.scale).then_some(i)
                        }
                        _ => None,
                    })
                };
                if let Some(i) = drag_card {
                    self.st.borrow_mut().press = None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    self.begin_drag(i);
                    return Some(LRESULT(0));
                }
                if enter {
                    self.relayout();
                }
                let h = self.hit(x, y);
                let changed = {
                    let mut st = self.st.borrow_mut();
                    std::mem::replace(&mut st.hover, h) != h
                };
                if changed {
                    self.relayout();
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                {
                    let mut st = self.st.borrow_mut();
                    st.tracking = false;
                    st.hover = None;
                }
                unsafe { SetTimer(Some(hwnd), TIMER_LEAVE, 400, None) };
                self.relayout();
                Some(LRESULT(0))
            }
            // Class có CS_DBLCLKS: lần nhấn thứ hai nhanh và gần lần trước tới dưới dạng DBLCLK. Không xử lý thì lần
            // nhấn đó mất, kéo thẻ không ăn (chup-t8m).
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                // Nhấn ngay khi vừa rê vào (chưa kịp xử lý WM_MOUSEMOVE): bật trạng thái hover trước để bố cục (header)
                // không đổi sau khi đã nhấn, rồi tính hit theo con trỏ thật.
                let enter = !std::mem::replace(&mut self.st.borrow_mut().hovering, true);
                if enter {
                    self.relayout();
                }
                let h = self.hit_cursor().or_else(|| self.hit(loword(lp.0), hiword(lp.0)));
                log::debug!("stack: nhấn {h:?} (vừa vào: {enter})");
                if let Some(hit) = h {
                    self.st.borrow_mut().press = Some(Press { hit, at: cursor_pos() });
                    unsafe { SetCapture(hwnd) };
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let up = self.hit(loword(lp.0), hiword(lp.0));
                let press = self.st.borrow_mut().press.take();
                unsafe {
                    let _ = ReleaseCapture();
                }
                let Some(p) = press else { return Some(LRESULT(0)) };
                if up != Some(p.hit) {
                    return Some(LRESULT(0));
                }
                match p.hit {
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
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                self.st.borrow_mut().press = None;
                Some(LRESULT(0))
            }
            WM_TIMER => {
                let id = wp.0;
                if id == TIMER_LEAVE {
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_LEAVE);
                    }
                    let changed = {
                        let mut st = self.st.borrow_mut();
                        std::mem::replace(&mut st.hovering, false)
                    };
                    if changed {
                        self.relayout();
                    }
                } else if id > TIMER_TOAST_BASE {
                    self.remove_toast((id - TIMER_TOAST_BASE) as u32);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
