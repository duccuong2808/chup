//! Chup trên macOS: AppKit + CoreGraphics/CoreText, không WebView (SPEC 11, thiết kế
//! `docs/plans/2026-11-05-macos-design.md`).
//!
//! - App chạy kiểu accessory (`LSUIElement`): không có icon ở Dock, chỉ có icon trên menu bar.
//! - Một UI thread: mọi nơi `emit(Action)`, `drain` xử lý trên main thread (`win.rs`).

mod app;
mod autoselect;
mod capture;
mod frame;
mod gfx;
#[path = "../common/hotkeys.rs"]
mod hotkeys;
#[path = "../common/logger.rs"]
mod logger;
mod overlay;
mod panel;
mod preview;
mod selftest;
mod settings;
mod stack;
mod toolbar;
#[path = "../common/tray.rs"]
mod tray;
mod util;
mod vision;
mod win;

use std::{cell::RefCell, os::fd::AsRawFd, ptr::NonNull};

use block2::RcBlock;
use chup_core::{
    config,
    model::{Model, Paths, Toast},
};
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSOperationQueue, NSString};

use self::{
    app::{Action, App},
    win::{emit, take_actions, Timer},
};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Hẹn giờ trả RAM khi rảnh, đếm lại sau mỗi lần có hoạt động.
    static TRIM: RefCell<Option<Timer>> = const { RefCell::new(None) };
    /// Hẹn giờ gộp các lần đổi màn hình liên tiếp.
    static DISPLAY: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

const TRIM_AFTER_MS: u32 = 15_000;
const SHOW_SETTINGS: &str = "app.chup.desktop.show-settings";

/// Xử lý hết Action đang chờ. App đang bận (gọi lồng) thì thử lại sau 10 ms.
fn drain() {
    APP.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            std::mem::forget(win::after(10, drain));
            return;
        };
        let Some(app) = guard.as_mut() else { return };
        loop {
            let actions = take_actions();
            if actions.is_empty() {
                break;
            }
            for a in actions {
                app.handle(a);
            }
        }
    });
    arm_trim();
}

fn arm_trim() {
    let t = win::after(TRIM_AFTER_MS, || {
        APP.with(|cell| {
            if let Ok(guard) = cell.try_borrow() {
                match guard.as_ref() {
                    Some(app) if app.is_idle() => app.trim(),
                    Some(_) => arm_trim(),
                    None => {}
                }
            }
        })
    });
    TRIM.with(|c| *c.borrow_mut() = Some(t));
}

/// Hỏi quyền Screen Recording khi người dùng chụp mà chưa có quyền (chạy ngoài lúc App bận: hộp thoại modal có vòng
/// lặp riêng). Lần đầu macOS cũng tự hiện hộp thoại của nó; hộp thoại này dẫn thẳng tới Cài đặt hệ thống.
pub(crate) fn ask_permission() {
    if capture::request_permission() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let t = chup_core::i18n::t();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(t.mac_perm_title));
    alert.setInformativeText(&NSString::from_str(t.mac_perm_body));
    alert.addButtonWithTitle(&NSString::from_str(t.mac_perm_open));
    alert.addButtonWithTitle(&NSString::from_str(t.mac_perm_later));
    NSApplication::sharedApplication(mtm).activate();
    if alert.runModal() == NSAlertFirstButtonReturn {
        capture::open_permission_settings();
    }
}

/// Chỉ 1 instance (SPEC 7): khoá file. Instance mới nhờ instance cũ mở Cài đặt rồi thoát.
fn single_instance(dir: &std::path::Path) -> Option<std::fs::File> {
    let _ = std::fs::create_dir_all(dir);
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("chup.lock")).ok()?;
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (r == 0).then_some(f)
}

fn observe(center: &NSNotificationCenter, name: &NSString, f: impl Fn() + 'static) {
    let block = RcBlock::new(move |_n: NonNull<NSNotification>| f());
    let queue = NSOperationQueue::mainQueue();
    let token = unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, Some(&queue), &block) };
    // Theo dõi suốt đời app.
    std::mem::forget(token);
}

fn init(mtm: MainThreadMarker) {
    let dirs = util::dirs();
    let default_save_dir = dirs.pictures.join("Chup");
    let config_file = dirs.config_dir.join("config.json");
    let thumbs_dir = dirs.cache_dir.join("thumbs");
    let _ = std::fs::remove_dir_all(&thumbs_dir);
    let loaded = config::load(&config_file, &default_save_dir);
    util::apply_ui_language(loaded.config.ui_language);
    let model = Model::new(loaded.config, Paths { config_file, thumbs_dir, default_save_dir }, util::monitors());
    let app = App::new(model, mtm);
    APP.with(|c| *c.borrow_mut() = Some(app));
    if let Some(w) = loaded.warning {
        emit(Action::Toast(Toast::error(w)));
    }

    // Cắm / rút màn hình, đổi độ phân giải, Dock / menu bar: gộp trong 300 ms.
    let center = NSNotificationCenter::defaultCenter();
    let name = unsafe { objc2_app_kit::NSApplicationDidChangeScreenParametersNotification };
    observe(&center, name, || {
        let t = win::emit_after(300, || Action::DisplayChanged);
        DISPLAY.with(|c| *c.borrow_mut() = Some(t));
    });
    let dist = NSDistributedNotificationCenter::defaultCenter();
    observe(&dist, &NSString::from_str("AppleInterfaceThemeChangedNotification"), || emit(Action::AppearanceChanged));
    observe(&dist, &NSString::from_str(SHOW_SETTINGS), || emit(Action::OpenSettings));

    std::mem::forget(win::after(1000, gfx::warm_up));
    log::info!("sẵn sàng");
    util::log_memory("khởi động xong");
    selftest::start();
    // Lần đầu chạy: để macOS thêm Chup vào danh sách Ghi màn hình (hộp thoại của hệ thống, chỉ lần đầu). Hộp thoại
    // hướng dẫn của Chup chỉ hiện khi người dùng chụp mà vẫn chưa có quyền.
    if !capture::has_permission() && selftest::dir().is_none() {
        let granted = capture::request_permission();
        log::info!("quyền Ghi màn hình: {granted}");
    }
}

pub fn run() -> i32 {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("Chup phải chạy trên main thread");
        return 1;
    };
    // Lệnh dev: OCR một file ảnh bằng Vision rồi in kết quả.
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--ocr-file" {
        let img = match image::open(&args[2]) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        };
        eprintln!("ocr {}×{}...", img.width(), img.height());
        let t0 = std::time::Instant::now();
        match vision::recognize(&img, chup_core::config::OcrLanguage::Vi) {
            Ok(lines) => println!("{} ({:?})", chup_core::ocr::TextLayout::build(lines, 1.0).all_text(), t0.elapsed()),
            Err(e) => eprintln!("lỗi: {e}"),
        }
        return 0;
    }
    let dirs = util::dirs();
    let Some(_lock) = single_instance(&dirs.cache_dir) else {
        let dist = NSDistributedNotificationCenter::defaultCenter();
        unsafe { dist.postNotificationName_object_userInfo_deliverImmediately(&NSString::from_str(SHOW_SETTINGS), None, None, true) };
        return 0;
    };
    logger::init(&dirs.log_dir.join("chup.log"));
    log::info!("Chup {} khởi động (macOS)", env!("CARGO_PKG_VERSION"));

    let ns_app = NSApplication::sharedApplication(mtm);
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    win::set_drain(drain);
    // Tray (NSStatusItem) và hotkey dựng sau khi vòng lặp chạy.
    let started: Retained<NSOperationQueue> = NSOperationQueue::mainQueue();
    let block = RcBlock::new(move || init(mtm));
    unsafe { started.addOperationWithBlock(&block) };
    ns_app.run();
    0
}
