//! Stack thumbnail (SPEC 5.4). Không bao giờ lấy focus. Cửa sổ co giãn đúng theo nội dung,
//! neo ở góc dưới của work area. Kéo thẻ ≥ 4px = kéo file PNG ra app khác; click = mở ảnh.

use std::{cell::RefCell, collections::HashMap};

use chup_core::{capture::Shot, coords::Rect, model::Toast};
use chup_ui::{
    stack::{self as ui_stack, Hit, StackFonts, StackState, ToastItem, DRAG_DIP, LEAVE_MS, MARGIN_DIP, MAX_TOASTS},
    Canvas as _,
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
    gfx::{self, Canvas, Color, Font, Surface},
    util::{cursor_pos, hiword, loword, HwndHandle},
    win::{self, emit, Handler},
};

const TIMER_LEAVE: usize = 1;
/// wParam của WM_MOUSEMOVE: phím trái đang giữ.
const MK_LBUTTON: usize = 0x0001;
const TIMER_TOAST_BASE: usize = 100;

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
    /// File đang upload (nút Upload của thẻ đó quay vòng bận).
    uploading: Option<std::path::PathBuf>,
}

pub struct StackWin {
    pub hwnd: HWND,
    st: RefCell<State>,
}

thread_local! {
    static CLASS: Vec<u16> = win::register_class::<StackWin>("ChupStack");
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
            if st.toasts.len() > MAX_TOASTS {
                st.toasts.remove(0);
            }
            id
        };
        let ms = ui_stack::toast_ms(&t);
        unsafe { SetTimer(Some(self.hwnd), TIMER_TOAST_BASE + id as usize, ms, None) };
        self.relayout();
    }

    /// File đang upload (`None` = không upload gì).
    pub fn set_uploading(&self, path: Option<std::path::PathBuf>) {
        let changed = {
            let mut st = self.st.borrow_mut();
            let changed = st.uploading != path;
            st.uploading = path;
            changed
        };
        if changed {
            self.relayout();
        }
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
        let theme = gfx::theme();
        let mut st = self.st.borrow_mut();
        let k = st.scale as f32;
        let [body, small, btn] = ui_stack::font_specs(k).map(Font::from_spec);
        let fonts = StackFonts { body, small, btn };
        let input = StackState {
            shots: &st.shots,
            toasts: &st.toasts,
            collapsed: st.collapsed,
            hovering: st.hovering,
            hover: st.hover,
            edge_right: st.edge_right,
            uploading: st.uploading.as_deref(),
            k,
        };
        let Some(layout) = ui_stack::layout(&input, &fonts) else {
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
        };
        let (total_w, total_h) = (layout.w, layout.h);

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
            let bmp = st.canvas.as_ref().and_then(|c| c.image(&st.shots[i].thumb));
            if let Some(b) = bmp {
                let id = st.shots[i].id.clone();
                st.bitmaps.insert(id, b);
            }
        }

        let Some(c) = st.canvas.as_ref() else { return };
        let input = StackState {
            shots: &st.shots,
            toasts: &st.toasts,
            collapsed: st.collapsed,
            hovering: st.hovering,
            hover: st.hover,
            edge_right: st.edge_right,
            uploading: st.uploading.as_deref(),
            k,
        };
        c.begin(Color::CLEAR);
        let hits = ui_stack::paint(c, &theme, &fonts, &input, &layout, |id| st.bitmaps.get(id));
        c.end();

        // Neo góc dưới của work area (SPEC 5.4).
        let margin = (MARGIN_DIP * k).round() as i32;
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
        ui_stack::hit_at(&self.st.borrow().hits, x, y)
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
                            (d >= DRAG_DIP * st.scale).then_some(i)
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
                unsafe { SetTimer(Some(hwnd), TIMER_LEAVE, LEAVE_MS, None) };
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
                    Hit::Upload(i) => {
                        if let Some(id) = self.shot_id(i) {
                            let (x, y) = cursor_pos();
                            // Menu chạy vòng message riêng: không giữ borrow nào lúc này.
                            if let Some(kind) = super::share::pick(self.hwnd, x, y) {
                                emit(Action::ShotUpload(id, kind));
                            }
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
