//! Hạ tầng cửa sổ: đăng ký class, wndproc chung, hàng đợi Action.
//!
//! Quy tắc tránh re-entrancy (mọi thứ chạy trên 1 UI thread):
//! - Mỗi cửa sổ là 1 struct trong `Rc`, con trỏ lưu ở `GWLP_USERDATA`. State bên trong dùng
//!   `Cell`/`RefCell`, **không giữ borrow khi gọi API Win32** (API có thể gửi message ngược lại).
//! - Cửa sổ không gọi thẳng vào `App`. Chúng `emit(Action)`; App xử lý khi nhận `WM_APP_ACTION`.
//!   Nhờ vậy vòng lặp modal (kéo/resize frame, kéo thả file, dialog) vẫn xử lý được Action.

use std::{
    rc::Rc,
    sync::{
        atomic::{AtomicIsize, Ordering},
        Mutex,
    },
};

use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, LoadCursorW, PostMessageW, RegisterClassExW,
            SetWindowLongPtrW, CS_DBLCLKS, GWLP_USERDATA, HMENU, IDC_ARROW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
            WM_NCDESTROY, WNDCLASSEXW,
        },
    },
};

use super::{app::Action, util::wide};

/// Có Action mới trong hàng đợi.
pub const WM_APP_ACTION: u32 = WM_APP + 1;
/// Instance thứ 2 nhờ instance đang chạy mở Cài đặt.
pub const WM_APP_SHOW_SETTINGS: u32 = WM_APP + 2;
/// Hỏi/tải gói OCR tiếng Việt (wParam 1 = hỏi trước). Chạy ngoài lúc App đang bị borrow vì hộp thoại là modal.
pub const WM_APP_OCR_DOWNLOAD: u32 = WM_APP + 3;

static QUEUE: Mutex<Vec<Action>> = Mutex::new(Vec::new());
static APP_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn set_app_hwnd(h: HWND) {
    APP_HWND.store(h.0 as isize, Ordering::SeqCst);
}

/// Cửa sổ ẩn của app (đặt timer hẹn giờ Freeze trên đó).
pub fn app_hwnd() -> HWND {
    HWND(APP_HWND.load(Ordering::SeqCst) as *mut _)
}

/// Gửi Action cho App. Gọi được từ mọi thread.
pub fn emit(a: Action) {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push(a);
    let h = APP_HWND.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_APP_ACTION, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn take_actions() -> Vec<Action> {
    std::mem::take(&mut *QUEUE.lock().unwrap_or_else(|e| e.into_inner()))
}

pub fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}

/// Struct của 1 cửa sổ. `None` = để DefWindowProc xử lý.
pub trait Handler: 'static {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT>;
}

unsafe extern "system" fn wndproc<T: Handler>(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const T;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if msg == WM_NCDESTROY {
        // Cho struct biết cửa sổ sắp mất, rồi mới thả nó.
        let _ = (*ptr).handle(hwnd, msg, wp, lp);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        // Trả lại phần Rc đã giữ trong GWLP_USERDATA.
        drop(Rc::from_raw(ptr));
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    // Giữ 1 bản Rc trong lúc xử lý, phòng khi handler gây DestroyWindow.
    Rc::increment_strong_count(ptr);
    let me = Rc::from_raw(ptr);
    let t0 = std::time::Instant::now();
    let r = me.handle(hwnd, msg, wp, lp);
    record_ui::<T>(msg, t0.elapsed());
    drop(me);
    match r {
        Some(v) => v,
        None => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ---------- đo thời gian xử lý message (chup-3p3) ----------

/// (loại cửa sổ, message) -> (số lần, tổng thời gian). Chỉ UI thread ghi. Message lồng nhau (SendMessage trong
/// handler) bị tính cả ở message ngoài: số liệu để biết chỗ nào tốn, không phải để cộng ra tổng CPU.
type UiStats = std::collections::HashMap<(&'static str, u32), (u32, std::time::Duration)>;

thread_local! {
    static UI_STATS: std::cell::RefCell<UiStats> = Default::default();
}

fn record_ui<T: Handler>(msg: u32, dt: std::time::Duration) {
    // Bỏ qua message rẻ (đa số): đỡ tốn cho chính việc đo.
    if dt < std::time::Duration::from_micros(500) {
        return;
    }
    let name = std::any::type_name::<T>().rsplit("::").next().unwrap_or("?");
    UI_STATS.with(|s| {
        let mut s = s.borrow_mut();
        let e = s.entry((name, msg)).or_default();
        e.0 += 1;
        e.1 += dt;
    });
}

fn msg_name(msg: u32) -> String {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let n = match msg {
        WM_MOUSEMOVE => "WM_MOUSEMOVE",
        WM_MOUSEWHEEL => "WM_MOUSEWHEEL",
        WM_LBUTTONDOWN => "WM_LBUTTONDOWN",
        WM_LBUTTONUP => "WM_LBUTTONUP",
        WM_TIMER => "WM_TIMER",
        WM_PAINT => "WM_PAINT",
        WM_KEYDOWN => "WM_KEYDOWN",
        WM_SETCURSOR => "WM_SETCURSOR",
        0x02A3 => "WM_MOUSELEAVE",
        WM_WINDOWPOSCHANGED => "WM_WINDOWPOSCHANGED",
        WM_SIZE => "WM_SIZE",
        WM_MOVE => "WM_MOVE",
        WM_COMMAND => "WM_COMMAND",
        WM_DPICHANGED => "WM_DPICHANGED",
        WM_ERASEBKGND => "WM_ERASEBKGND",
        m if m == WM_APP_ACTION => "WM_APP_ACTION",
        _ => return format!("0x{msg:04X}"),
    };
    n.to_string()
}

/// Ghi vào log các (cửa sổ, message) tốn thời gian nhất từ lần ghi trước, rồi đặt lại. Gọi lúc trả RAM (rảnh).
pub fn log_ui_stats() {
    let mut v: Vec<_> = UI_STATS.with(|s| s.borrow_mut().drain().collect());
    if v.is_empty() {
        return;
    }
    v.sort_by_key(|x| std::cmp::Reverse(x.1 .1));
    let total: std::time::Duration = v.iter().map(|x| x.1 .1).sum();
    log::info!("ui: tổng {total:?} ở message ≥ 0,5 ms");
    for ((win, msg), (n, dt)) in v.into_iter().take(10) {
        log::info!("ui: {win} {}: {n} lần, {dt:?} (tb {:?})", msg_name(msg), dt / n.max(1));
    }
}

/// Đăng ký class (một lần cho mỗi tên) dùng wndproc của `T`.
pub fn register_class<T: Handler>(name: &str) -> Vec<u16> {
    let cls = wide(name);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS,
        lpfnWndProc: Some(wndproc::<T>),
        hInstance: hinstance(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        lpszClassName: PCWSTR(cls.as_ptr()),
        ..Default::default()
    };
    unsafe { RegisterClassExW(&wc) };
    cls
}

pub struct CreateParams<'a> {
    pub class: &'a [u16],
    pub title: &'a str,
    pub style: WINDOW_STYLE,
    pub ex_style: WINDOW_EX_STYLE,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub parent: Option<HWND>,
}

/// Tạo cửa sổ rồi gắn struct (do `make` dựng từ HWND) vào GWLP_USERDATA.
pub fn create<T: Handler>(p: CreateParams, make: impl FnOnce(HWND) -> T) -> windows::core::Result<Rc<T>> {
    let title = wide(p.title);
    let hwnd = unsafe {
        CreateWindowExW(
            p.ex_style,
            PCWSTR(p.class.as_ptr()),
            PCWSTR(title.as_ptr()),
            p.style,
            p.x,
            p.y,
            p.w,
            p.h,
            p.parent,
            None::<HMENU>,
            Some(hinstance()),
            None,
        )?
    };
    let rc = Rc::new(make(hwnd));
    let raw = Rc::into_raw(rc.clone());
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize) };
    Ok(rc)
}
