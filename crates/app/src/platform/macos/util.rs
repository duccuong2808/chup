//! Tiện ích macOS: thư mục, màn hình (point <-> px), con trỏ, giao diện sáng/tối, shell, mở cùng hệ thống.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    process::Command,
};

use chup_core::{
    coords::{MacLayout, MonitorInfo, Rect, RectPt, ScreenPt},
    i18n::{fill, t},
};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSEvent, NSScreen};
use objc2_foundation::{NSArray, NSRect};

/// Ngôn ngữ ưu tiên của macOS là tiếng Việt.
pub fn system_is_vietnamese() -> bool {
    let langs = objc2_foundation::NSLocale::preferredLanguages();
    langs.firstObject().is_some_and(|l| l.to_string().starts_with("vi"))
}

/// Chọn ngôn ngữ giao diện theo config (và ngôn ngữ của macOS khi config là "theo hệ thống").
pub fn apply_ui_language(pref: chup_core::i18n::UiLanguage) {
    let lang = pref.resolve(system_is_vietnamese());
    chup_core::i18n::set(lang);
    log::info!("ngôn ngữ giao diện: {lang:?} (config {pref:?})");
}

/// Ghi RAM của process vào log: bộ nhớ thường trú hiện tại và đỉnh.
pub fn log_memory(label: &str) {
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
    let n = unsafe {
        libc::proc_pidinfo(std::process::id() as i32, libc::PROC_PIDTASKINFO, 0, &mut info as *mut _ as *mut _, size)
    };
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let mb = |b: u64| b as f64 / 1_048_576.0;
    if n == size {
        log::info!("mem {label}: rss {:.1} MB, đỉnh {:.1} MB", mb(info.pti_resident_size), mb(ru.ru_maxrss as u64));
    }
}

// ---------- thư mục ----------

pub struct Dirs {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub pictures: PathBuf,
}

pub fn dirs() -> Dirs {
    const ID: &str = "app.chup.desktop";
    // Tự kiểm tra có config, log, khoá riêng: không đụng config thật, chạy được song song với app đang dùng.
    let home = super::selftest::dir()
        .map(|d| d.join("home"))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let lib = home.join("Library");
    Dirs {
        config_dir: lib.join("Application Support").join(ID),
        cache_dir: lib.join("Caches").join(ID),
        log_dir: lib.join("Logs").join("Chup"),
        pictures: home.join("Pictures"),
    }
}

// ---------- màn hình ----------

thread_local! {
    /// Bố cục màn hình hiện tại (main thread). Đọc lại khi màn hình đổi (`monitors`).
    static LAYOUT: RefCell<MacLayout> = RefCell::new(MacLayout::default());
    /// `CGDirectDisplayID` theo thứ tự của `NSScreen.screens` (cùng thứ tự `monitors()`).
    static DISPLAY_IDS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

/// `CGDirectDisplayID` của một `NSScreen` (khoá "NSScreenNumber" trong deviceDescription).
fn display_id(s: &NSScreen) -> u32 {
    let desc = s.deviceDescription();
    let key = objc2_foundation::NSString::from_str("NSScreenNumber");
    desc.objectForKey(&key)
        .and_then(|v| v.downcast::<objc2_foundation::NSNumber>().ok())
        .map_or(0, |n| n.unsignedIntValue())
}

/// Màn hình (display ID, rect px, scale) chứa tâm của `r`.
pub fn display_for(r: Rect) -> Option<(u32, MonitorInfo)> {
    let mons = LAYOUT.with(|l| l.borrow().monitors());
    let (cx, cy) = r.center();
    let i = mons.iter().position(|m| m.rect.contains_point(cx, cy))?;
    let id = DISPLAY_IDS.with(|d| d.borrow().get(i).copied())?;
    Some((id, mons[i].clone()))
}

/// Mọi màn hình: (display ID, MonitorInfo).
pub fn displays() -> Vec<(u32, MonitorInfo)> {
    let mons = LAYOUT.with(|l| l.borrow().monitors());
    DISPLAY_IDS.with(|d| d.borrow().iter().copied().zip(mons).collect())
}

fn rect_pt(r: NSRect) -> RectPt {
    RectPt::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

fn screens(mtm: MainThreadMarker) -> Vec<ScreenPt> {
    let list: objc2::rc::Retained<NSArray<NSScreen>> = NSScreen::screens(mtm);
    list.iter()
        .map(|s| ScreenPt {
            key: s.localizedName().to_string(),
            frame: rect_pt(s.frame()),
            visible: rect_pt(s.visibleFrame()),
            scale: s.backingScaleFactor(),
        })
        .collect()
}

/// Đọc lại danh sách màn hình từ AppKit, trả về theo physical px.
pub fn monitors() -> Vec<MonitorInfo> {
    let Some(mtm) = MainThreadMarker::new() else { return LAYOUT.with(|l| l.borrow().monitors()) };
    let ids: Vec<u32> = NSScreen::screens(mtm).iter().map(|s| display_id(&s)).collect();
    DISPLAY_IDS.with(|d| *d.borrow_mut() = ids);
    let layout = MacLayout::new(&screens(mtm));
    let mons = layout.monitors();
    LAYOUT.with(|l| *l.borrow_mut() = layout);
    mons
}

pub fn layout() -> MacLayout {
    LAYOUT.with(|l| l.borrow().clone())
}

/// Rect px -> frame của `NSWindow` (point) và scale của màn hình chứa nó.
pub fn to_window_frame(r: Rect) -> (NSRect, f64) {
    let (p, k) = LAYOUT.with(|l| l.borrow().px_to_pt(r));
    (NSRect::new(objc2_foundation::NSPoint::new(p.x, p.y), objc2_foundation::NSSize::new(p.w, p.h)), k)
}

/// Vị trí con trỏ (physical px toàn desktop).
pub fn cursor_pos() -> (i32, i32) {
    let p = NSEvent::mouseLocation();
    LAYOUT.with(|l| l.borrow().pt_to_px(p.x, p.y))
}

/// Giao diện tối đang bật.
pub fn dark_mode() -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let app = NSApplication::sharedApplication(mtm);
    let appearance = app.effectiveAppearance();
    let names = unsafe { NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]) };
    appearance.bestMatchFromAppearancesWithNames(&names).is_some_and(|n| unsafe { &*n == NSAppearanceNameDarkAqua })
}

// ---------- shell ----------

fn run_open(args: &[&std::ffi::OsStr], path: &Path) -> Result<(), String> {
    match Command::new("/usr/bin/open").args(args).arg(path).status() {
        Ok(s) if s.success() => Ok(()),
        _ => Err(fill(t().err_open_path, &[&path.display()])),
    }
}

/// Mở file, thư mục hoặc URL bằng app mặc định.
pub fn shell_open(path: &Path) -> Result<(), String> {
    run_open(&[], path)
}

/// Mở Finder và chọn sẵn file.
pub fn shell_reveal(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(fill(t().err_not_found, &[&path.display()]));
    }
    run_open(&["-R".as_ref()], path)
}

// ---------- mở cùng hệ thống (SMAppService, macOS 13+) ----------

/// Đang chạy từ `Chup.app` (SMAppService chỉ đăng ký được app có bundle).
pub fn bundled() -> bool {
    objc2_foundation::NSBundle::mainBundle().bundleIdentifier().is_some()
}

pub fn autostart_enabled() -> bool {
    if !bundled() {
        return false;
    }
    let s = unsafe { objc2_service_management::SMAppService::mainAppService() };
    (unsafe { s.status() }) == objc2_service_management::SMAppServiceStatus::Enabled
}

pub fn set_autostart(enable: bool) -> Result<(), String> {
    if !bundled() {
        return Err(t().mac_err_autostart_bundle.to_string());
    }
    let s = unsafe { objc2_service_management::SMAppService::mainAppService() };
    let r = unsafe { if enable { s.registerAndReturnError() } else { s.unregisterAndReturnError() } };
    r.map_err(|e| fill(t().mac_err_autostart, &[&e.localizedDescription()]))
}
