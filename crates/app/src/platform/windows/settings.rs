//! Cửa sổ Cài đặt (SPEC 5.6) bằng control gốc của Windows (Common Controls v6).
//! Lưu ngay khi sửa, không có nút Save. Mọi thay đổi gửi `Action::ConfigPatch` cho App.
//!
//! Bố cục kiểu Cài đặt của Windows 11: bên trái là danh sách trang, bên phải là các thẻ (nền trắng, viền bo góc) trên
//! nền xám nhạt. Thẻ vẽ ở `WM_ERASEBKGND`; control nằm trên thẻ lấy nền trắng qua `WM_CTLCOLOR*`.
//! Cửa sổ cỡ cố định theo trang cao nhất, nên đổi trang hay hiện dòng báo lỗi không làm cửa sổ nhảy.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashSet},
    path::PathBuf,
};

use chup_core::{
    config::{AfterCapture, Config, Edge, OcrLanguage, PreviewPosition, StackMonitor},
    i18n::{fill, t, Strings, UiLanguage},
    hotkey_name::{self, Mods, Recorded},
    model::OcrViState,
    ocr::pack,
};
use serde_json::json;
use windows::{
    core::{w, HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            CreateFontIndirectW, CreatePen, CreateSolidBrush, DeleteObject, DrawFocusRect, DrawTextW, FillRect,
            GetDC, GetStockObject, GetSysColor, RedrawWindow, ReleaseDC, RoundRect, SelectObject, SetBkColor,
            SetBkMode, SetTextColor, COLOR_HIGHLIGHT, DT_CALCRECT, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
            DT_WORDBREAK, FW_SEMIBOLD, HBRUSH, HDC, HFONT, HGDIOBJ, NULL_PEN, PS_SOLID, RDW_ALLCHILDREN, RDW_ERASE,
            RDW_FRAME, RDW_INVALIDATE, TRANSPARENT,
        },
        System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER},
        UI::{
            Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_FOCUS, ODS_NOFOCUSRECT, ODS_SELECTED},
            HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow, SystemParametersInfoForDpi},
            Input::KeyboardAndMouse::{
                EnableWindow, GetFocus, GetKeyState, SetFocus, VK_BACK, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_LWIN,
                VK_MENU, VK_RWIN, VK_SHIFT,
            },
            Shell::{
                DefSubclassProc, FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
                SetWindowSubclass, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetClientRect, GetDlgCtrlID, GetWindowLongPtrW, GetWindowTextLengthW,
                GetWindowTextW, KillTimer, SendMessageW, SetForegroundWindow, SetTimer,
                SetWindowPos, SetWindowTextW, ShowWindow, BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BS_AUTOCHECKBOX,
                BS_PUSHBUTTON, CBN_SELCHANGE, CBS_DROPDOWNLIST, CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CW_USEDEFAULT,
                DLGC_WANTALLKEYS, EN_CHANGE, ES_AUTOHSCROLL, ES_READONLY, GWLP_USERDATA, GWL_EXSTYLE, GWL_STYLE, HMENU,
                LBN_SELCHANGE, LBS_HASSTRINGS, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, LBS_OWNERDRAWFIXED, LB_ADDSTRING,
                LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, LB_SETITEMHEIGHT, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, SW_SHOWNORMAL,
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CHAR, WM_CLOSE, WM_COMMAND,
                WM_CTLCOLORBTN, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND,
                WM_GETDLGCODE, WM_GETFONT, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
                WM_MEASUREITEM, WM_MOUSEMOVE, WM_NCDESTROY, WM_SETFOCUS, WM_SETFONT, WM_SETREDRAW, WM_SYSCHAR,
                WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE,
                WS_EX_CONTROLPARENT, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
            },
        },
    },
};

use super::{
    app::Action,
    ocr::EngineInfo,
    util::wide,
    win::{self, emit, hinstance, Handler},
};

/// Dữ liệu App gửi sang để hiển thị.
#[derive(Clone)]
pub struct SettingsView {
    pub config: Config,
    pub hotkey_errors: BTreeMap<String, String>,
    pub preview: String,
    pub exclusion_ok: bool,
    pub ocr_vi: OcrViState,
    /// Byte còn phải tải để OCR tiếng Việt chạy được (0 = đủ).
    pub ocr_download: u64,
    pub ocr_engine: EngineInfo,
    pub ocr_root: PathBuf,
}

/// Trang dự án (trang Giới thiệu).
const PROJECT_URL: &str = "https://github.com/duccuong2808/chup";

const ID_PICK: u16 = 101;
const ID_OPEN: u16 = 102;
const ID_PATTERN: u16 = 103;
const ID_HK_SELECT: u16 = 104;
const ID_HK_CAPTURE: u16 = 105;
const ID_COPY: u16 = 106;
const ID_AFTER: u16 = 107;
const ID_EDGE: u16 = 108;
const ID_MONITOR: u16 = 109;
const ID_AUTOSTART: u16 = 110;
const ID_HIDEUI: u16 = 111;
const ID_HK_FREEZE: u16 = 112;
const ID_PV_TOP: u16 = 113;
const ID_PV_OPACITY: u16 = 114;
const ID_PV_POS: u16 = 115;
const ID_PV_SIZE: u16 = 116;
const ID_HK_OCR: u16 = 117;
const ID_OCR_LANG: u16 = 118;
const ID_OCR_PACK: u16 = 119;
const ID_OCR_AUTO: u16 = 120;
const ID_AUTO_ON: u16 = 121;
const ID_AUTO_IMG: u16 = 122;
const ID_NAV: u16 = 123;
const ID_ABOUT_WEB: u16 = 124;
const ID_ABOUT_LOG: u16 = 125;
const ID_ABOUT_NOTICE: u16 = 126;
const ID_HK_FREEZE_ON: u16 = 127;
const ID_UI_LANG: u16 = 128;
const TIMER_PATTERN: usize = 1;


// Màu (COLORREF là 0x00BBGGRR), theo Cài đặt của Windows 11.
const BG: u32 = 0xF3F3F3;
const CARD: u32 = 0xFFFFFF;
const CARD_BORDER: u32 = 0xE5E5E5;
const NAV_SELECTED: u32 = 0xEAEAEA;
const TEXT: u32 = 0x1A1A1A;
const HINT: u32 = 0x70635B;
const ERROR: u32 = 0x2F24D1;

// Kích thước (DIP).
const NAV_X: f64 = 12.0;
const NAV_W: f64 = 200.0;
const CONTENT_X: f64 = NAV_X + NAV_W + 20.0;
const CONTENT_W: f64 = 520.0;
const RIGHT_MARGIN: f64 = 24.0;
const TOP: f64 = 16.0;
const CONTENT_TOP: f64 = 66.0;
const NAV_ITEM_H: f64 = 36.0;
const PAD_X: f64 = 16.0;
const PAD_Y: f64 = 14.0;
const ROW_GAP: f64 = 10.0;
const COMBO_W: f64 = 230.0;
const STATUS_H: f64 = 30.0;

/// Các trang, theo thứ tự trong danh sách bên trái.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    General,
    Hotkeys,
    Capture,
    Ocr,
    StackPreview,
    About,
}

/// Chữ lấy theo ngôn ngữ giao diện hiện hành.
type Text = fn(&Strings) -> &'static str;

const PAGES: [(Page, Text); 6] = [
    (Page::General, |t| t.page_general),
    (Page::Hotkeys, |t| t.page_hotkeys),
    (Page::Capture, |t| t.page_capture),
    (Page::Ocr, |t| t.page_ocr),
    (Page::StackPreview, |t| t.page_stack_preview),
    (Page::About, |t| t.page_about),
];

/// Tên ngôn ngữ trong ô chọn ngôn ngữ giao diện: viết bằng chính ngôn ngữ đó, không dịch.
const UI_LANGS: [(UiLanguage, Option<&str>); 3] =
    [(UiLanguage::System, None), (UiLanguage::Vi, Some("Tiếng Việt")), (UiLanguage::En, Some("English"))];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Title,
    Section,
    Label,
    Hint,
    Error,
    Control,
}

struct Ctl {
    hwnd: HWND,
    kind: Kind,
    /// `None`: hiện ở mọi trang (danh sách trang, tiêu đề, dòng trạng thái).
    page: Option<Page>,
    /// Chữ cố định của control (gán lại khi đổi ngôn ngữ). Chữ động (`|_| ""`) do `refresh` đặt.
    text: Text,
}

impl Ctl {
    /// Nằm trên thẻ trắng (mọi thứ của một trang, trừ tên nhóm).
    fn on_card(&self) -> bool {
        self.page.is_some() && self.kind != Kind::Section
    }
}

#[derive(Default)]
struct Gdi {
    normal: HFONT,
    bold: HFONT,
    title: HFONT,
    bg: HBRUSH,
    card: HBRUSH,
}

impl Gdi {
    fn delete(&mut self) {
        let objs = [self.normal.0, self.bold.0, self.title.0, self.bg.0, self.card.0];
        for o in objs {
            if !o.is_null() {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(o));
                }
            }
        }
        *self = Gdi::default();
    }
}

#[derive(Default)]
struct State {
    ctls: Vec<Ctl>,
    gdi: Gdi,
    view: Option<SettingsView>,
    /// Lỗi phát hiện ngay lúc ghi phím (chưa lưu vào config), ưu tiên hơn lỗi từ App.
    record_errors: BTreeMap<String, String>,
    recording: Option<&'static str>,
    seen_down: HashSet<u16>,
    status: (String, bool),
    /// Thẻ của trang đang mở (px, toạ độ client), vẽ ở `WM_ERASEBKGND`.
    cards: Vec<RECT>,
}

pub struct Settings {
    pub hwnd: HWND,
    st: RefCell<State>,
    updating: Cell<bool>,
    page: Cell<Page>,
    /// Trang của control đang tạo (`add` gắn vào control).
    building: Cell<Option<Page>>,
    // HWND của từng control, để truy cập nhanh.
    h: RefCell<Handles>,
}

#[derive(Default, Clone, Copy)]
struct Handles {
    nav: HWND,
    title: HWND,
    status: HWND,
    // Chung
    sec_save: HWND,
    lbl_dir: HWND,
    dir: HWND,
    pick: HWND,
    open: HWND,
    lbl_pattern: HWND,
    pattern: HWND,
    hint_pattern: HWND,
    sec_sys: HWND,
    autostart: HWND,
    lbl_ui_lang: HWND,
    ui_lang: HWND,
    // Phím tắt
    hint_hk: HWND,
    lbl_hk1: HWND,
    hk1: HWND,
    err1: HWND,
    lbl_hk2: HWND,
    hk2: HWND,
    err2: HWND,
    lbl_hk3: HWND,
    hk3: HWND,
    err3: HWND,
    hk3_on: HWND,
    lbl_hk4: HWND,
    hk4: HWND,
    err4: HWND,
    // Chụp ảnh
    sec_auto: HWND,
    auto_on: HWND,
    auto_img: HWND,
    hint_auto: HWND,
    sec_after: HWND,
    copy: HWND,
    lbl_after: HWND,
    hint_after: HWND,
    after: HWND,
    sec_during: HWND,
    hideui: HWND,
    note: HWND,
    // OCR
    sec_ocr: HWND,
    lbl_ocr_lang: HWND,
    ocr_lang: HWND,
    ocr_status: HWND,
    ocr_pack: HWND,
    ocr_auto: HWND,
    hint_ocr_auto: HWND,
    // Stack và Preview
    sec_stack: HWND,
    lbl_edge: HWND,
    edge: HWND,
    lbl_mon: HWND,
    mon: HWND,
    sec_pv: HWND,
    pv_top: HWND,
    lbl_pv_op: HWND,
    hint_pv_op: HWND,
    pv_op: HWND,
    lbl_pv_pos: HWND,
    pv_pos: HWND,
    lbl_pv_size: HWND,
    hint_pv_size: HWND,
    pv_size: HWND,
    // Giới thiệu
    about_name: HWND,
    about_ver: HWND,
    about_desc: HWND,
    about_web: HWND,
    about_log: HWND,
    sec_about_ocr: HWND,
    about_engine: HWND,
    about_notice: HWND,
}

thread_local! {
    static CLASS: Vec<u16> = win::register_class::<Settings>("ChupSettings");
    /// Cho vòng lặp message gọi IsDialogMessage (Tab giữa các control).
    pub static SETTINGS_HWND: Cell<isize> = const { Cell::new(0) };
    /// Trang mở lần trước (trong phiên chạy này): mở lại Cài đặt thì về đúng trang đó.
    static LAST_PAGE: Cell<usize> = const { Cell::new(0) };
}

fn hk_name(id: u16) -> &'static str {
    match id {
        ID_HK_SELECT => "selectRegion",
        ID_HK_FREEZE => "freeze",
        ID_HK_OCR => "ocr",
        _ => "capture",
    }
}

fn set_text(h: HWND, s: &str) {
    let w = wide(s);
    unsafe {
        let _ = SetWindowTextW(h, PCWSTR(w.as_ptr()));
    }
}

fn get_text(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h);
        let mut buf = vec![0u16; n as usize + 1];
        let got = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..got as usize])
    }
}

fn checked(h: HWND) -> bool {
    unsafe { SendMessageW(h, BM_GETCHECK, None, None).0 == 1 }
}

fn set_checked(h: HWND, on: bool) {
    unsafe { SendMessageW(h, BM_SETCHECK, Some(WPARAM(on as usize)), None) };
}

fn combo_sel(h: HWND) -> i32 {
    unsafe { SendMessageW(h, CB_GETCURSEL, None, None).0 as i32 }
}

fn set_combo(h: HWND, i: i32) {
    unsafe { SendMessageW(h, CB_SETCURSEL, Some(WPARAM(i as usize)), None) };
}

fn current_mods() -> Mods {
    let down = |vk: u16| unsafe { GetKeyState(vk as i32) } < 0;
    Mods {
        ctrl: down(VK_CONTROL.0),
        alt: down(VK_MENU.0),
        shift: down(VK_SHIFT.0),
        win: down(VK_LWIN.0) || down(VK_RWIN.0),
    }
}

/// "Phiên bản 0.2.0 · commit 2f451f1 · build 2026-11-04 08:15 UTC"; build tay thêm "(bản build tay)",
/// không có git thì không có commit.
fn version_text() -> String {
    let t = t();
    let mut s = fill(t.st_about_version, &[&env!("CARGO_PKG_VERSION")]);
    let commit = env!("CHUP_COMMIT");
    if !commit.is_empty() {
        s += &format!(" · commit {commit}");
    }
    if !env!("CHUP_LOCAL_BUILD").is_empty() {
        s += &format!(" ({})", t.st_about_local_build);
    }
    s + " · " + &fill(t.st_about_built, &[&env!("CHUP_BUILD_TIME")])
}

/// File giấy phép đi cùng engine đang dùng, nếu có.
fn notice_path(e: &EngineInfo) -> Option<PathBuf> {
    match e {
        EngineInfo::Installed(p) | EngineInfo::Downloaded(p) => {
            Some(p.with_file_name(pack::ENGINE_NOTICE)).filter(|n| n.is_file())
        }
        EngineInfo::Missing => None,
    }
}

/// Nguồn engine và dữ liệu OCR tiếng Việt, cho trang Giới thiệu.
fn engine_text(v: &SettingsView) -> String {
    let t = t();
    let engine = match &v.ocr_engine {
        EngineInfo::Installed(_) => t.st_about_engine_installed.to_string(),
        EngineInfo::Downloaded(_) => fill(t.st_about_engine_downloaded, &[&pack::engine().source]),
        EngineInfo::Missing => t.st_about_engine_missing.to_string(),
    };
    let d = pack::data();
    let data = if d.is_installed(&v.ocr_root) {
        fill(t.st_about_data, &[&d.source])
    } else {
        t.st_about_data_missing.to_string()
    };
    format!("{} {engine}\n{data}", t.st_about_tesseract)
}

/// Chiều cao (px) chữ của control khi xuống dòng trong bề rộng `width` px. Chữ rỗng thì 0.
fn text_height(c: HWND, width: i32) -> i32 {
    let mut text: Vec<u16> = get_text(c).encode_utf16().collect();
    if text.is_empty() {
        return 0;
    }
    unsafe {
        let dc = GetDC(Some(c));
        let font = HGDIOBJ(SendMessageW(c, WM_GETFONT, None, None).0 as *mut _);
        let old = SelectObject(dc, font);
        let mut r = RECT { left: 0, top: 0, right: width, bottom: 0 };
        DrawTextW(dc, &mut text, &mut r, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
        SelectObject(dc, old);
        ReleaseDC(Some(c), dc);
        r.bottom
    }
}

/// Xếp control của một trang từ trên xuống (DIP, nhân theo DPI khi đặt vị trí).
struct Lay {
    k: f64,
    y: f64,
    card_top: f64,
    cards: Vec<RECT>,
}

impl Lay {
    fn px(&self, v: f64) -> i32 {
        (v * self.k).round() as i32
    }

    fn mv(&self, c: HWND, x: f64, y: f64, w: f64, h: f64) {
        unsafe {
            let _ = SetWindowPos(c, None, self.px(x), self.px(y), self.px(w), self.px(h), SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// Chiều cao (DIP) của chữ trong bề rộng `w` DIP.
    fn measure(&self, c: HWND, w: f64) -> f64 {
        (text_height(c, self.px(w)) as f64 / self.k).ceil()
    }

    fn inner(&self) -> (f64, f64) {
        (CONTENT_X + PAD_X, CONTENT_W - 2.0 * PAD_X)
    }

    /// Mở một thẻ, có tên nhóm phía trên (nằm trên nền xám).
    fn card(&mut self, section: Option<HWND>) {
        if let Some(s) = section {
            self.mv(s, CONTENT_X + 2.0, self.y, CONTENT_W, 20.0);
            self.y += 26.0;
        }
        self.card_top = self.y;
        self.y += PAD_Y;
    }

    fn end_card(&mut self) {
        self.y += PAD_Y - ROW_GAP;
        let r = RECT {
            left: self.px(CONTENT_X),
            top: self.px(self.card_top),
            right: self.px(CONTENT_X + CONTENT_W),
            bottom: self.px(self.y),
        };
        self.cards.push(r);
        self.y += 22.0;
    }

    /// Nhãn (kèm gợi ý nhỏ bên dưới) bên trái, control bên phải. `drop`: combobox (chiều cao gồm danh sách xổ).
    fn row(&mut self, label: HWND, hint: Option<HWND>, ctl: HWND, ctl_w: f64, ctl_h: f64, drop: bool) {
        let (ix, iw) = self.inner();
        let lw = iw - ctl_w - 16.0;
        let hh = hint.map(|h| self.measure(h, lw)).unwrap_or(0.0);
        let text_h = 20.0 + hh;
        let row_h = text_h.max(ctl_h);
        let ty = self.y + (row_h - text_h) / 2.0;
        self.mv(label, ix, ty, lw, 20.0);
        if let Some(h) = hint {
            self.mv(h, ix, ty + 20.0, lw, hh);
        }
        let set_h = if drop { 240.0 } else { ctl_h };
        self.mv(ctl, ix + iw - ctl_w, self.y + (row_h - ctl_h) / 2.0, ctl_w, set_h);
        self.y += row_h + ROW_GAP;
    }

    /// Checkbox cả dòng, thụt vào `indent`.
    fn check(&mut self, c: HWND, indent: f64) {
        let (ix, iw) = self.inner();
        self.mv(c, ix + indent, self.y, iw - indent, 22.0);
        self.y += 22.0 + ROW_GAP;
    }

    /// Đoạn chữ cả dòng (gợi ý, báo lỗi), cao theo nội dung. Rỗng thì không chiếm chỗ.
    fn text(&mut self, c: HWND, indent: f64) {
        let (ix, iw) = self.inner();
        let h = self.measure(c, iw - indent);
        self.mv(c, ix + indent, self.y, iw - indent, h.max(1.0));
        if h > 0.0 {
            self.y += h + ROW_GAP;
        }
    }

    /// Control cả dòng, cao `h`.
    fn full(&mut self, c: HWND, h: f64) {
        let (ix, iw) = self.inner();
        self.mv(c, ix, self.y, iw, h);
        self.y += h + ROW_GAP;
    }

    /// Nhãn nhỏ ngay trên control bên dưới nó.
    fn caption(&mut self, c: HWND) {
        let (ix, iw) = self.inner();
        self.mv(c, ix, self.y, iw, 20.0);
        self.y += 22.0;
    }
}

impl Settings {
    pub fn create() -> windows::core::Result<std::rc::Rc<Settings>> {
        let me = CLASS.with(|cls| {
            win::create(
                win::CreateParams {
                    class: cls,
                    title: t().app_settings_title,
                    style: WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
                    ex_style: WS_EX_CONTROLPARENT,
                    x: CW_USEDEFAULT,
                    y: CW_USEDEFAULT,
                    w: 800,
                    h: 600,
                    parent: None,
                },
                |hwnd| Settings {
                    hwnd,
                    st: RefCell::new(State::default()),
                    updating: Cell::new(false),
                    page: Cell::new(PAGES[LAST_PAGE.with(|c| c.get()).min(PAGES.len() - 1)].0),
                    building: Cell::new(None),
                    h: RefCell::new(Handles::default()),
                },
            )
        })?;
        {
            let mut st = me.st.borrow_mut();
            st.gdi.bg = unsafe { CreateSolidBrush(COLORREF(BG)) };
            st.gdi.card = unsafe { CreateSolidBrush(COLORREF(CARD)) };
        }
        me.build();
        me.set_icon();
        SETTINGS_HWND.with(|c| c.set(me.hwnd.0 as isize));
        Ok(me)
    }

    fn add(&self, class: PCWSTR, text: Text, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE, id: u16, kind: Kind) -> HWND {
        let t = wide(text(chup_core::i18n::t()));
        let page = self.building.get();
        // Control của trang khác tạo ẩn sẵn; `layout` lo hiện/ẩn theo trang.
        let visible = if page.is_none_or(|p| p == self.page.get()) { WS_VISIBLE } else { WINDOW_STYLE(0) };
        let h = unsafe {
            CreateWindowExW(
                ex,
                class,
                PCWSTR(t.as_ptr()),
                WS_CHILD | visible | style,
                0,
                0,
                10,
                10,
                Some(self.hwnd),
                Some(HMENU(id as usize as *mut _)),
                Some(hinstance()),
                None,
            )
            .unwrap_or_default()
        };
        self.st.borrow_mut().ctls.push(Ctl { hwnd: h, kind, page, text });
        h
    }

    fn build(&self) {
        let none = WINDOW_EX_STYLE(0);
        let s = WINDOW_STYLE(0);
        let edit = WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
        let check = WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32);
        let button = WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32);
        let combo = WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32);
        let list = WS_TABSTOP
            | WINDOW_STYLE((LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT) as u32);
        let st = w!("STATIC");
        let bt = w!("BUTTON");
        let cb = w!("COMBOBOX");
        let ed = w!("EDIT");
        let mut h = Handles::default();

        // Chung cho mọi trang. Danh sách trang tạo đầu tiên: đứng đầu thứ tự Tab.
        self.building.set(None);
        h.nav = self.add(w!("LISTBOX"), |_| "", list, none, ID_NAV, Kind::Control);
        h.title = self.add(st, |_| "", s, none, 0, Kind::Title);

        self.building.set(Some(Page::General));
        h.sec_save = self.add(st, |t| t.st_sec_save, s, none, 0, Kind::Section);
        h.lbl_dir = self.add(st, |t| t.st_save_dir, s, none, 0, Kind::Label);
        h.dir = self.add(ed, |_| "", WINDOW_STYLE((ES_READONLY | ES_AUTOHSCROLL) as u32), WS_EX_CLIENTEDGE, 0, Kind::Control);
        h.pick = self.add(bt, |t| t.st_pick_folder, button, none, ID_PICK, Kind::Control);
        h.open = self.add(bt, |t| t.st_open_folder, button, none, ID_OPEN, Kind::Control);
        h.lbl_pattern = self.add(st, |t| t.st_file_name, s, none, 0, Kind::Label);
        h.pattern = self.add(ed, |_| "", edit, WS_EX_CLIENTEDGE, ID_PATTERN, Kind::Control);
        h.hint_pattern = self.add(st, |_| "", s, none, 0, Kind::Hint);
        h.sec_sys = self.add(st, |t| t.st_sec_system, s, none, 0, Kind::Section);
        h.autostart = self.add(bt, |t| t.st_autostart, check, none, ID_AUTOSTART, Kind::Control);
        h.lbl_ui_lang = self.add(st, |t| t.st_ui_language, s, none, 0, Kind::Label);
        h.ui_lang = self.add(cb, |_| "", combo, none, ID_UI_LANG, Kind::Control);

        self.building.set(Some(Page::Hotkeys));
        h.hint_hk = self.add(
            st,
            |t| t.st_hotkeys_hint,
            s,
            none,
            0,
            Kind::Hint,
        );
        h.lbl_hk1 = self.add(st, |t| t.st_hk_select, s, none, 0, Kind::Label);
        h.hk1 = self.add(ed, |_| "", edit, WS_EX_CLIENTEDGE, ID_HK_SELECT, Kind::Control);
        h.err1 = self.add(st, |_| "", s, none, 0, Kind::Error);
        h.lbl_hk2 = self.add(st, |t| t.st_hk_capture, s, none, 0, Kind::Label);
        h.hk2 = self.add(ed, |_| "", edit, WS_EX_CLIENTEDGE, ID_HK_CAPTURE, Kind::Control);
        h.err2 = self.add(st, |_| "", s, none, 0, Kind::Error);
        h.lbl_hk3 = self.add(st, |t| t.st_hk_freeze, s, none, 0, Kind::Label);
        h.hk3 = self.add(ed, |_| "", edit, WS_EX_CLIENTEDGE, ID_HK_FREEZE, Kind::Control);
        h.err3 = self.add(st, |_| "", s, none, 0, Kind::Error);
        h.hk3_on = self.add(
            bt,
            |t| t.st_hk_freeze_on,
            check,
            none,
            ID_HK_FREEZE_ON,
            Kind::Control,
        );
        h.lbl_hk4 = self.add(st, |t| t.st_hk_ocr, s, none, 0, Kind::Label);
        h.hk4 = self.add(ed, |_| "", edit, WS_EX_CLIENTEDGE, ID_HK_OCR, Kind::Control);
        h.err4 = self.add(st, |_| "", s, none, 0, Kind::Error);

        self.building.set(Some(Page::Capture));
        h.sec_auto = self.add(st, |t| t.st_sec_auto, s, none, 0, Kind::Section);
        h.auto_on = self.add(bt, |t| t.st_auto_on, check, none, ID_AUTO_ON, Kind::Control);
        h.auto_img = self.add(bt, |t| t.st_auto_img, check, none, ID_AUTO_IMG, Kind::Control);
        h.hint_auto = self.add(
            st,
            |t| t.st_auto_hint,
            s,
            none,
            0,
            Kind::Hint,
        );
        h.sec_after = self.add(st, |t| t.st_sec_after, s, none, 0, Kind::Section);
        h.copy = self.add(bt, |t| t.st_copy, check, none, ID_COPY, Kind::Control);
        h.lbl_after = self.add(st, |t| t.st_after_normal, s, none, 0, Kind::Label);
        h.hint_after = self.add(st, |t| t.st_after_hint, s, none, 0, Kind::Hint);
        h.after = self.add(cb, |_| "", combo, none, ID_AFTER, Kind::Control);
        h.sec_during = self.add(st, |t| t.st_sec_during, s, none, 0, Kind::Section);
        h.hideui = self.add(bt, |t| t.st_hide_ui, check, none, ID_HIDEUI, Kind::Control);
        h.note = self.add(st, |_| "", s, none, 0, Kind::Hint);

        self.building.set(Some(Page::Ocr));
        h.sec_ocr = self.add(st, |t| t.st_sec_ocr, s, none, 0, Kind::Section);
        h.lbl_ocr_lang = self.add(st, |t| t.st_ocr_language, s, none, 0, Kind::Label);
        h.ocr_lang = self.add(cb, |_| "", combo, none, ID_OCR_LANG, Kind::Control);
        h.ocr_status = self.add(st, |_| "", s, none, 0, Kind::Hint);
        h.ocr_pack = self.add(bt, |_| "", button, none, ID_OCR_PACK, Kind::Control);
        h.ocr_auto = self.add(bt, |t| t.st_ocr_auto, check, none, ID_OCR_AUTO, Kind::Control);
        h.hint_ocr_auto = self.add(st, |t| t.st_ocr_auto_hint, s, none, 0, Kind::Hint);

        self.building.set(Some(Page::StackPreview));
        h.sec_stack = self.add(st, |t| t.st_sec_stack, s, none, 0, Kind::Section);
        h.lbl_edge = self.add(st, |t| t.st_edge, s, none, 0, Kind::Label);
        h.edge = self.add(cb, |_| "", combo, none, ID_EDGE, Kind::Control);
        h.lbl_mon = self.add(st, |t| t.st_monitor, s, none, 0, Kind::Label);
        h.mon = self.add(cb, |_| "", combo, none, ID_MONITOR, Kind::Control);
        h.sec_pv = self.add(st, |t| t.st_sec_preview, s, none, 0, Kind::Section);
        h.pv_top = self.add(bt, |t| t.st_pv_top, check, none, ID_PV_TOP, Kind::Control);
        h.lbl_pv_op = self.add(st, |t| t.st_pv_opacity, s, none, 0, Kind::Label);
        h.hint_pv_op = self.add(st, |t| t.st_pv_opacity_hint, s, none, 0, Kind::Hint);
        h.pv_op = self.add(cb, |_| "", combo, none, ID_PV_OPACITY, Kind::Control);
        h.lbl_pv_pos = self.add(st, |t| t.st_pv_position, s, none, 0, Kind::Label);
        h.pv_pos = self.add(cb, |_| "", combo, none, ID_PV_POS, Kind::Control);
        h.lbl_pv_size = self.add(st, |t| t.st_pv_size, s, none, 0, Kind::Label);
        h.hint_pv_size = self.add(st, |t| t.st_pv_size_hint, s, none, 0, Kind::Hint);
        h.pv_size = self.add(cb, |_| "", combo, none, ID_PV_SIZE, Kind::Control);

        self.building.set(Some(Page::About));
        h.about_name = self.add(st, |_| "Chup", s, none, 0, Kind::Title);
        h.about_ver = self.add(st, |_| "", s, none, 0, Kind::Label);
        h.about_desc = self.add(
            st,
            |t| t.st_about_desc,
            s,
            none,
            0,
            Kind::Hint,
        );
        h.about_web = self.add(bt, |t| t.st_about_web, button, none, ID_ABOUT_WEB, Kind::Control);
        h.about_log = self.add(bt, |t| t.st_about_log, button, none, ID_ABOUT_LOG, Kind::Control);
        h.sec_about_ocr = self.add(st, |t| t.st_about_sec_ocr, s, none, 0, Kind::Section);
        h.about_engine = self.add(st, |_| "", s, none, 0, Kind::Hint);
        h.about_notice =
            self.add(bt, |t| t.st_about_notice, button, none, ID_ABOUT_NOTICE, Kind::Control);

        // Dòng trạng thái (lỗi chọn folder…) ở đáy, chung cho mọi trang.
        self.building.set(None);
        h.status = self.add(st, |_| "", s, none, 0, Kind::Hint);

        for hk in [h.hk1, h.hk2, h.hk3, h.hk4] {
            unsafe {
                let _ = SetWindowSubclass(hk, Some(hotkey_proc), 1, self.hwnd.0 as usize);
            }
        }
        *self.h.borrow_mut() = h;
        self.fill_lists();
        self.set_page_title();
        self.apply_dpi();
    }

    /// Mục của danh sách trang và các combobox (theo ngôn ngữ hiện hành). Chọn mục nào thì `refresh` đặt.
    fn fill_lists(&self) {
        let h = *self.h.borrow();
        let t = t();
        let page_idx = PAGES.iter().position(|(p, _)| *p == self.page.get()).unwrap_or(0);
        let pages: Vec<String> = PAGES.iter().map(|(_, name)| name(t).to_string()).collect();
        let pct = |n: u32| fill(t.st_pv_size_max, &[&n]);
        let ui_langs: Vec<String> =
            UI_LANGS.iter().map(|(_, name)| name.unwrap_or(t.st_ui_language_system).to_string()).collect();
        let lists: [(HWND, bool, Vec<String>); 9] = [
            (h.nav, true, pages),
            (h.after, false, vec![t.st_after_close.into(), t.st_after_keep.into()]),
            (h.edge, false, vec![t.st_edge_right.into(), t.st_edge_left.into()]),
            (h.ocr_lang, false, vec![t.st_ocr_en.into(), t.st_ocr_vi.into()]),
            (h.mon, false, vec![t.st_monitor_primary.into(), t.st_monitor_cursor.into()]),
            (h.pv_op, false, (2..=10).rev().map(|n| format!("{}%", n * 10)).collect()),
            (h.pv_pos, false, vec![t.st_pv_pos_last.into(), t.st_pv_pos_stack.into(), t.st_pv_pos_center.into()]),
            (h.pv_size, false, vec![pct(30), pct(50), pct(70), pct(90)]),
            (h.ui_lang, false, ui_langs),
        ];
        for (c, is_list, items) in lists {
            let (reset, add) = if is_list { (LB_RESETCONTENT, LB_ADDSTRING) } else { (CB_RESETCONTENT, CB_ADDSTRING) };
            unsafe { SendMessageW(c, reset, None, None) };
            for it in items {
                let w = wide(&it);
                unsafe { SendMessageW(c, add, None, Some(LPARAM(w.as_ptr() as isize))) };
            }
        }
        unsafe { SendMessageW(h.nav, LB_SETCURSEL, Some(WPARAM(page_idx)), None) };
    }

    /// Ngôn ngữ giao diện đổi: gán lại chữ cố định của mọi control, danh sách, tiêu đề. App gọi `refresh` ngay sau
    /// để đặt lại chữ động, mục đang chọn và sắp xếp lại (chữ dài ngắn khác nhau).
    pub fn retext(&self) {
        self.updating.set(true);
        let ctls: Vec<(HWND, Text)> = self.st.borrow().ctls.iter().map(|c| (c.hwnd, c.text)).collect();
        for (c, text) in ctls {
            set_text(c, text(t()));
        }
        set_text(self.hwnd, t().app_settings_title);
        self.fill_lists();
        self.set_page_title();
        self.updating.set(false);
    }

    fn set_icon(&self) {
        super::util::set_app_icon(self.hwnd);
    }

    fn dpi(&self) -> u32 {
        unsafe { GetDpiForWindow(self.hwnd) }.max(96)
    }

    fn scale(&self) -> f64 {
        self.dpi() as f64 / 96.0
    }

    /// Tạo lại font theo DPI rồi sắp xếp lại.
    fn apply_dpi(&self) {
        let dpi = self.dpi();
        let mut ncm = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        unsafe {
            let _ = SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                ncm.cbSize,
                Some(&mut ncm as *mut _ as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0).0,
                dpi,
            );
        }
        let mut lf = ncm.lfMessageFont;
        let normal = unsafe { CreateFontIndirectW(&lf) };
        lf.lfWeight = FW_SEMIBOLD.0 as i32;
        let bold = unsafe { CreateFontIndirectW(&lf) };
        lf.lfHeight = -((26 * dpi / 96) as i32);
        let title = unsafe { CreateFontIndirectW(&lf) };
        let nav = self.h.borrow().nav;
        // Thả borrow trước khi gửi WM_SETFONT: control có thể vẽ lại ngay và xin nền của cha (WM_ERASEBKGND,
        // WM_CTLCOLOR* đọc `st`). Font cũ xoá sau khi mọi control đã nhận font mới.
        let (old, fonts) = {
            let mut st = self.st.borrow_mut();
            let old = [st.gdi.normal, st.gdi.bold, st.gdi.title];
            st.gdi.normal = normal;
            st.gdi.bold = bold;
            st.gdi.title = title;
            let fonts: Vec<(HWND, HFONT)> = st
                .ctls
                .iter()
                .map(|c| {
                    let f = match c.kind {
                        Kind::Title => title,
                        Kind::Section => bold,
                        _ => normal,
                    };
                    (c.hwnd, f)
                })
                .collect();
            (old, fonts)
        };
        for (c, f) in fonts {
            unsafe { SendMessageW(c, WM_SETFONT, Some(WPARAM(f.0 as usize)), Some(LPARAM(1))) };
        }
        for f in old {
            if !f.is_invalid() {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(f.0));
                }
            }
        }
        let item_h = (NAV_ITEM_H * dpi as f64 / 96.0).round() as isize;
        unsafe { SendMessageW(nav, LB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(item_h))) };
        self.layout();
    }

    fn set_page_title(&self) {
        let name = PAGES.iter().find(|(p, _)| *p == self.page.get()).map(|(_, n)| n(t())).unwrap_or_default();
        set_text(self.h.borrow().title, name);
    }

    /// Đổi trang: ẩn control trang cũ, hiện trang mới, vẽ lại một lần (không nháy từng control).
    fn show_page(&self, idx: usize) {
        let Some(&(page, _)) = PAGES.get(idx) else { return };
        if page == self.page.get() {
            return;
        }
        LAST_PAGE.with(|c| c.set(idx));
        self.page.set(page);
        unsafe { SendMessageW(self.hwnd, WM_SETREDRAW, Some(WPARAM(0)), None) };
        self.set_page_title();
        self.layout();
        unsafe {
            SendMessageW(self.hwnd, WM_SETREDRAW, Some(WPARAM(1)), None);
            let _ = RedrawWindow(Some(self.hwnd), None, None, RDW_ERASE | RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN);
        }
    }

    /// Xếp control của mọi trang (DIP, nhân theo DPI), hiện trang đang mở, đặt cỡ cửa sổ theo trang cao nhất.
    fn layout(&self) {
        let k = self.scale();
        let h = *self.h.borrow();
        let current = self.page.get();
        let mut max_bottom: f64 = 0.0;
        let mut cards = Vec::new();
        // Dòng báo lỗi phím tắt chỉ hiện khi có lỗi.
        let has = |c: HWND| !get_text(c).is_empty();
        let errs = [(h.err1, has(h.err1)), (h.err2, has(h.err2)), (h.err3, has(h.err3)), (h.err4, has(h.err4))];

        for (page, _) in PAGES {
            let mut l = Lay { k, y: CONTENT_TOP, card_top: CONTENT_TOP, cards: Vec::new() };
            // Chỗ dành cho dòng báo lỗi đang ẩn, để cỡ cửa sổ đủ cho cả lúc chúng hiện.
            let mut reserve = 0.0;
            match page {
                Page::General => {
                    l.card(Some(h.sec_save));
                    l.caption(h.lbl_dir);
                    l.full(h.dir, 26.0);
                    let (ix, _) = l.inner();
                    l.mv(h.pick, ix, l.y, 124.0, 28.0);
                    l.mv(h.open, ix + 132.0, l.y, 100.0, 28.0);
                    l.y += 28.0 + ROW_GAP + 6.0;
                    l.caption(h.lbl_pattern);
                    l.full(h.pattern, 26.0);
                    l.text(h.hint_pattern, 0.0);
                    l.end_card();
                    l.card(Some(h.sec_sys));
                    l.row(h.lbl_ui_lang, None, h.ui_lang, COMBO_W, 24.0, true);
                    l.check(h.autostart, 0.0);
                    l.end_card();
                }
                Page::Hotkeys => {
                    l.card(None);
                    l.text(h.hint_hk, 0.0);
                    l.y += 4.0;
                    for ((lbl, edit), (err, show)) in
                        [(h.lbl_hk1, h.hk1), (h.lbl_hk2, h.hk2), (h.lbl_hk3, h.hk3), (h.lbl_hk4, h.hk4)]
                            .into_iter()
                            .zip(errs)
                    {
                        l.row(lbl, None, edit, COMBO_W, 26.0, false);
                        if show {
                            l.text(err, 0.0);
                        } else {
                            reserve += 46.0;
                        }
                        if edit == h.hk3 {
                            l.check(h.hk3_on, 20.0);
                        }
                    }
                    l.end_card();
                }
                Page::Capture => {
                    l.card(Some(h.sec_auto));
                    l.check(h.auto_on, 0.0);
                    l.check(h.auto_img, 20.0);
                    l.text(h.hint_auto, 0.0);
                    l.end_card();
                    l.card(Some(h.sec_after));
                    l.check(h.copy, 0.0);
                    l.row(h.lbl_after, Some(h.hint_after), h.after, COMBO_W, 24.0, true);
                    l.end_card();
                    l.card(Some(h.sec_during));
                    l.check(h.hideui, 0.0);
                    l.text(h.note, 20.0);
                    l.end_card();
                }
                Page::Ocr => {
                    l.card(Some(h.sec_ocr));
                    l.row(h.lbl_ocr_lang, None, h.ocr_lang, 290.0, 24.0, true);
                    let (ix, iw) = l.inner();
                    let bw = 150.0;
                    let sh = l.measure(h.ocr_status, iw - bw - 16.0).max(20.0);
                    let rh = sh.max(28.0);
                    l.mv(h.ocr_status, ix, l.y + (rh - sh) / 2.0, iw - bw - 16.0, sh);
                    l.mv(h.ocr_pack, ix + iw - bw, l.y + (rh - 28.0) / 2.0, bw, 28.0);
                    l.y += rh + ROW_GAP + 6.0;
                    l.check(h.ocr_auto, 0.0);
                    l.text(h.hint_ocr_auto, 20.0);
                    l.end_card();
                }
                Page::StackPreview => {
                    l.card(Some(h.sec_stack));
                    l.row(h.lbl_edge, None, h.edge, COMBO_W, 24.0, true);
                    l.row(h.lbl_mon, None, h.mon, COMBO_W, 24.0, true);
                    l.end_card();
                    l.card(Some(h.sec_pv));
                    l.check(h.pv_top, 0.0);
                    l.row(h.lbl_pv_op, Some(h.hint_pv_op), h.pv_op, COMBO_W, 24.0, true);
                    l.row(h.lbl_pv_pos, None, h.pv_pos, COMBO_W, 24.0, true);
                    l.row(h.lbl_pv_size, Some(h.hint_pv_size), h.pv_size, COMBO_W, 24.0, true);
                    l.end_card();
                }
                Page::About => {
                    l.card(None);
                    l.full(h.about_name, 36.0);
                    l.caption(h.about_ver);
                    l.y += 2.0;
                    l.text(h.about_desc, 0.0);
                    let (ix, _) = l.inner();
                    l.mv(h.about_web, ix, l.y, 124.0, 28.0);
                    l.mv(h.about_log, ix + 132.0, l.y, 140.0, 28.0);
                    l.y += 28.0 + ROW_GAP;
                    l.end_card();
                    l.card(Some(h.sec_about_ocr));
                    l.text(h.about_engine, 0.0);
                    let (ix, _) = l.inner();
                    l.mv(h.about_notice, ix, l.y, 230.0, 28.0);
                    l.y += 28.0 + ROW_GAP;
                    l.end_card();
                }
            }
            max_bottom = max_bottom.max(l.y + reserve);
            if page == current {
                cards = l.cards;
            }
        }

        // Hiện control của trang đang mở (dòng lỗi phím tắt thì chỉ khi có lỗi).
        let ctls: Vec<(HWND, Option<Page>)> = self.st.borrow().ctls.iter().map(|c| (c.hwnd, c.page)).collect();
        for (c, page) in ctls {
            let mut show = page.is_none_or(|p| p == current);
            if let Some((_, has)) = errs.iter().find(|(e, _)| *e == c) {
                show &= *has;
            }
            // Đọc cờ WS_VISIBLE của chính control: lúc cha đang tắt vẽ (WM_SETREDRAW), IsWindowVisible luôn trả false.
            let visible = unsafe { GetWindowLongPtrW(c, GWL_STYLE) } as u32 & WS_VISIBLE.0 != 0;
            if show != visible {
                // Control đang giữ focus sắp bị ẩn: trả focus cho danh sách trang (không để phím gõ vào control ẩn).
                if !show && unsafe { GetFocus() } == c {
                    unsafe {
                        let _ = SetFocus(Some(h.nav));
                    }
                }
                unsafe {
                    let _ = ShowWindow(c, if show { SW_SHOW } else { SW_HIDE });
                }
            }
        }
        self.st.borrow_mut().cards = cards;

        // Cố định: danh sách trang, tiêu đề, dòng trạng thái, cỡ cửa sổ.
        let content_h = max_bottom - 22.0 + STATUS_H;
        let p = |v: f64| (v * k).round() as i32;
        let mv = |c: HWND, x: f64, y: f64, w: f64, hh: f64| unsafe {
            let _ = SetWindowPos(c, None, p(x), p(y), p(w), p(hh), SWP_NOZORDER | SWP_NOACTIVATE);
        };
        mv(h.nav, NAV_X, TOP, NAV_W, content_h - TOP);
        mv(h.title, CONTENT_X, TOP - 4.0, CONTENT_W, 40.0);
        mv(h.status, CONTENT_X + 2.0, content_h - STATUS_H + 12.0, CONTENT_W, 20.0);

        let (cw, ch) = (p(CONTENT_X + CONTENT_W + RIGHT_MARGIN), p(content_h + 12.0));
        let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
        unsafe {
            let style = WINDOW_STYLE(GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32);
            let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32);
            let _ = AdjustWindowRectExForDpi(&mut r, style, false, ex, self.dpi());
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE,
            );
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(self.hwnd);
            // Focus vào danh sách trang, không rơi vào ô ghi phím tắt (sẽ bắt đầu ghi phím).
            let _ = SetFocus(Some(self.h.borrow().nav));
        }
    }

    /// Hiển thị lại theo state mới từ App.
    pub fn refresh(&self, v: SettingsView) {
        self.updating.set(true);
        let h = *self.h.borrow();
        let c = &v.config;
        let t = t();
        set_text(h.dir, &c.save_dir.display().to_string());
        if unsafe { GetFocus() } != h.pattern {
            set_text(h.pattern, &c.file_name_pattern);
        }
        set_text(h.hint_pattern, &fill(t.st_file_name_hint, &[&v.preview]));
        set_checked(h.copy, c.copy_to_clipboard);
        set_checked(h.hk3_on, c.hotkeys.freeze_enabled);
        set_combo(h.ui_lang, UI_LANGS.iter().position(|(l, _)| *l == c.ui_language).unwrap_or(0) as i32);
        set_text(h.about_ver, &version_text());
        unsafe {
            let _ = EnableWindow(h.hk3_on, !c.hotkeys.freeze.is_empty());
        }
        set_combo(h.after, if c.after_capture_normal == AfterCapture::Keep { 1 } else { 0 });
        set_combo(h.edge, if c.stack.edge == Edge::Left { 1 } else { 0 });
        set_combo(h.mon, if c.stack.monitor == StackMonitor::Cursor { 1 } else { 0 });
        set_checked(h.pv_top, c.preview.always_on_top);
        set_combo(h.pv_op, (100 - c.preview.opacity.min(100) as i32) / 10);
        let pos = match c.preview.position {
            PreviewPosition::Remember => 0,
            PreviewPosition::Stack => 1,
            PreviewPosition::Center => 2,
        };
        set_combo(h.pv_pos, pos);
        // 30/50/70/90: giá trị khác (sửa tay trong file) lấy mục gần nhất.
        set_combo(h.pv_size, ((c.preview.size as i32 - 20) / 20).clamp(0, 3));
        set_combo(h.ocr_lang, if c.ocr.language == OcrLanguage::Vi { 1 } else { 0 });
        set_checked(h.ocr_auto, c.ocr.auto_preview);
        set_checked(h.auto_on, c.auto_detect.enabled);
        set_checked(h.auto_img, c.auto_detect.image_edges);
        unsafe {
            let _ = EnableWindow(h.auto_img, c.auto_detect.enabled);
        }
        let engine_missing = matches!(v.ocr_engine, EngineInfo::Missing);
        let downloaded = matches!(v.ocr_engine, EngineInfo::Downloaded(_)) || pack::data().is_installed(&v.ocr_root);
        let (status, button, enabled) = match v.ocr_vi {
            OcrViState::Ready => (t.st_ocr_ready.to_string(), t.st_ocr_remove.to_string(), downloaded),
            OcrViState::NoPack => (
                if engine_missing { t.st_ocr_missing_all } else { t.st_ocr_missing_data }.to_string(),
                fill(t.st_ocr_download, &[&pack::size_label(v.ocr_download)]),
                true,
            ),
        };
        set_text(h.ocr_status, &status);
        set_text(h.ocr_pack, &button);
        unsafe {
            let _ = EnableWindow(h.ocr_pack, enabled);
        }
        set_text(h.about_engine, &engine_text(&v));
        unsafe {
            let _ = EnableWindow(h.about_notice, notice_path(&v.ocr_engine).is_some());
        }
        set_checked(h.autostart, c.launch_at_startup);
        set_checked(h.hideui, c.hide_ui_during_capture || !v.exclusion_ok);
        unsafe {
            let _ = EnableWindow(h.hideui, v.exclusion_ok);
        }
        set_text(
            h.note,
            if v.exclusion_ok { t.st_hide_ui_note_ok } else { t.st_hide_ui_note_old },
        );
        self.st.borrow_mut().view = Some(v);
        self.render_hotkeys();
        self.updating.set(false);
    }

    fn render_hotkeys(&self) {
        let h = *self.h.borrow();
        let (recording, view, rec_err) = {
            let st = self.st.borrow();
            (st.recording, st.view.clone(), st.record_errors.clone())
        };
        let Some(v) = view else { return };
        for (name, edit, err, val) in [
            ("selectRegion", h.hk1, h.err1, &v.config.hotkeys.select_region),
            ("capture", h.hk2, h.err2, &v.config.hotkeys.capture),
            ("freeze", h.hk3, h.err3, &v.config.hotkeys.freeze),
            ("ocr", h.hk4, h.err4, &v.config.hotkeys.ocr),
        ] {
            if recording != Some(name) {
                set_text(edit, if val.is_empty() { t().unused_hotkey } else { val });
            }
            let msg = rec_err.get(name).or_else(|| v.hotkey_errors.get(name)).cloned().unwrap_or_default();
            set_text(err, &msg);
        }
        self.layout();
    }

    pub fn set_status(&self, text: &str, error: bool) {
        self.st.borrow_mut().status = (text.to_string(), error);
        set_text(self.h.borrow().status, text);
    }

    fn patch(&self, p: serde_json::Value) {
        if !self.updating.get() {
            emit(Action::ConfigPatch(p));
        }
    }

    fn pick_folder(&self) {
        let current = self.st.borrow().view.as_ref().map(|v| v.config.save_dir.clone());
        match pick_folder_dialog(self.hwnd, current) {
            Ok(Some(dir)) => self.patch(json!({ "saveDir": dir })),
            Ok(None) => {}
            Err(e) => self.set_status(&e, true),
        }
    }

    // ---------- ghi phím tắt ----------

    fn start_recording(&self, name: &'static str, edit: HWND) {
        {
            let mut st = self.st.borrow_mut();
            st.recording = Some(name);
            st.seen_down.clear();
            st.record_errors.remove(name);
        }
        emit(Action::PauseHotkeys);
        set_text(edit, t().st_recording);
        self.render_hotkeys();
    }

    fn stop_recording(&self) {
        let was = self.st.borrow_mut().recording.take();
        if was.is_some() {
            emit(Action::ResumeHotkeys);
            self.render_hotkeys();
        }
    }

    /// Trả về `true` nếu đã xử lý phím (không cho control mặc định xử lý).
    fn on_key(&self, edit: HWND, name: &'static str, vk: u16, down: bool) -> bool {
        if down {
            self.st.borrow_mut().seen_down.insert(vk);
            let mods = current_mods();
            if vk == VK_ESCAPE.0 {
                unsafe {
                    let _ = SetFocus(Some(self.hwnd));
                }
                return true;
            }
            if (vk == VK_DELETE.0 || vk == VK_BACK.0) && !mods.ctrl && !mods.alt && !mods.win {
                self.st.borrow_mut().record_errors.remove(name);
                self.patch(json!({ "hotkeys": { name: "" } }));
                unsafe {
                    let _ = SetFocus(Some(self.hwnd));
                }
                return true;
            }
            match hotkey_name::record(vk, mods) {
                Recorded::Accelerator(a) => {
                    set_text(edit, &a);
                    self.st.borrow_mut().record_errors.remove(name);
                    self.patch(json!({ "hotkeys": { name: a } }));
                    unsafe {
                        let _ = SetFocus(Some(self.hwnd));
                    }
                }
                Recorded::Incomplete(t) => set_text(edit, &t),
                Recorded::Ignored => {}
            }
            return true;
        }
        // Tổ hợp đã bị app khác đăng ký thì Windows nuốt keydown của phím chính nhưng keyup vẫn
        // tới (đã kiểm chứng trên Windows 11). Thấy keyup mà chưa thấy keydown => tổ hợp đã có chủ.
        let seen = self.st.borrow().seen_down.contains(&vk);
        if !seen && !hotkey_name::is_modifier_vk(vk) {
            if let Recorded::Accelerator(a) = hotkey_name::record(vk, current_mods()) {
                self.st.borrow_mut().record_errors.insert(
                    name.to_string(),
                    fill(t().hotkey_taken_recording, &[&a]),
                );
                set_text(edit, t().st_recording);
                self.render_hotkeys();
            }
        }
        true
    }
}

impl Drop for Settings {
    fn drop(&mut self) {
        self.st.get_mut().gdi.delete();
    }
}

impl Settings {
    /// Nền xám + các thẻ của trang đang mở. Cũng chạy khi control con (nút, checkbox) xin vẽ nền của cha
    /// (DrawThemeParentBackground gửi WM_ERASEBKGND với DC đã dời gốc), nên luôn vẽ theo toạ độ client.
    fn paint_background(&self, dc: HDC) {
        let st = self.st.borrow();
        let k = self.scale();
        let radius = (8.0 * k).round() as i32;
        let mut r = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut r);
            FillRect(dc, &r, st.gdi.bg);
            let pen = CreatePen(PS_SOLID, 1, COLORREF(CARD_BORDER));
            let old_pen = SelectObject(dc, HGDIOBJ(pen.0));
            let old_brush = SelectObject(dc, HGDIOBJ(st.gdi.card.0));
            for c in &st.cards {
                let _ = RoundRect(dc, c.left, c.top, c.right, c.bottom, radius, radius);
            }
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
    }

    /// Một mục của danh sách trang: mục đang chọn có nền đậm hơn và vạch màu nhấn bên trái.
    fn draw_nav_item(&self, di: &DRAWITEMSTRUCT) {
        let Some((_, name)) = PAGES.get(di.itemID as usize) else { return };
        let k = self.scale();
        let p = |v: f64| (v * k).round() as i32;
        let dc = di.hDC;
        let (bg, font) = {
            let st = self.st.borrow();
            (st.gdi.bg, st.gdi.normal)
        };
        unsafe {
            FillRect(dc, &di.rcItem, bg);
            let mut r = di.rcItem;
            r.left += p(2.0);
            r.right -= p(2.0);
            r.top += p(2.0);
            r.bottom -= p(2.0);
            if di.itemState.0 & ODS_SELECTED.0 != 0 {
                let fill = CreateSolidBrush(COLORREF(NAV_SELECTED));
                let accent = CreateSolidBrush(COLORREF(GetSysColor(COLOR_HIGHLIGHT)));
                let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
                let old_brush = SelectObject(dc, HGDIOBJ(fill.0));
                let _ = RoundRect(dc, r.left, r.top, r.right + 1, r.bottom + 1, p(8.0), p(8.0));
                // Vạch nhấn 3 × 16 DIP, giữa theo chiều dọc.
                let cy = (r.top + r.bottom) / 2;
                SelectObject(dc, HGDIOBJ(accent.0));
                let _ = RoundRect(dc, r.left, cy - p(8.0), r.left + p(3.0) + 1, cy + p(8.0) + 1, p(3.0), p(3.0));
                SelectObject(dc, old_brush);
                SelectObject(dc, old_pen);
                let _ = DeleteObject(HGDIOBJ(fill.0));
                let _ = DeleteObject(HGDIOBJ(accent.0));
            }
            let old_font = SelectObject(dc, HGDIOBJ(font.0));
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(TEXT));
            let mut tr = r;
            tr.left += p(14.0);
            let mut text: Vec<u16> = name(t()).encode_utf16().collect();
            DrawTextW(dc, &mut text, &mut tr, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(dc, old_font);
            if di.itemState.0 & ODS_FOCUS.0 != 0 && di.itemState.0 & ODS_NOFOCUSRECT.0 == 0 {
                let _ = DrawFocusRect(dc, &r);
            }
        }
    }

    /// Màu chữ / nền cho static, checkbox, ô chỉ đọc: nền trắng nếu nằm trên thẻ, xám nếu nằm trên nền.
    fn ctl_color(&self, dc: HDC, ctl: HWND) -> LRESULT {
        let st = self.st.borrow();
        let c = st.ctls.iter().find(|c| c.hwnd == ctl);
        let on_card = c.is_some_and(Ctl::on_card);
        let error_status = ctl == self.h.borrow().status && st.status.1;
        let color = match c.map(|c| c.kind) {
            Some(Kind::Error) => ERROR,
            _ if error_status => ERROR,
            Some(Kind::Hint) => HINT,
            _ => TEXT,
        };
        let (bg, brush) = if on_card { (CARD, st.gdi.card) } else { (BG, st.gdi.bg) };
        unsafe {
            SetTextColor(dc, COLORREF(color));
            SetBkColor(dc, COLORREF(bg));
        }
        LRESULT(brush.0 as isize)
    }
}

unsafe extern "system" fn hotkey_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    parent: usize,
) -> LRESULT {
    let parent = HWND(parent as *mut _);
    let ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const Settings;
    if ptr.is_null() {
        return DefSubclassProc(hwnd, msg, wp, lp);
    }
    let me = &*ptr;
    let id = GetDlgCtrlID(hwnd) as u16;
    let name = hk_name(id);
    match msg {
        WM_GETDLGCODE => LRESULT(DLGC_WANTALLKEYS as isize),
        WM_SETFOCUS => {
            me.start_recording(name, hwnd);
            DefSubclassProc(hwnd, msg, wp, lp)
        }
        WM_KILLFOCUS => {
            me.stop_recording();
            DefSubclassProc(hwnd, msg, wp, lp)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            me.on_key(hwnd, name, wp.0 as u16, true);
            LRESULT(0)
        }
        WM_KEYUP | WM_SYSKEYUP => {
            me.on_key(hwnd, name, wp.0 as u16, false);
            LRESULT(0)
        }
        WM_CHAR | WM_SYSCHAR => LRESULT(0),
        // Ô ghi phím không cho bôi đen chữ (click, kéo, hay Tab tới đều không chọn).
        windows::Win32::UI::Controls::EM_SETSEL => LRESULT(0),
        WM_LBUTTONDOWN
        | WM_LBUTTONDBLCLK => {
            let _ = SetFocus(Some(hwnd));
            LRESULT(0)
        }
        WM_MOUSEMOVE => LRESULT(0),
        _ => DefSubclassProc(hwnd, msg, wp, lp),
    }
}

/// Hộp chọn folder gốc của Windows. `Ok(None)` = người dùng huỷ.
fn pick_folder_dialog(owner: HWND, current: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    unsafe {
        let dlg: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let opts = dlg.GetOptions().map_err(|e| e.to_string())?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).map_err(|e| e.to_string())?;
        let _ = dlg.SetTitle(&HSTRING::from(t().st_pick_folder_title));
        if let Some(dir) = current.filter(|d| d.is_dir()) {
            let p = HSTRING::from(dir.as_os_str());
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&p, None) {
                let _ = dlg.SetFolder(&item);
            }
        }
        if dlg.Show(Some(owner)).is_err() {
            return Ok(None);
        }
        let item = dlg.GetResult().map_err(|e| e.to_string())?;
        let name: PWSTR = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|e| e.to_string())?;
        let s = name.to_string().map_err(|e| e.to_string())?;
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(Some(PathBuf::from(s)))
    }
}

impl Handler for Settings {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_COMMAND => {
                let id = (wp.0 & 0xFFFF) as u16;
                let code = ((wp.0 >> 16) & 0xFFFF) as u32;
                let h = *self.h.borrow();
                match (id, code) {
                    (ID_NAV, LBN_SELCHANGE) => {
                        let i = unsafe { SendMessageW(h.nav, LB_GETCURSEL, None, None).0 };
                        if i >= 0 {
                            self.show_page(i as usize);
                        }
                    }
                    (ID_PICK, BN_CLICKED) => self.pick_folder(),
                    (ID_OPEN, BN_CLICKED) => emit(Action::OpenSaveDir),
                    (ID_ABOUT_WEB, BN_CLICKED) => emit(Action::Open(PathBuf::from(PROJECT_URL))),
                    (ID_ABOUT_LOG | ID_ABOUT_NOTICE, BN_CLICKED) => {
                        let target = self.st.borrow().view.as_ref().and_then(|v| {
                            if id == ID_ABOUT_LOG {
                                v.ocr_root.parent().map(PathBuf::from)
                            } else {
                                notice_path(&v.ocr_engine)
                            }
                        });
                        if let Some(t) = target {
                            emit(Action::Open(t));
                        }
                    }
                    (ID_PATTERN, EN_CHANGE) if !self.updating.get() => unsafe {
                        SetTimer(Some(hwnd), TIMER_PATTERN, 400, None);
                    },
                    (ID_COPY, BN_CLICKED) => self.patch(json!({ "copyToClipboard": checked(h.copy) })),
                    (ID_UI_LANG, CBN_SELCHANGE) => {
                        let i = combo_sel(h.ui_lang).clamp(0, UI_LANGS.len() as i32 - 1) as usize;
                        self.patch(json!({ "uiLanguage": UI_LANGS[i].0 }))
                    }
                    (ID_HK_FREEZE_ON, BN_CLICKED) => {
                        self.patch(json!({ "hotkeys": { "freezeEnabled": checked(h.hk3_on) } }))
                    }
                    (ID_AUTOSTART, BN_CLICKED) => self.patch(json!({ "launchAtStartup": checked(h.autostart) })),
                    (ID_PV_TOP, BN_CLICKED) => self.patch(json!({ "preview": { "alwaysOnTop": checked(h.pv_top) } })),
                    (ID_PV_POS, CBN_SELCHANGE) => {
                        let v = ["remember", "stack", "center"][combo_sel(h.pv_pos).clamp(0, 2) as usize];
                        self.patch(json!({ "preview": { "position": v } }))
                    }
                    (ID_PV_SIZE, CBN_SELCHANGE) => {
                        let v = 30 + 20 * combo_sel(h.pv_size).clamp(0, 3);
                        self.patch(json!({ "preview": { "size": v } }))
                    }
                    (ID_PV_OPACITY, CBN_SELCHANGE) => {
                        let v = 100 - 10 * combo_sel(h.pv_op).clamp(0, 8);
                        self.patch(json!({ "preview": { "opacity": v } }))
                    }
                    (ID_OCR_LANG, CBN_SELCHANGE) => {
                        let v = if combo_sel(h.ocr_lang) == 1 { "vi" } else { "en" };
                        self.patch(json!({ "ocr": { "language": v } }))
                    }
                    (ID_OCR_PACK, BN_CLICKED) => {
                        let ready = self.st.borrow().view.as_ref().is_some_and(|v| v.ocr_vi == OcrViState::Ready);
                        emit(if ready { Action::OcrRemove } else { Action::OcrDownload });
                    }
                    (ID_OCR_AUTO, BN_CLICKED) => self.patch(json!({ "ocr": { "autoPreview": checked(h.ocr_auto) } })),
                    (ID_AUTO_ON, BN_CLICKED) => self.patch(json!({ "autoDetect": { "enabled": checked(h.auto_on) } })),
                    (ID_AUTO_IMG, BN_CLICKED) => {
                        self.patch(json!({ "autoDetect": { "imageEdges": checked(h.auto_img) } }))
                    }
                    (ID_HIDEUI, BN_CLICKED) => self.patch(json!({ "hideUiDuringCapture": checked(h.hideui) })),
                    (ID_AFTER, CBN_SELCHANGE) => {
                        let v = if combo_sel(h.after) == 1 { "keep" } else { "close" };
                        self.patch(json!({ "afterCaptureNormal": v }))
                    }
                    (ID_EDGE, CBN_SELCHANGE) => {
                        let v = if combo_sel(h.edge) == 1 { "left" } else { "right" };
                        self.patch(json!({ "stack": { "edge": v } }))
                    }
                    (ID_MONITOR, CBN_SELCHANGE) => {
                        let v = if combo_sel(h.mon) == 1 { "cursor" } else { "primary" };
                        self.patch(json!({ "stack": { "monitor": v } }))
                    }
                    _ => return None,
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_PATTERN => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_PATTERN);
                }
                let t = get_text(self.h.borrow().pattern);
                if !t.trim().is_empty() {
                    self.patch(json!({ "fileNamePattern": t }));
                }
                Some(LRESULT(0))
            }
            WM_MEASUREITEM => {
                // Danh sách trang (LBS_OWNERDRAWFIXED) hỏi một lần lúc tạo; đổi DPI thì LB_SETITEMHEIGHT.
                let mi = unsafe { &mut *(lp.0 as *mut MEASUREITEMSTRUCT) };
                if mi.CtlID == ID_NAV as u32 {
                    mi.itemHeight = (NAV_ITEM_H * self.scale()).round() as u32;
                    return Some(LRESULT(1));
                }
                None
            }
            WM_DRAWITEM => {
                let di = unsafe { &*(lp.0 as *const DRAWITEMSTRUCT) };
                if di.CtlID == ID_NAV as u32 {
                    self.draw_nav_item(di);
                    return Some(LRESULT(1));
                }
                None
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => Some(self.ctl_color(HDC(wp.0 as *mut _), HWND(lp.0 as *mut _))),
            // Chỉ danh sách trang lấy nền xám; danh sách xổ của combobox cũng gửi message này (qua combobox), để mặc định.
            WM_CTLCOLORLISTBOX if HWND(lp.0 as *mut _) == self.h.borrow().nav => {
                Some(LRESULT(self.st.borrow().gdi.bg.0 as isize))
            }
            WM_DPICHANGED => {
                self.apply_dpi();
                let r = unsafe { &*(lp.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(hwnd, None, r.left, r.top, 0, 0, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE);
                    let _ = RedrawWindow(Some(hwnd), None, None, RDW_ERASE | RDW_INVALIDATE | RDW_ALLCHILDREN);
                }
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => {
                self.paint_background(HDC(wp.0 as *mut _));
                Some(LRESULT(1))
            }
            WM_CLOSE => {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                Some(LRESULT(0))
            }
            WM_NCDESTROY => {
                SETTINGS_HWND.with(|c| c.set(0));
                emit(Action::SettingsClosed);
                None
            }
            _ => None,
        }
    }
}
