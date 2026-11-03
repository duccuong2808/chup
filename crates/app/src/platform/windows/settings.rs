//! Cửa sổ Cài đặt (SPEC 5.6) bằng control gốc của Windows (Common Controls v6).
//! Lưu ngay khi sửa, không có nút Save. Mọi thay đổi gửi `Action::ConfigPatch` cho App.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashSet},
    path::PathBuf,
};

use chup_core::{
    config::{AfterCapture, Config, Edge, OcrLanguage, PreviewPosition, StackMonitor},
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
            CreateFontIndirectW, DeleteObject, FillRect, GetSysColor, GetSysColorBrush, SetBkColor, SetTextColor,
            COLOR_WINDOW,
            FW_SEMIBOLD, HBRUSH, HDC, HFONT, HGDIOBJ,
        },
        System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER},
        UI::{
            HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow, SystemParametersInfoForDpi},
            Input::KeyboardAndMouse::{
                EnableWindow, GetFocus, GetKeyState, SetFocus, VK_BACK, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_LWIN,
                VK_MENU, VK_RWIN, VK_SHIFT,
            },
            Shell::{
                DefSubclassProc, SetWindowSubclass, FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM,
                FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, KillTimer,
                SendMessageW, SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow, BM_GETCHECK,
                BM_SETCHECK, BN_CLICKED, BS_AUTOCHECKBOX, BS_PUSHBUTTON, CBN_SELCHANGE, CBS_DROPDOWNLIST,
                CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, DLGC_WANTALLKEYS, EN_CHANGE, ES_AUTOHSCROLL, ES_READONLY,
                GWL_EXSTYLE, GWL_STYLE, HMENU, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE,
                SWP_NOZORDER, SW_HIDE, SW_SHOW, SW_SHOWNORMAL, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE,
                WINDOW_STYLE, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_GETDLGCODE,
                WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_NCDESTROY, WM_SETFOCUS, WM_SETFONT, WM_SYSCHAR,
                WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT,
                WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
            },
        },
    },
};

use super::{
    app::Action,
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
}

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
const TIMER_PATTERN: usize = 1;

const RECORDING_TEXT: &str = "Nhấn tổ hợp phím…";

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
}

#[derive(Default)]
struct Fonts {
    normal: HFONT,
    bold: HFONT,
    title: HFONT,
}

impl Fonts {
    fn delete(&mut self) {
        for f in [self.normal, self.bold, self.title] {
            if !f.is_invalid() {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(f.0));
                }
            }
        }
        *self = Fonts::default();
    }
}

#[derive(Default)]
struct State {
    ctls: Vec<Ctl>,
    fonts: Fonts,
    view: Option<SettingsView>,
    /// Lỗi phát hiện ngay lúc ghi phím (chưa lưu vào config), ưu tiên hơn lỗi từ App.
    record_errors: BTreeMap<String, String>,
    recording: Option<&'static str>,
    seen_down: HashSet<u16>,
    status: (String, bool),
}

pub struct Settings {
    pub hwnd: HWND,
    st: RefCell<State>,
    updating: Cell<bool>,
    // HWND của từng control, để truy cập nhanh.
    h: RefCell<Handles>,
}

#[derive(Default, Clone, Copy)]
struct Handles {
    title: HWND,
    sec_save: HWND,
    lbl_dir: HWND,
    dir: HWND,
    pick: HWND,
    open: HWND,
    lbl_pattern: HWND,
    pattern: HWND,
    hint_pattern: HWND,
    sec_hk: HWND,
    lbl_hk1: HWND,
    hk1: HWND,
    err1: HWND,
    lbl_hk2: HWND,
    hk2: HWND,
    err2: HWND,
    lbl_hk3: HWND,
    hk3: HWND,
    err3: HWND,
    lbl_hk4: HWND,
    hk4: HWND,
    err4: HWND,
    hint_hk: HWND,
    sec_ocr: HWND,
    lbl_ocr_lang: HWND,
    ocr_lang: HWND,
    ocr_status: HWND,
    ocr_pack: HWND,
    ocr_auto: HWND,
    sec_auto: HWND,
    auto_on: HWND,
    auto_img: HWND,
    hint_auto: HWND,
    sec_after: HWND,
    copy: HWND,
    lbl_after: HWND,
    after: HWND,
    hint_after: HWND,
    sec_stack: HWND,
    lbl_edge: HWND,
    edge: HWND,
    lbl_mon: HWND,
    mon: HWND,
    sec_pv: HWND,
    pv_top: HWND,
    lbl_pv_op: HWND,
    pv_op: HWND,
    lbl_pv_pos: HWND,
    pv_pos: HWND,
    lbl_pv_size: HWND,
    pv_size: HWND,
    sec_sys: HWND,
    autostart: HWND,
    hideui: HWND,
    note: HWND,
    status: HWND,
}

thread_local! {
    static CLASS: Vec<u16> = win::register_class::<Settings>("ChupSettings");
    /// Cho vòng lặp message gọi IsDialogMessage (Tab giữa các control).
    pub static SETTINGS_HWND: Cell<isize> = const { Cell::new(0) };
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

impl Settings {
    pub fn create() -> windows::core::Result<std::rc::Rc<Settings>> {
        let me = CLASS.with(|cls| {
            win::create(
                win::CreateParams {
                    class: cls,
                    title: "Chup — Cài đặt",
                    style: WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                    ex_style: WS_EX_CONTROLPARENT,
                    x: windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
                    y: windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
                    w: 800,
                    h: 600,
                    parent: None,
                },
                |hwnd| Settings {
                    hwnd,
                    st: RefCell::new(State::default()),
                    updating: Cell::new(false),
                    h: RefCell::new(Handles::default()),
                },
            )
        })?;
        me.build();
        me.set_icon();
        SETTINGS_HWND.with(|c| c.set(me.hwnd.0 as isize));
        Ok(me)
    }

    fn add(&self, class: PCWSTR, text: &str, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE, id: u16, kind: Kind) -> HWND {
        let t = wide(text);
        let h = unsafe {
            CreateWindowExW(
                ex,
                class,
                PCWSTR(t.as_ptr()),
                WS_CHILD | WS_VISIBLE | style,
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
        self.st.borrow_mut().ctls.push(Ctl { hwnd: h, kind });
        h
    }

    fn build(&self) {
        let none = WINDOW_EX_STYLE(0);
        let s = WINDOW_STYLE(0);
        let edit = WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
        let check = WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32);
        let button = WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32);
        let combo = WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32);
        let st = w!("STATIC");
        let mut h = Handles {
            title: self.add(st, "Cài đặt", s, none, 0, Kind::Title),
            sec_save: self.add(st, "LƯU ẢNH", s, none, 0, Kind::Section),
            lbl_dir: self.add(st, "Folder lưu ảnh", s, none, 0, Kind::Label),
            dir: self.add(w!("EDIT"), "", WINDOW_STYLE((ES_READONLY | ES_AUTOHSCROLL) as u32), WS_EX_CLIENTEDGE, 0, Kind::Control),
            pick: self.add(w!("BUTTON"), "Chọn folder…", button, none, ID_PICK, Kind::Control),
            open: self.add(w!("BUTTON"), "Mở folder", button, none, ID_OPEN, Kind::Control),
            lbl_pattern: self.add(st, "Tên file", s, none, 0, Kind::Label),
            pattern: self.add(w!("EDIT"), "", edit, WS_EX_CLIENTEDGE, ID_PATTERN, Kind::Control),
            hint_pattern: self.add(st, "", s, none, 0, Kind::Hint),
            sec_hk: self.add(st, "PHÍM TẮT", s, none, 0, Kind::Section),
            lbl_hk1: self.add(st, "Chọn vùng", s, none, 0, Kind::Label),
            hk1: self.add(w!("EDIT"), "", edit, WS_EX_CLIENTEDGE, ID_HK_SELECT, Kind::Control),
            err1: self.add(st, "", s, none, 0, Kind::Error),
            lbl_hk2: self.add(st, "Chụp vùng hiện tại / vùng cuối", s, none, 0, Kind::Label),
            hk2: self.add(w!("EDIT"), "", edit, WS_EX_CLIENTEDGE, ID_HK_CAPTURE, Kind::Control),
            err2: self.add(st, "", s, none, 0, Kind::Error),
            lbl_hk3: self.add(st, "Chụp đóng băng (menu, tooltip)", s, none, 0, Kind::Label),
            hk3: self.add(w!("EDIT"), "", edit, WS_EX_CLIENTEDGE, ID_HK_FREEZE, Kind::Control),
            err3: self.add(st, "", s, none, 0, Kind::Error),
            lbl_hk4: self.add(st, "Chụp text (chọn vùng rồi copy chữ)", s, none, 0, Kind::Label),
            hk4: self.add(w!("EDIT"), "", edit, WS_EX_CLIENTEDGE, ID_HK_OCR, Kind::Control),
            err4: self.add(st, "", s, none, 0, Kind::Error),
            hint_hk: self.add(
                st,
                "Bấm vào ô rồi nhấn tổ hợp phím. Delete để tắt phím tắt, Esc để huỷ.",
                s,
                none,
                0,
                Kind::Hint,
            ),
            sec_ocr: self.add(st, "COPY CHỮ TỪ ẢNH (OCR)", s, none, 0, Kind::Section),
            lbl_ocr_lang: self.add(st, "Ngôn ngữ", s, none, 0, Kind::Label),
            ocr_lang: self.add(w!("COMBOBOX"), "", combo, none, ID_OCR_LANG, Kind::Control),
            ocr_status: self.add(st, "", s, none, 0, Kind::Hint),
            ocr_pack: self.add(w!("BUTTON"), "", button, none, ID_OCR_PACK, Kind::Control),
            ocr_auto: self.add(w!("BUTTON"), "Tự nhận diện chữ khi mở Preview", check, none, ID_OCR_AUTO, Kind::Control),
            sec_auto: self.add(st, "TỰ ĐỘNG CHỌN VÙNG", s, none, 0, Kind::Section),
            auto_on: self.add(
                w!("BUTTON"),
                "Rê chuột là tô sẵn khung cửa sổ, nút, ảnh…",
                check,
                none,
                ID_AUTO_ON,
                Kind::Control,
            ),
            auto_img: self.add(
                w!("BUTTON"),
                "Đoán khung từ hình ảnh khi app không tự báo",
                check,
                none,
                ID_AUTO_IMG,
                Kind::Control,
            ),
            hint_auto: self.add(
                st,
                "Click để chọn khung đang tô. Lăn chuột hoặc ↑↓ để đổi cỡ khung, giữ Shift để tạm tắt.",
                s,
                none,
                0,
                Kind::Hint,
            ),
            sec_after: self.add(st, "SAU KHI CHỤP", s, none, 0, Kind::Section),
            copy: self.add(w!("BUTTON"), "Copy ảnh vào clipboard", check, none, ID_COPY, Kind::Control),
            lbl_after: self.add(st, "Vùng chọn ở chế độ thường", s, none, 0, Kind::Label),
            after: self.add(w!("COMBOBOX"), "", combo, none, ID_AFTER, Kind::Control),
            hint_after: self.add(st, "Ở chế độ Mờ, vùng chọn luôn được giữ.", s, none, 0, Kind::Hint),
            sec_stack: self.add(st, "STACK ẢNH", s, none, 0, Kind::Section),
            lbl_edge: self.add(st, "Cạnh màn hình", s, none, 0, Kind::Label),
            edge: self.add(w!("COMBOBOX"), "", combo, none, ID_EDGE, Kind::Control),
            lbl_mon: self.add(st, "Màn hình", s, none, 0, Kind::Label),
            mon: self.add(w!("COMBOBOX"), "", combo, none, ID_MONITOR, Kind::Control),
            sec_pv: self.add(st, "PREVIEW (BẤM VÀO THUMBNAIL)", s, none, 0, Kind::Section),
            pv_top: self.add(w!("BUTTON"), "Luôn nằm trên cửa sổ khác", check, none, ID_PV_TOP, Kind::Control),
            lbl_pv_op: self.add(st, "Opacity (Ctrl + lăn chuột để chỉnh tạm)", s, none, 0, Kind::Label),
            pv_op: self.add(w!("COMBOBOX"), "", combo, none, ID_PV_OPACITY, Kind::Control),
            lbl_pv_pos: self.add(st, "Chỗ mở", s, none, 0, Kind::Label),
            pv_pos: self.add(w!("COMBOBOX"), "", combo, none, ID_PV_POS, Kind::Control),
            lbl_pv_size: self.add(st, "Cỡ khi mở (kéo mép hoặc lăn chuột để đổi)", s, none, 0, Kind::Label),
            pv_size: self.add(w!("COMBOBOX"), "", combo, none, ID_PV_SIZE, Kind::Control),
            sec_sys: self.add(st, "HỆ THỐNG", s, none, 0, Kind::Section),
            autostart: self.add(w!("BUTTON"), "Khởi động cùng Windows", check, none, ID_AUTOSTART, Kind::Control),
            hideui: self.add(w!("BUTTON"), "Ẩn toolbar và stack trong lúc chụp", check, none, ID_HIDEUI, Kind::Control),
            note: self.add(st, "", s, none, 0, Kind::Hint),
            status: self.add(st, "", s, none, 0, Kind::Hint),
        };
        for (c, items) in [
            (h.after, &["Đóng vùng chọn", "Giữ vùng chọn để chụp tiếp"][..]),
            (h.edge, &["Phải", "Trái"][..]),
            (h.ocr_lang, &["Tiếng Anh (có sẵn trong Windows)", "Tiếng Việt + Anh (cần tải gói dữ liệu)"][..]),
            (h.mon, &["Màn hình chính", "Màn hình đang có con trỏ"][..]),
            (h.pv_op, &["100%", "90%", "80%", "70%", "60%", "50%", "40%", "30%", "20%"][..]),
            (h.pv_pos, &["Chỗ và cỡ lần trước", "Cạnh stack", "Giữa màn hình"][..]),
            (
                h.pv_size,
                &["Tối đa 30% màn hình", "Tối đa 50% màn hình", "Tối đa 70% màn hình", "Tối đa 90% màn hình"][..],
            ),
        ] {
            for it in items {
                let t = wide(it);
                unsafe { SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize))) };
            }
        }
        for hk in [h.hk1, h.hk2, h.hk3, h.hk4] {
            unsafe {
                let _ = SetWindowSubclass(hk, Some(hotkey_proc), 1, self.hwnd.0 as usize);
            }
        }
        // Tránh cảnh báo khi ô path chỉ đọc: vẫn cho Tab qua.
        let _ = &mut h;
        *self.h.borrow_mut() = h;
        self.apply_dpi();
    }

    fn set_icon(&self) {
        super::util::set_app_icon(self.hwnd);
    }

    fn dpi(&self) -> u32 {
        unsafe { GetDpiForWindow(self.hwnd) }.max(96)
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
        lf.lfHeight = -((22 * dpi / 96) as i32);
        let title = unsafe { CreateFontIndirectW(&lf) };
        {
            let mut st = self.st.borrow_mut();
            st.fonts.delete();
            st.fonts = Fonts { normal, bold, title };
            for c in &st.ctls {
                let f = match c.kind {
                    Kind::Title => title,
                    Kind::Section => bold,
                    _ => normal,
                };
                unsafe { SendMessageW(c.hwnd, WM_SETFONT, Some(WPARAM(f.0 as usize)), Some(LPARAM(1))) };
            }
        }
        self.layout();
    }

    /// Bố cục 2 cột (DIP), nhân theo DPI. Ô báo lỗi trống thì ẩn và không chiếm chỗ.
    fn layout(&self) {
        let k = self.dpi() as f64 / 96.0;
        let p = |v: f64| (v * k).round() as i32;
        let h = *self.h.borrow();
        let mv = |c: HWND, x: f64, y: f64, w: f64, hh: f64| unsafe {
            let _ = SetWindowPos(c, None, p(x), p(y), p(w), p(hh), SWP_NOZORDER | SWP_NOACTIVATE);
        };
        let (margin, col, gutter) = (20.0, 340.0, 28.0);
        let (x0, x1) = (margin, margin + col + gutter);

        mv(h.title, x0, 14.0, col, 34.0);
        // Cột trái.
        let mut y = 58.0;
        let section = |c: HWND, x: f64, y: &mut f64| {
            mv(c, x, *y, col, 18.0);
            *y += 22.0;
        };
        section(h.sec_save, x0, &mut y);
        mv(h.lbl_dir, x0, y, col, 18.0);
        y += 20.0;
        mv(h.dir, x0, y, col, 24.0);
        y += 30.0;
        mv(h.pick, x0, y, 124.0, 28.0);
        mv(h.open, x0 + 132.0, y, 100.0, 28.0);
        y += 38.0;
        mv(h.lbl_pattern, x0, y, col, 18.0);
        y += 20.0;
        mv(h.pattern, x0, y, col, 24.0);
        y += 28.0;
        mv(h.hint_pattern, x0, y, col, 34.0);
        y += 46.0;
        section(h.sec_hk, x0, &mut y);
        let has = |c: HWND| !get_text(c).is_empty();
        for (lbl, edit, err) in [
            (h.lbl_hk1, h.hk1, h.err1),
            (h.lbl_hk2, h.hk2, h.err2),
            (h.lbl_hk3, h.hk3, h.err3),
            (h.lbl_hk4, h.hk4, h.err4),
        ] {
            let show = has(err);
            mv(lbl, x0, y, col, 18.0);
            y += 20.0;
            mv(edit, x0, y, col, 26.0);
            y += 30.0;
            unsafe {
                let _ = ShowWindow(err, if show { SW_SHOW } else { SW_HIDE });
            }
            if show {
                mv(err, x0, y, col, 34.0);
                y += 38.0;
            }
            y += 4.0;
        }
        mv(h.hint_hk, x0, y, col, 34.0);
        y += 46.0;
        section(h.sec_ocr, x0, &mut y);
        mv(h.lbl_ocr_lang, x0, y, col, 18.0);
        y += 20.0;
        mv(h.ocr_lang, x0, y, col, 200.0);
        y += 32.0;
        mv(h.ocr_status, x0, y, col, 34.0);
        y += 38.0;
        mv(h.ocr_pack, x0, y, 150.0, 28.0);
        y += 38.0;
        mv(h.ocr_auto, x0, y, col, 22.0);
        y += 40.0;
        section(h.sec_auto, x0, &mut y);
        mv(h.auto_on, x0, y, col, 22.0);
        y += 26.0;
        mv(h.auto_img, x0, y, col, 22.0);
        y += 28.0;
        mv(h.hint_auto, x0, y, col, 34.0);
        let left_bottom = y + 34.0;

        // Cột phải.
        let mut y = 58.0;
        section(h.sec_after, x1, &mut y);
        mv(h.copy, x1, y, col, 22.0);
        y += 30.0;
        mv(h.lbl_after, x1, y, col, 18.0);
        y += 20.0;
        mv(h.after, x1, y, col, 200.0);
        y += 30.0;
        mv(h.hint_after, x1, y, col, 18.0);
        y += 30.0;
        section(h.sec_stack, x1, &mut y);
        mv(h.lbl_edge, x1, y, col, 18.0);
        y += 20.0;
        mv(h.edge, x1, y, col, 200.0);
        y += 32.0;
        mv(h.lbl_mon, x1, y, col, 18.0);
        y += 20.0;
        mv(h.mon, x1, y, col, 200.0);
        y += 40.0;
        section(h.sec_pv, x1, &mut y);
        mv(h.pv_top, x1, y, col, 22.0);
        y += 30.0;
        mv(h.lbl_pv_op, x1, y, col, 18.0);
        y += 20.0;
        mv(h.pv_op, x1, y, col, 200.0);
        y += 32.0;
        mv(h.lbl_pv_pos, x1, y, col, 18.0);
        y += 20.0;
        mv(h.pv_pos, x1, y, col, 200.0);
        y += 32.0;
        mv(h.lbl_pv_size, x1, y, col, 18.0);
        y += 20.0;
        mv(h.pv_size, x1, y, col, 200.0);
        y += 40.0;
        section(h.sec_sys, x1, &mut y);
        mv(h.autostart, x1, y, col, 22.0);
        y += 26.0;
        mv(h.hideui, x1, y, col, 22.0);
        y += 26.0;
        mv(h.note, x1, y, col, 50.0);
        y += 56.0;
        mv(h.status, x1, y, col, 20.0);
        let right_bottom = y + 20.0;

        // Kích thước cửa sổ theo nội dung.
        let (cw, ch) = (p(margin * 2.0 + col * 2.0 + gutter), p(left_bottom.max(right_bottom) + 16.0));
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
                SWP_NOZORDER | SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE,
            );
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    /// Hiển thị lại theo state mới từ App.
    pub fn refresh(&self, v: SettingsView) {
        self.updating.set(true);
        let h = *self.h.borrow();
        let c = &v.config;
        set_text(h.dir, &c.save_dir.display().to_string());
        if unsafe { GetFocus() } != h.pattern {
            set_text(h.pattern, &c.file_name_pattern);
        }
        set_text(
            h.hint_pattern,
            &format!("Phần trong {{ }} là ngày giờ: yyyy MM dd HH mm ss SSS. Ví dụ: {}", v.preview),
        );
        set_checked(h.copy, c.copy_to_clipboard);
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
        let (status, button, enabled) = match v.ocr_vi {
            OcrViState::Ready => ("Gói dữ liệu tiếng Việt: đã cài.".to_string(), "Xoá gói".to_string(), true),
            OcrViState::NoPack => (
                "Gói dữ liệu tiếng Việt: chưa cài. Tải từ GitHub (tesseract-ocr/tessdata_fast).".to_string(),
                format!("Tải gói ({})", pack::size_label()),
                true,
            ),
            OcrViState::NoHelper => (
                "Thiếu chup-ocr.exe: hãy cài Chup bằng bộ cài để dùng tiếng Việt.".to_string(),
                format!("Tải gói ({})", pack::size_label()),
                false,
            ),
        };
        set_text(h.ocr_status, &status);
        set_text(h.ocr_pack, &button);
        unsafe {
            let _ = EnableWindow(h.ocr_pack, enabled);
        }
        set_checked(h.autostart, c.launch_at_startup);
        set_checked(h.hideui, c.hide_ui_during_capture || !v.exclusion_ok);
        unsafe {
            let _ = EnableWindow(h.hideui, v.exclusion_ok);
        }
        set_text(
            h.note,
            if v.exclusion_ok {
                "Thường không cần bật: Windows đã loại toolbar và stack khỏi ảnh chụp. Bật nếu vẫn thấy chúng lọt vào ảnh."
            } else {
                "Windows này không hỗ trợ loại cửa sổ khỏi ảnh chụp (cần Windows 10 bản 2004 trở lên), nên Chup luôn ẩn toolbar và stack trong lúc chụp."
            },
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
                set_text(edit, if val.is_empty() { "Không dùng" } else { val });
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
        set_text(edit, RECORDING_TEXT);
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
                    format!("{a} đang được app khác (hoặc Windows) dùng nên Chup không đăng ký được. Hãy chọn tổ hợp khác."),
                );
                set_text(edit, RECORDING_TEXT);
                self.render_hotkeys();
            }
        }
        true
    }
}

impl Drop for Settings {
    fn drop(&mut self) {
        self.st.get_mut().fonts.delete();
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
    let ptr = GetWindowLongPtrW(parent, windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA) as *const Settings;
    if ptr.is_null() {
        return DefSubclassProc(hwnd, msg, wp, lp);
    }
    let me = &*ptr;
    let id = windows::Win32::UI::WindowsAndMessaging::GetDlgCtrlID(hwnd) as u16;
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
        windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN
        | windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDBLCLK => {
            let _ = SetFocus(Some(hwnd));
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE => LRESULT(0),
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
        let _ = dlg.SetTitle(w!("Chọn folder lưu ảnh"));
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
                    (ID_PICK, BN_CLICKED) => self.pick_folder(),
                    (ID_OPEN, BN_CLICKED) => emit(Action::OpenSaveDir),
                    (ID_PATTERN, EN_CHANGE) if !self.updating.get() => unsafe {
                        SetTimer(Some(hwnd), TIMER_PATTERN, 400, None);
                    },
                    (ID_COPY, BN_CLICKED) => self.patch(json!({ "copyToClipboard": checked(h.copy) })),
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
            WM_CTLCOLORSTATIC => {
                let dc = HDC(wp.0 as *mut _);
                let ctl = HWND(lp.0 as *mut _);
                let kind = self.st.borrow().ctls.iter().find(|c| c.hwnd == ctl).map(|c| c.kind);
                let error_status = ctl == self.h.borrow().status && self.st.borrow().status.1;
                let color = match kind {
                    Some(Kind::Error) => Some(0x2f24d1),
                    _ if error_status => Some(0x2f24d1),
                    Some(Kind::Hint | Kind::Section) => Some(0x70635b),
                    _ => None,
                };
                unsafe {
                    if let Some(c) = color {
                        SetTextColor(dc, COLORREF(c));
                    }
                    SetBkColor(dc, COLORREF(GetSysColor(COLOR_WINDOW)));
                    let b: HBRUSH = GetSysColorBrush(COLOR_WINDOW);
                    Some(LRESULT(b.0 as isize))
                }
            }
            WM_DPICHANGED => {
                self.apply_dpi();
                let r = unsafe { &*(lp.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        0,
                        0,
                        SWP_NOZORDER | SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE,
                    );
                }
                Some(LRESULT(0))
            }
            windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => {
                // Class không có brush nền: tự tô màu nền cửa sổ.
                let dc = HDC(wp.0 as *mut _);
                let mut r = RECT::default();
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut r);
                    FillRect(dc, &r, GetSysColorBrush(COLOR_WINDOW));
                }
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
