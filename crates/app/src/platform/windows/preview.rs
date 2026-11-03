//! Cửa sổ Preview (SPEC 17): ảnh to khi bấm thumbnail. Chỉ có 1 cửa sổ, tạo lúc khởi động và ẩn đi khi đóng;
//! ảnh, DIB và bitmap Direct2D chỉ tồn tại khi đang mở.
//!
//! Layered window: opacity là `SourceConstantAlpha`. Vẽ 2 lớp:
//! - `base`: ảnh đã scale + nền + viền, chỉ vẽ lại khi kích thước đổi.
//! - `out`: chép `base` rồi vẽ vùng chọn chữ, toolbar (khi hover) và nhãn zoom/opacity, đẩy lên cửa sổ.
//!
//! Chọn chữ (SPEC 17.2): App gửi các từ OCR nhận diện được (`set_text`), kéo trên chữ để chọn theo ký tự như Live Text.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    sync::Arc,
};

use chup_core::{
    coords::{monitor_at, Rect},
    icons::Icon,
    ocr::{Pos, TextLayout},
    preview::{self, View},
    selection::Grip,
};
use image::RgbaImage;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Direct2D::ID2D1Bitmap,
    UI::{
        Controls::WM_MOUSELEAVE,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            GetCapture, GetDoubleClickTime, GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
            VK_ADD, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_NUMPAD0, VK_OEM_MINUS, VK_OEM_PLUS, VK_SHIFT, VK_SUBTRACT,
        },
        WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, DestroyMenu, GetMessageTime, GetSystemMetrics, GetWindowRect, KillTimer, LoadCursorW,
            SendMessageW, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow,
            TrackPopupMenu, HTCAPTION, HWND_NOTOPMOST, HWND_TOPMOST, IDC_HAND, IDC_IBEAM, IDC_SIZENESW, IDC_SIZENS,
            IDC_SIZENWSE, IDC_SIZEWE, MF_GRAYED, MF_SEPARATOR, MF_STRING, SM_CXDOUBLECLK, SM_CXDRAG, SM_CYDOUBLECLK, SM_CYDRAG, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_CAPTURECHANGED,
            WM_CLOSE, WM_CONTEXTMENU, WM_DPICHANGED, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCLBUTTONDOWN, WM_SETCURSOR, WM_TIMER, WS_EX_LAYERED, WS_POPUP,
        },
    },
};

use super::{
    app::Action,
    gfx::{rectf, Align, Canvas, Color, Font, Surface, Theme},
    util::{self, cursor_pos, hiword, loword, wide},
    win::{self, emit, Handler},
};

// Kích thước (DIP).
const MIN_W: f32 = 160.0;
const MIN_H: f32 = 48.0;
const RADIUS: f32 = 8.0;
const BTN: f32 = 28.0;
const BTN_GAP: f32 = 4.0;
const BAR_PAD: f32 = 8.0;
/// Bề rộng dải mép cửa sổ để kéo đổi cỡ.
const EDGE: f32 = 6.0;
/// Nới vùng nhận chuột quanh dòng chữ (DIP).
const TEXT_PAD: f32 = 4.0;
/// Độ đục của màu tô vùng chọn chữ.
const SEL_ALPHA: f32 = 0.35;

// Lệnh của menu chuột phải.
const CMD_COPY_TEXT: usize = 1;
const CMD_SELECT_ALL: usize = 2;
const CMD_COPY_IMAGE: usize = 3;

const TIMER_PILL: usize = 1;
const PILL_MS: u32 = 1200;
/// Zoom bằng lăn chuột xong một lúc thì mới báo App lưu khung (không ghi file mỗi nấc).
const TIMER_SAVE: usize = 2;
/// Đổi DPI: vẽ lại sau, không vẽ ngay trong message Windows gửi giữa lúc đang đẩy ảnh lên cửa sổ.
const TIMER_DPI: usize = 3;
const SAVE_MS: u32 = 700;
const MK_CONTROL: usize = 0x0008;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    /// OCR: copy chữ trong ảnh (SPEC 15.2). Đang OCR thì vẽ vòng "bận", bấm lần nữa là huỷ.
    Ocr,
    Pin,
    Copy,
    Folder,
    Close,
}

const BUTTONS: [(Btn, Icon); 5] = [
    (Btn::Ocr, Icon::Text),
    (Btn::Pin, Icon::Pin),
    (Btn::Copy, Icon::Copy),
    (Btn::Folder, Icon::Folder),
    (Btn::Close, Icon::Close),
];

#[derive(Default)]
struct State {
    /// File vừa được yêu cầu mở, đang giải mã ở worker. Chỉ nhận ảnh khớp file này.
    pending: Option<PathBuf>,
    path: PathBuf,
    img: Option<RgbaImage>,
    /// `None` = đang ẩn.
    view: Option<View>,
    /// View lúc mở (phím 0 quay về).
    home_zoom: f64,
    work: Rect,
    topmost: bool,
    opacity: u8,
    /// Giá trị Cài đặt đã áp lần cuối: chỉ áp lại khi Cài đặt đổi, để không đè ghim/opacity người dùng vừa chỉnh.
    cfg: Option<(bool, u8)>,
    base: Option<Surface>,
    out: Option<Surface>,
    canvas: Option<Canvas>,
    bmp: Option<ID2D1Bitmap>,
    hover: Option<Btn>,
    hovering: bool,
    tracking: bool,
    press: Option<Btn>,
    pill: Option<String>,
    hits: Vec<(Btn, Rect)>,
    /// Phần lẻ của lăn chuột (chuột lăn mịn gửi delta < 120).
    wheel: i32,
    /// Đang kéo mép để đổi cỡ: mép nào, view lúc bắt đầu, điểm bắt đầu (toạ độ màn hình).
    resize: Option<(Grip, View, (i32, i32))>,
    /// Đã zoom, chưa báo App lưu khung (đợi `TIMER_SAVE`).
    save_pending: bool,
    /// File đang được OCR (nút [T] vẽ "bận" khi trùng file đang xem).
    ocr_busy: Option<PathBuf>,
    /// Chữ nhận diện được của ảnh đang xem.
    text: Option<Arc<TextLayout>>,
    /// Vùng chọn chữ: (vị trí neo, vị trí đang kéo tới).
    sel: Option<(Pos, Pos)>,
    /// Đang bấm trên chữ: vị trí neo và điểm bấm (toạ độ cửa sổ). Kéo quá ngưỡng mới bắt đầu chọn.
    text_press: Option<(Pos, (i32, i32))>,
    /// Đã kéo quá ngưỡng (hoặc Shift + bấm): đang chọn chữ.
    selecting: bool,
    /// Lần bấm đúp trên chữ gần nhất: thời điểm (`GetMessageTime`) và điểm, để nhận ra bấm 3 lần.
    last_dbl: Option<(i32, (i32, i32))>,
}

impl State {
    /// Bỏ chữ và vùng chọn (đổi ảnh, đóng).
    fn clear_text(&mut self) {
        self.text = None;
        self.sel = None;
        self.text_press = None;
        self.selecting = false;
        self.last_dbl = None;
    }
}

fn alt_down() -> bool {
    (unsafe { GetKeyState(VK_MENU.0 as i32) }) < 0
}

fn shift_down() -> bool {
    (unsafe { GetKeyState(VK_SHIFT.0 as i32) }) < 0
}

/// Cỡ cửa sổ nhỏ nhất (physical px) ở scale `k`.
pub fn min_size(k: f64) -> (u32, u32) {
    ((MIN_W as f64 * k).round() as u32, (MIN_H as f64 * k).round() as u32)
}

pub struct Preview {
    pub hwnd: HWND,
    /// Đang mở. Để ngoài `st`: wndproc đọc cờ này ở mọi message, kể cả message Windows gửi đồng bộ
    /// trong lúc `st` đang bị borrow (UpdateLayeredWindow đổi vị trí/cỡ gửi WM_WINDOWPOSCHANGED...).
    open: Cell<bool>,
    st: RefCell<State>,
}

thread_local! {
    static CLASS: Vec<u16> = win::register_class::<Preview>("ChupPreview");
}

impl Preview {
    pub fn create() -> windows::core::Result<std::rc::Rc<Preview>> {
        let p = CLASS.with(|cls| {
            win::create(
                win::CreateParams {
                    class: cls,
                    title: "Chup",
                    style: WS_POPUP,
                    // Không WS_EX_TOOLWINDOW: hiện trên taskbar và Alt+Tab để tìm lại khi không ghim.
                    ex_style: WS_EX_LAYERED,
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1,
                    parent: None,
                },
                |hwnd| Preview { hwnd, open: Cell::new(false), st: RefCell::new(State::default()) },
            )
        })?;
        util::set_app_icon(p.hwnd);
        Ok(p)
    }

    pub fn is_visible(&self) -> bool {
        self.open.get()
    }

    /// Bắt đầu mở `path` (App giải mã ở worker rồi gọi `show`).
    pub fn request(&self, path: PathBuf) {
        self.st.borrow_mut().pending = Some(path);
    }

    pub fn is_pending(&self, path: &PathBuf) -> bool {
        self.st.borrow().pending.as_ref() == Some(path)
    }

    pub fn cancel_pending(&self) {
        self.st.borrow_mut().pending = None;
    }

    /// Hiện ảnh vừa giải mã ở `view` (App tính chỗ và cỡ mở). Đang mở ảnh khác thì thay.
    pub fn show(&self, path: PathBuf, img: RgbaImage, view: View, work: Rect, topmost: bool, opacity: u8) {
        let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // Đặt cửa sổ (đang ẩn) vào chỗ đích trước khi vẽ, để GetDpiForWindow trả DPI của màn hình đó.
        let r = view.rect;
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, r.x, r.y, r.w as i32, r.h as i32, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        {
            let mut st = self.st.borrow_mut();
            st.pending = None;
            st.home_zoom = view.zoom;
            st.view = Some(view);
            st.work = work;
            st.resize = None;
            st.path = path;
            st.img = Some(img);
            // Ảnh mới: bitmap cũ không dùng được nữa.
            st.bmp = None;
            st.topmost = topmost;
            st.opacity = preview::clamp_opacity(opacity);
            st.cfg = Some((topmost, opacity));
            st.pill = None;
            st.clear_text();
        }
        let t = wide(&format!("{title} — Chup"));
        unsafe {
            let _ = SetWindowTextW(self.hwnd, windows::core::PCWSTR(t.as_ptr()));
        }
        self.open.set(true);
        if !self.rescale() {
            self.close();
            emit(Action::Toast(chup_core::model::Toast::error(chup_core::i18n::t().err_preview_memory)));
            return;
        }
        self.paint();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.apply_z(topmost);
    }

    /// Cài đặt đổi: áp phần Preview nếu khác lần trước.
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
            self.repaint();
        }
    }

    /// Ẩn và bỏ ảnh, DIB, bitmap.
    pub fn close(&self) {
        // Khung đang chờ lưu (vừa zoom xong) thì lưu luôn trước khi đóng.
        self.sync_pos();
        let unsaved = {
            let mut st = self.st.borrow_mut();
            let pending = std::mem::take(&mut st.save_pending);
            st.view.filter(|_| pending).map(|v| v.rect)
        };
        if let Some(r) = unsaved {
            emit(Action::PreviewBox(r));
        }
        let was_open = self.open.replace(false);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_PILL);
            let _ = KillTimer(Some(self.hwnd), TIMER_SAVE);
            let _ = KillTimer(Some(self.hwnd), TIMER_DPI);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        {
            let mut st = self.st.borrow_mut();
            st.view = None;
            // Đóng trong lúc ảnh khác đang giải mã: không tự mở lại khi giải mã xong.
            st.pending = None;
            st.bmp = None;
            st.canvas = None;
            st.base = None;
            st.out = None;
            st.img = None;
            st.cfg = None;
            st.hover = None;
            st.hovering = false;
            st.tracking = false;
            st.press = None;
            st.resize = None;
            st.pill = None;
            st.hits.clear();
            st.clear_text();
        }
        // Đóng giữa lúc đang kéo (chọn chữ, đổi cỡ, bấm nút): cửa sổ ẩn không còn xử lý WM_LBUTTONUP để nhả capture.
        if unsafe { GetCapture() } == self.hwnd {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        if was_open {
            emit(Action::PreviewClosed);
        }
    }

    fn scale(&self) -> f32 {
        unsafe { GetDpiForWindow(self.hwnd) }.max(96) as f32 / 96.0
    }

    fn apply_z(&self, topmost: bool) {
        let after = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    /// Cửa sổ có thể đã bị kéo đi: đọc lại vị trí thật.
    fn sync_pos(&self) {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut r) }.is_err() {
            return;
        }
        if let Some(v) = self.st.borrow_mut().view.as_mut() {
            v.rect.x = r.left;
            v.rect.y = r.top;
        }
    }

    /// Vẽ lại `base` theo kích thước của view. `false` = không cấp được bộ nhớ.
    fn rescale(&self) -> bool {
        let theme = Theme::current();
        let k = self.scale();
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return false };
        let (w, h) = (v.rect.w as i32, v.rect.h as i32);
        if st.base.as_ref().map(|s| (s.w, s.h)) != Some((w, h)) {
            st.base = None;
            st.out = None;
            st.base = Surface::new(w, h);
            st.out = Surface::new(w, h);
        }
        if st.out.is_none() {
            st.out = Surface::new(w, h);
        }
        let Some(base) = st.base.as_ref() else { return false };
        if st.out.is_none() {
            return false;
        }
        if st.canvas.is_none() {
            st.canvas = Canvas::new(base);
            st.bmp = None;
        }
        let Some(c) = st.canvas.as_ref() else { return false };
        if st.bmp.is_none() {
            st.bmp = c.bitmap(img);
        }
        let Some(bmp) = st.bmp.as_ref() else { return false };
        if !c.bind(base) {
            return false;
        }
        let t0 = std::time::Instant::now();
        let rad = RADIUS * k;
        let full = rectf(0.0, 0.0, w as f32, h as f32);
        // Alpha 1/255 thay vì 0: pixel alpha 0 của layered window không nhận chuột, mà góc bo tròn
        // vẫn phải kéo được để đổi cỡ.
        c.begin(Color::rgba(0, 0, 0, 1.0 / 255.0));
        let ir = v.image_rect(img.dimensions());
        let fills = ir.w as i32 == w && ir.h as i32 == h;
        if !fills {
            // Ảnh nhỏ hơn cỡ tối thiểu: nằm giữa trên nền tối.
            c.fill_round(full, rad, Color::rgba(24, 24, 27, 1.0));
        }
        let img_r = rectf(ir.x as f32, ir.y as f32, ir.w as f32, ir.h as f32);
        c.image_cover(bmp, img_r, if fills { rad } else { 0.0 });
        c.stroke_round(rectf(0.5, 0.5, w as f32 - 1.0, h as f32 - 1.0), rad, 1.0, theme.line);
        c.end();
        log::debug!("preview: vẽ ảnh {}×{} mất {:?}", w, h, t0.elapsed());
        true
    }

    /// Chép `base` sang `out`, vẽ toolbar và nhãn, đẩy lên cửa sổ.
    /// `view.rect` phải là vị trí thật (gọi `repaint` nếu không chủ động đổi vị trí/cỡ).
    fn paint(&self) {
        let k = self.scale();
        // Lấy `out` ra khỏi state rồi mới đẩy lên cửa sổ: UpdateLayeredWindow có thể gửi ngược message
        // (WM_WINDOWPOSCHANGED, WM_DPICHANGED) vào wndproc, không được giữ borrow lúc đó.
        let Some((out, pos, alpha)) = self.compose(k) else { return };
        out.present(self.hwnd, pos, alpha, None);
        let mut st = self.st.borrow_mut();
        let same = st.base.as_ref().map(|b| (b.w, b.h)) == Some((out.w, out.h));
        if self.open.get() && st.out.is_none() && same {
            st.out = Some(out);
        }
    }

    /// Vẽ lại khi vị trí/cỡ không đổi chủ động (hover, nhãn, opacity, ghim): đọc lại vị trí thật trước,
    /// phòng khi cửa sổ đang bị kéo hoặc vừa bị dời bằng Win+Shift+mũi tên.
    fn repaint(&self) {
        self.sync_pos();
        self.paint();
    }

    fn compose(&self, k: f32) -> Option<(Surface, (i32, i32), u8)> {
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let v = st.view?;
        let mut owned = st.out.take()?;
        let (Some(base), Some(c)) = (st.base.as_mut(), st.canvas.as_ref()) else {
            st.out = Some(owned);
            return None;
        };
        let out = &mut owned;
        out.pixels().copy_from_slice(base.pixels());

        // Toolbar góc trên-phải (luôn tính vùng bấm; chỉ vẽ khi hover).
        let (w, h) = (out.w as f32, out.h as f32);
        let b = BTN * k;
        let n = BUTTONS.len() as f32;
        let x0 = w - BAR_PAD * k - n * b - (n - 1.0) * BTN_GAP * k;
        st.hits.clear();
        let draw_bar = st.hovering;
        let draw_pill = st.pill.is_some();
        // Vùng chọn chữ: rect theo toạ độ ảnh gốc -> toạ độ cửa sổ.
        let sel_rects: Vec<_> = match (st.text.as_ref(), st.sel, st.img.as_ref()) {
            (Some(t), Some((a, b)), Some(img)) => {
                let ir = v.image_rect(img.dimensions());
                let z = v.zoom as f32;
                let pad = k;
                t.highlight(a, b)
                    .iter()
                    .map(|r| {
                        let x = ir.x as f32 + r.x as f32 * z - pad;
                        let y = ir.y as f32 + r.y as f32 * z - pad;
                        rectf(x, y, r.w as f32 * z + 2.0 * pad, r.h as f32 * z + 2.0 * pad)
                    })
                    .collect()
            }
            _ => vec![],
        };
        if (draw_bar || draw_pill || !sel_rects.is_empty()) && c.bind(out) {
            c.begin_over();
            let sel_color = Theme::current().accent.with_alpha(SEL_ALPHA);
            for r in &sel_rects {
                c.fill_round(*r, 2.0 * k, sel_color);
            }
            for (i, (btn, icon)) in BUTTONS.iter().enumerate() {
                let r = rectf(x0 + i as f32 * (b + BTN_GAP * k), BAR_PAD * k, b, b);
                st.hits.push((*btn, Rect::new(r.left as i32, r.top as i32, b as u32, b as u32)));
                if !draw_bar {
                    continue;
                }
                let hot = st.hover == Some(*btn);
                let busy = *btn == Btn::Ocr && st.ocr_busy.as_ref() == Some(&st.path);
                let icon = if busy { &Icon::Busy } else { icon };
                let bg = if (*btn == Btn::Pin && st.topmost) || busy {
                    Theme::current().accent.with_alpha(if hot { 1.0 } else { 0.9 })
                } else {
                    Color::rgba(0, 0, 0, if hot { 0.85 } else { 0.6 })
                };
                c.fill_round(r, 6.0 * k, bg);
                let is = 16.0 * k;
                c.icon(*icon, r.left + (b - is) / 2.0, r.top + (b - is) / 2.0, is, Color::rgba(255, 255, 255, 1.0));
            }
            if let (Some(text), Some(f)) = (st.pill.as_ref(), Font::new(12.0 * k, false, Align::Center, false)) {
                let (tw, th) = f.measure(text, 10_000.0);
                let (pw, ph) = (tw + 20.0 * k, th + 8.0 * k);
                let r = rectf((w - pw) / 2.0, h - 12.0 * k - ph, pw, ph);
                c.fill_round(r, ph / 2.0, Color::rgba(0, 0, 0, 0.7));
                c.text(text, &f, r, Color::rgba(255, 255, 255, 1.0));
            }
            c.end();
        } else {
            for (i, (btn, _)) in BUTTONS.iter().enumerate() {
                let x = x0 + i as f32 * (b + BTN_GAP * k);
                st.hits.push((*btn, Rect::new(x as i32, (BAR_PAD * k) as i32, b as u32, b as u32)));
            }
        }
        Some((owned, (v.rect.x, v.rect.y), preview::alpha(st.opacity)))
    }

    /// Báo App khung hiện tại (App lưu nếu đang chọn "như lần trước").
    fn save_box(&self) {
        self.sync_pos();
        let r = {
            let mut st = self.st.borrow_mut();
            st.save_pending = false;
            st.view.map(|v| v.rect)
        };
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SAVE);
        }
        if let Some(r) = r {
            emit(Action::PreviewBox(r));
        }
    }

    fn schedule_save(&self) {
        self.st.borrow_mut().save_pending = true;
        unsafe { SetTimer(Some(self.hwnd), TIMER_SAVE, SAVE_MS, None) };
    }

    /// Mép để đổi cỡ dưới điểm (toạ độ cửa sổ). Nút toolbar được ưu tiên.
    fn edge(&self, x: i32, y: i32) -> Option<Grip> {
        let band = (EDGE * self.scale()).round() as i32;
        let st = self.st.borrow();
        let v = st.view?;
        preview::edge_hit((v.rect.w, v.rect.h), (x, y), band)
    }

    fn hit(&self, x: i32, y: i32) -> Option<Btn> {
        self.st.borrow().hits.iter().find(|(_, r)| r.contains_point(x, y)).map(|(b, _)| *b)
    }

    fn show_pill(&self, text: String) {
        self.st.borrow_mut().pill = Some(text);
        unsafe { SetTimer(Some(self.hwnd), TIMER_PILL, PILL_MS, None) };
    }

    /// Work area của màn hình chứa tâm cửa sổ (người dùng có thể đã kéo sang màn khác).
    fn current_work(&self) -> Rect {
        let st = self.st.borrow();
        let Some(v) = st.view else { return st.work };
        let (cx, cy) = v.rect.center();
        monitor_at(&util::monitors(), cx, cy).map(|m| m.work).unwrap_or(st.work)
    }

    fn min_size(&self) -> (u32, u32) {
        min_size(self.scale() as f64)
    }

    /// Đổi zoom (`steps` nấc, hoặc về `to`) quanh `anchor`.
    fn zoom(&self, steps: i32, to: Option<f64>, anchor: (i32, i32)) {
        self.sync_pos();
        let work = self.current_work();
        let min = self.min_size();
        let zoom = {
            let mut st = self.st.borrow_mut();
            let (Some(v), Some(img)) = (st.view, st.img.as_ref()) else { return };
            let dims = img.dimensions();
            let nv = match to {
                Some(z) => preview::resize_to(v, dims, z, anchor, work, min),
                None => preview::zoom_at(v, dims, steps, anchor, work, min),
            };
            st.view = Some(nv);
            st.work = work;
            nv.zoom
        };
        self.show_pill(format!("{}%", (zoom * 100.0).round()));
        if !self.rescale() {
            self.close();
            return;
        }
        self.paint();
        self.schedule_save();
    }

    /// Đang kéo mép: tính cỡ mới theo con trỏ.
    fn resize_move(&self) {
        let work = self.current_work();
        let min = self.min_size();
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

    /// Kết thúc kéo mép (thả chuột hoặc mất capture).
    fn resize_end(&self) -> bool {
        if self.st.borrow_mut().resize.take().is_none() {
            return false;
        }
        self.save_box();
        true
    }

    fn center(&self) -> (i32, i32) {
        self.sync_pos();
        self.st.borrow().view.map(|v| v.rect.center()).unwrap_or_default()
    }

    fn change_opacity(&self, steps: i32) {
        let op = {
            let mut st = self.st.borrow_mut();
            st.opacity = preview::step_opacity(st.opacity, steps);
            st.opacity
        };
        self.show_pill(format!("Opacity {op}%"));
        self.repaint();
    }

    fn toggle_pin(&self) {
        let top = {
            let mut st = self.st.borrow_mut();
            st.topmost = !st.topmost;
            st.topmost
        };
        self.apply_z(top);
        self.repaint();
    }

    /// File đang được OCR (`None` = không OCR gì): nút [T] của đúng file đó vẽ vòng "bận".
    pub fn set_ocr_busy(&self, busy: Option<PathBuf>) {
        let changed = {
            let mut st = self.st.borrow_mut();
            let changed = st.ocr_busy != busy;
            st.ocr_busy = busy;
            changed
        };
        if changed && self.open.get() {
            self.repaint();
        }
    }

    /// Chữ OCR nhận diện được của `path`: chỉ nhận nếu đúng ảnh đang xem. Vùng chọn cũ bỏ đi.
    pub fn set_text(&self, path: &std::path::Path, text: Arc<TextLayout>) {
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
            self.repaint();
        }
    }

    /// Điểm (toạ độ cửa sổ) -> toạ độ ảnh gốc, cùng chữ của ảnh và độ nới (px ảnh) cho `hit`.
    fn to_image(&self, x: i32, y: i32) -> Option<(Arc<TextLayout>, (i32, i32), i32)> {
        let k = self.scale();
        let st = self.st.borrow();
        let (t, v, img) = (st.text.as_ref()?, st.view?, st.img.as_ref()?);
        if t.is_empty() {
            return None;
        }
        let ir = v.image_rect(img.dimensions());
        let z = v.zoom.max(1e-6);
        let p = (((x - ir.x) as f64 / z).floor() as i32, ((y - ir.y) as f64 / z).floor() as i32);
        let pad = (TEXT_PAD as f64 * k as f64 / z).ceil() as i32;
        Some((t.clone(), p, pad))
    }

    /// Vị trí chữ dưới con trỏ (toạ độ cửa sổ). Giữ `Alt` thì không tính là chữ (kéo để di chuyển).
    fn text_at(&self, x: i32, y: i32) -> Option<Pos> {
        if alt_down() {
            return None;
        }
        let (t, p, pad) = self.to_image(x, y)?;
        t.hit(p, pad)
    }

    /// Đang kéo chọn: đổi vị trí cuối của vùng chọn theo con trỏ.
    fn drag_select(&self, x: i32, y: i32) {
        let changed = {
            let Some((t, p, _)) = self.to_image(x, y) else { return };
            let Some(focus) = t.nearest(p) else { return };
            let mut st = self.st.borrow_mut();
            let Some((anchor, start)) = st.text_press else { return };
            if !st.selecting {
                let (dx, dy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
                if (x - start.0).abs() <= dx && (y - start.1).abs() <= dy {
                    return;
                }
                st.selecting = true;
            }
            st.sel.replace((anchor, focus)) != Some((anchor, focus))
        };
        if changed {
            self.repaint();
        }
    }

    fn set_sel(&self, sel: Option<(Pos, Pos)>) {
        let changed = std::mem::replace(&mut self.st.borrow_mut().sel, sel) != sel;
        if changed {
            self.repaint();
        }
    }

    fn select_all(&self) {
        let all = self.st.borrow().text.as_ref().and_then(|t| t.all());
        if all.is_some() {
            self.set_sel(all);
        }
    }

    /// Bấm trên chữ ở vị trí `p`: bấm thứ 3 liền sau bấm đúp thì chọn cả dòng; Shift thì nới vùng chọn đang có;
    /// còn lại thì chờ kéo để chọn.
    fn text_down(&self, p: Pos, x: i32, y: i32) {
        let now = unsafe { GetMessageTime() };
        let row = {
            let mut st = self.st.borrow_mut();
            let triple = st.last_dbl.take().is_some_and(|(t, (dx, dy))| {
                let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK)) };
                now.wrapping_sub(t) as u32 <= unsafe { GetDoubleClickTime() } && (x - dx).abs() <= cx && (y - dy).abs() <= cy
            });
            match (triple, st.text.clone(), st.sel) {
                (true, Some(t), _) => Some(t.row_range(p)),
                (_, _, Some((anchor, _))) if shift_down() => {
                    st.text_press = Some((anchor, (x, y)));
                    st.selecting = true;
                    st.sel = Some((anchor, p));
                    None
                }
                _ => {
                    st.text_press = Some((p, (x, y)));
                    st.selecting = false;
                    None
                }
            }
        };
        match row {
            Some(r) => self.set_sel(Some(r)),
            None => {
                unsafe { SetCapture(self.hwnd) };
                self.repaint();
            }
        }
    }

    /// Chữ đang chọn (rỗng = không chọn gì).
    fn selected_text(&self) -> Option<String> {
        let st = self.st.borrow();
        let (t, (a, b)) = (st.text.as_ref()?, st.sel?);
        Some(t.text(a, b)).filter(|s| !s.is_empty())
    }

    /// `Ctrl+C`, menu: copy chữ đang chọn, không chọn gì thì copy ảnh.
    fn copy(&self) {
        match self.selected_text() {
            Some(text) => emit(Action::CopyText(text)),
            None => emit(Action::PreviewCopy(self.st.borrow().path.clone())),
        }
    }

    /// Menu chuột phải tại điểm màn hình `(x, y)`.
    fn context_menu(&self, x: i32, y: i32) {
        let has_sel = self.selected_text().is_some();
        let has_text = self.st.borrow().text.as_ref().is_some_and(|t| !t.is_empty());
        let item = |enabled: bool| if enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
        let t = chup_core::i18n::t();
        let (copy_text, select_all, copy_image) =
            (wide(t.menu_copy_text), wide(t.menu_select_all_text), wide(t.menu_copy_image));
        let cmd = unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let pc = |w: &Vec<u16>| windows::core::PCWSTR(w.as_ptr());
            let _ = AppendMenuW(menu, item(has_sel), CMD_COPY_TEXT, pc(&copy_text));
            let _ = AppendMenuW(menu, item(has_text), CMD_SELECT_ALL, pc(&select_all));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, CMD_COPY_IMAGE, pc(&copy_image));
            // Không giữ borrow: menu chạy vòng lặp message riêng (wndproc vẫn nhận WM_MOUSELEAVE...).
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, None, self.hwnd, None);
            let _ = DestroyMenu(menu);
            cmd.0 as usize
        };
        match cmd {
            CMD_COPY_TEXT => self.copy(),
            CMD_SELECT_ALL => self.select_all(),
            CMD_COPY_IMAGE => emit(Action::PreviewCopy(self.st.borrow().path.clone())),
            _ => {}
        }
    }

    /// Bản sao ảnh đang xem nếu đúng file `path` (cho OCR, không phải đọc lại file).
    pub fn image_of(&self, path: &std::path::Path) -> Option<RgbaImage> {
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
}

impl Handler for Preview {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if msg == WM_CLOSE {
            // Alt+F4, đóng từ taskbar, WM_CLOSE từ tool khác: ẩn chứ không huỷ cửa sổ (kể cả khi đang ẩn).
            self.close();
            return Some(LRESULT(0));
        }
        if !self.open.get() {
            return None;
        }
        match msg {
            WM_MOUSEMOVE => {
                if self.st.borrow().resize.is_some() {
                    self.resize_move();
                    return Some(LRESULT(0));
                }
                if self.st.borrow().text_press.is_some() {
                    self.drag_select(loword(lp.0), hiword(lp.0));
                    return Some(LRESULT(0));
                }
                let h = self.hit(loword(lp.0), hiword(lp.0));
                let (changed, start_track) = {
                    let mut st = self.st.borrow_mut();
                    let entered = !std::mem::replace(&mut st.hovering, true);
                    let moved = std::mem::replace(&mut st.hover, h) != h;
                    (entered || moved, !std::mem::replace(&mut st.tracking, true))
                };
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
                if changed {
                    self.repaint();
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                {
                    let mut st = self.st.borrow_mut();
                    st.tracking = false;
                    st.hovering = false;
                    st.hover = None;
                }
                self.repaint();
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                let mut r = RECT::default();
                let _ = unsafe { GetWindowRect(hwnd, &mut r) };
                let (cx, cy) = cursor_pos();
                let (x, y) = (cx - r.left, cy - r.top);
                let resizing = self.st.borrow().resize.map(|(g, _, _)| g);
                let cursor = if let Some(g) = resizing.or_else(|| self.hit(x, y).is_none().then(|| self.edge(x, y)).flatten())
                {
                    match g {
                        Grip::N | Grip::S => IDC_SIZENS,
                        Grip::E | Grip::W => IDC_SIZEWE,
                        Grip::NW | Grip::SE => IDC_SIZENWSE,
                        _ => IDC_SIZENESW,
                    }
                } else if self.hit(x, y).is_some() {
                    IDC_HAND
                } else if self.st.borrow().text_press.is_some() || self.text_at(x, y).is_some() {
                    IDC_IBEAM
                } else {
                    return None;
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                Some(LRESULT(1))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = (loword(lp.0), hiword(lp.0));
                if let Some(b) = self.hit(x, y) {
                    self.st.borrow_mut().press = Some(b);
                    unsafe { SetCapture(hwnd) };
                } else if let Some(g) = self.edge(x, y) {
                    self.sync_pos();
                    let start = {
                        let mut st = self.st.borrow_mut();
                        st.resize = st.view.map(|v| (g, v, cursor_pos()));
                        st.resize.is_some()
                    };
                    if start {
                        unsafe { SetCapture(hwnd) };
                    }
                } else if let Some(p) = self.text_at(x, y) {
                    // Bấm trên chữ: kéo để chọn (vùng chọn cũ giữ tới khi kéo hoặc thả).
                    self.text_down(p, x, y);
                } else {
                    self.set_sel(None);
                    // Kéo ở bất kỳ đâu ngoài nút và chữ: để Windows di chuyển cửa sổ.
                    let (cx, cy) = cursor_pos();
                    let pt = ((cy as u16 as isize) << 16) | (cx as u16 as isize);
                    unsafe {
                        let _ = ReleaseCapture();
                        SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(pt)));
                    }
                    // Vòng lặp di chuyển của Windows trả về khi thả chuột.
                    self.save_box();
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDBLCLK => {
                // Bấm đúp trên chữ: chọn từ đó. Trên nút: bỏ qua (không thành 2 lần bấm, vd. [T] bấm rồi huỷ).
                // Chỗ khác: như bấm đơn (class có CS_DBLCLKS).
                let (x, y) = (loword(lp.0), hiword(lp.0));
                if self.hit(x, y).is_some() {
                    return Some(LRESULT(0));
                }
                let on_text = self.text_at(x, y).filter(|_| self.hit(x, y).is_none() && self.edge(x, y).is_none());
                let range = on_text.and_then(|p| self.st.borrow().text.as_ref().map(|t| t.word_range(p)));
                match range {
                    Some(r) => {
                        self.st.borrow_mut().last_dbl = Some((unsafe { GetMessageTime() }, (x, y)));
                        self.set_sel(Some(r));
                    }
                    None => return self.handle(hwnd, WM_LBUTTONDOWN, wp, lp),
                }
                Some(LRESULT(0))
            }
            WM_CONTEXTMENU => {
                // lParam là toạ độ màn hình; -1 = mở bằng bàn phím (Shift+F10): đặt menu ở góc trên-trái ảnh.
                let (mut x, mut y) = (loword(lp.0), hiword(lp.0));
                if x == -1 && y == -1 {
                    let r = self.st.borrow().view.map(|v| v.rect).unwrap_or_default();
                    (x, y) = (r.x + 16, r.y + 16);
                }
                self.context_menu(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let text_click = {
                    let mut st = self.st.borrow_mut();
                    st.text_press.take().map(|_| !std::mem::take(&mut st.selecting))
                };
                if let Some(click) = text_click {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    // Bấm rồi thả, không kéo: bỏ vùng chọn.
                    if click {
                        self.set_sel(None);
                    }
                    return Some(LRESULT(0));
                }
                if self.resize_end() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    return Some(LRESULT(0));
                }
                let up = self.hit(loword(lp.0), hiword(lp.0));
                let press = self.st.borrow_mut().press.take();
                unsafe {
                    let _ = ReleaseCapture();
                }
                if let Some(b) = press.filter(|b| Some(*b) == up) {
                    self.click(b);
                }
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                {
                    let mut st = self.st.borrow_mut();
                    st.press = None;
                    st.text_press = None;
                    st.selecting = false;
                }
                self.resize_end();
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = hiword(wp.0 as isize);
                let steps = {
                    let mut st = self.st.borrow_mut();
                    st.wheel += delta;
                    let s = st.wheel / 120;
                    st.wheel %= 120;
                    s
                };
                if steps != 0 {
                    if wp.0 & MK_CONTROL != 0 {
                        self.change_opacity(steps);
                    } else {
                        // lParam của WM_MOUSEWHEEL là toạ độ màn hình.
                        self.zoom(steps, None, (loword(lp.0), hiword(lp.0)));
                    }
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                let vk = wp.0 as u16;
                let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
                match vk {
                    v if v == VK_ESCAPE.0 => {
                        // Đang bấm giữ trên chữ: huỷ thao tác kéo chọn. Có vùng chọn thì Esc bỏ vùng chọn trước,
                        // lần sau mới đóng.
                        let pressing = {
                            let mut st = self.st.borrow_mut();
                            st.selecting = false;
                            st.text_press.take().is_some()
                        };
                        if pressing {
                            unsafe {
                                let _ = ReleaseCapture();
                            }
                            self.set_sel(None);
                        } else if self.selected_text().is_some() {
                            self.set_sel(None);
                        } else {
                            self.close();
                        }
                    }
                    0x43 if ctrl => self.copy(),
                    0x41 if ctrl => self.select_all(),
                    v if v == VK_OEM_PLUS.0 || v == VK_ADD.0 => self.zoom(1, None, self.center()),
                    v if v == VK_OEM_MINUS.0 || v == VK_SUBTRACT.0 => self.zoom(-1, None, self.center()),
                    v if v == 0x30 || v == VK_NUMPAD0.0 => {
                        let z = self.st.borrow().home_zoom;
                        self.zoom(0, Some(z), self.center());
                    }
                    _ => return None,
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_PILL => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_PILL);
                }
                self.st.borrow_mut().pill = None;
                self.repaint();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_SAVE => {
                self.save_box();
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                // Ảnh giữ nguyên physical px; chỉ nút, nhãn, bo góc đổi cỡ theo DPI mới. Vẽ lại sau.
                unsafe { SetTimer(Some(hwnd), TIMER_DPI, 1, None) };
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_DPI => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_DPI);
                }
                self.sync_pos();
                if !self.rescale() {
                    self.close();
                } else {
                    self.paint();
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
