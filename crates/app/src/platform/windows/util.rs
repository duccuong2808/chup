//! Tiện ích Win32 dùng chung: chuỗi UTF-16, màn hình, theme, thư mục, shell, autostart.

use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::Path, path::PathBuf};

use chup_core::coords::{MonitorInfo, Rect};
use windows::{
    core::{w, BOOL, HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, POINT, RECT},
        Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW},
        System::{
            Com::CoTaskMemFree,
            Registry::{
                RegCloseKey, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
                KEY_SET_VALUE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
            },
        },
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            Shell::{
                FOLDERID_LocalAppData, FOLDERID_Pictures, FOLDERID_RoamingAppData, ILCreateFromPathW, ILFree,
                SHGetKnownFolderPath, SHOpenFolderAndSelectItems, ShellExecuteW, KF_FLAG_DEFAULT,
            },
            WindowsAndMessaging::{GetCursorPos, SW_SHOWNORMAL},
        },
    },
};

/// Chuỗi kết thúc bằng NUL cho API Win32.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn rect_of(r: &RECT) -> Rect {
    Rect::new(r.left, r.top, (r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32)
}

pub fn win_rect(r: &Rect) -> RECT {
    RECT { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() }
}

pub fn loword(v: isize) -> i32 {
    (v & 0xFFFF) as u16 as i16 as i32
}

pub fn hiword(v: isize) -> i32 {
    ((v >> 16) & 0xFFFF) as u16 as i16 as i32
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

// ---------- màn hình ----------

unsafe extern "system" fn enum_monitor(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let out = &mut *(data.0 as *mut Vec<MonitorInfo>);
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(h, &mut mi.monitorInfo as *mut _ as *mut _).as_bool() {
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let rect = rect_of(&mi.monitorInfo.rcMonitor);
        let name_len = mi.szDevice.iter().position(|&c| c == 0).unwrap_or(mi.szDevice.len());
        let name = String::from_utf16_lossy(&mi.szDevice[..name_len]);
        out.push(MonitorInfo {
            key: format!("{name}@{},{}", rect.x, rect.y),
            rect,
            work: rect_of(&mi.monitorInfo.rcWork),
            scale: dx as f64 / 96.0,
            primary: mi.monitorInfo.dwFlags & 1 != 0, // MONITORINFOF_PRIMARY
        });
    }
    true.into()
}

/// Danh sách màn hình theo thứ tự OS trả về.
pub fn monitors() -> Vec<MonitorInfo> {
    let mut v: Vec<MonitorInfo> = vec![];
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(enum_monitor), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

// ---------- theme ----------

/// `true` nếu app đang dùng theme tối (Cài đặt > Cá nhân hoá > Màu > Chế độ ứng dụng).
pub fn dark_mode() -> bool {
    let mut val: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut val as *mut _ as *mut _),
            Some(&mut size),
        )
    };
    ok.is_ok() && val == 0
}

// ---------- thư mục ----------

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let p: PWSTR = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.as_wide();
        let path = PathBuf::from(OsString::from_wide(s));
        CoTaskMemFree(Some(p.0 as *const _));
        Some(path)
    }
}

pub struct Dirs {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub pictures: PathBuf,
}

/// Giữ đúng đường dẫn của bản Tauri cũ để không mất cấu hình khi nâng cấp.
pub fn dirs() -> Dirs {
    const ID: &str = "app.chup.desktop";
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"));
    Dirs {
        config_dir: known_folder(&FOLDERID_RoamingAppData).unwrap_or_else(|| home.join("AppData\\Roaming")).join(ID),
        cache_dir: known_folder(&FOLDERID_LocalAppData).unwrap_or_else(|| home.join("AppData\\Local")).join(ID),
        pictures: known_folder(&FOLDERID_Pictures).unwrap_or_else(|| home.join("Pictures")),
    }
}

// ---------- shell ----------

/// Mở file hoặc thư mục bằng app mặc định của Windows.
pub fn shell_open(path: &Path) -> Result<(), String> {
    let p = HSTRING::from(path.as_os_str());
    let r = unsafe { ShellExecuteW(None, w!("open"), &p, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    // ShellExecute trả về giá trị > 32 nếu thành công.
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("Không mở được {}", path.display()))
    }
}

/// Mở Explorer và chọn sẵn file.
pub fn shell_reveal(path: &Path) -> Result<(), String> {
    let p = HSTRING::from(path.as_os_str());
    unsafe {
        let pidl = ILCreateFromPathW(&p);
        if pidl.is_null() {
            return Err(format!("Không tìm thấy {}", path.display()));
        }
        let r = SHOpenFolderAndSelectItems(pidl, None, 0);
        ILFree(Some(pidl));
        r.map_err(|e| format!("Không mở được Explorer: {e}"))
    }
}

// ---------- khởi động cùng Windows ----------

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Chup");

pub fn autostart_enabled() -> bool {
    let mut size = 0u32;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, Some(&mut size)).is_ok() }
}

/// Ghi hoặc xoá giá trị `Chup` trong khoá Run của user hiện tại.
pub fn set_autostart(enable: bool) -> Result<(), String> {
    unsafe {
        let mut key = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut key)
            .ok()
            .map_err(|e| format!("Không mở được khoá Run: {e}"))?;
        let res = if enable {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let cmd = wide(&format!("\"{}\"", exe.display()));
            let bytes = std::slice::from_raw_parts(cmd.as_ptr() as *const u8, cmd.len() * 2);
            RegSetValueExW(key, RUN_VALUE, None, REG_SZ, Some(bytes)).ok()
        } else {
            match RegDeleteValueW(key, RUN_VALUE).ok() {
                // Không có sẵn thì coi như đã tắt.
                Err(e) if e.code().0 as u32 == 0x80070002 => Ok(()),
                r => r,
            }
        };
        let _ = RegCloseKey(key);
        res.map_err(|e| format!("Không đổi được chế độ khởi động cùng Windows: {e}"))
    }
}

/// HWND bọc lại để dùng với crate cần `raw-window-handle` (kéo thả).
pub struct HwndHandle(pub HWND);

impl raw_window_handle::HasWindowHandle for HwndHandle {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let nz = std::num::NonZeroIsize::new(self.0 .0 as isize).ok_or(raw_window_handle::HandleError::Unavailable)?;
        let raw = raw_window_handle::RawWindowHandle::Win32(raw_window_handle::Win32WindowHandle::new(nz));
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}
