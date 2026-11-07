//! Chụp màn hình bằng ScreenCaptureKit (macOS 14+, thiết kế macOS mục 2.5).
//!
//! - Loại mọi cửa sổ của Chup khỏi ảnh bằng `SCContentFilter(excludingApplications:)` (thay `sharingType`).
//! - Chụp đúng độ phân giải gốc, đổi sang sRGB để ảnh lưu ra giống trên màn hình.
//! - Kết quả về qua completion block ở thread của ScreenCaptureKit: chỉ gọi `done` (gửi Action), không đụng App.
//! - Quyền Screen Recording: `has_permission` / `request_permission`.

use std::sync::{Arc, Mutex};

use block2::RcBlock;
use chup_core::{
    capture::MonitorShot,
    coords::{MonitorInfo, Rect},
    i18n::{fill, t},
};
use image::RgbaImage;
use objc2::{rc::Retained, AnyThread};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCCaptureResolutionType, SCContentFilter, SCDisplay, SCRunningApplication, SCScreenshotManager, SCShareableContent,
    SCStreamConfiguration,
};

/// Đã có quyền Screen Recording.
pub fn has_permission() -> bool {
    objc2_core_graphics::CGPreflightScreenCaptureAccess()
}

/// Xin quyền (lần đầu macOS hiện hộp thoại và thêm Chup vào danh sách trong Cài đặt hệ thống).
pub fn request_permission() -> bool {
    objc2_core_graphics::CGRequestScreenCaptureAccess()
}

/// Mở trang Ghi màn hình của Cài đặt hệ thống.
pub fn open_permission_settings() {
    let url = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
    let _ = std::process::Command::new("/usr/bin/open").arg(url).status();
}

fn err(e: impl std::fmt::Display) -> String {
    fill(t().mac_err_capture, &[&e])
}

fn ns_err(e: *mut NSError) -> String {
    match unsafe { e.as_ref() } {
        Some(e) => err(e.localizedDescription()),
        None => err("?"),
    }
}

/// CGImage (BGRA hoặc bất kỳ) -> RGBA thẳng, sRGB.
pub fn to_rgba(img: &CGImage) -> Option<RgbaImage> {
    let (w, h) = (CGImage::width(Some(img)), CGImage::height(Some(img)));
    if w == 0 || h == 0 {
        return None;
    }
    let mut buf = vec![0u8; w * h * 4];
    let cs = CGColorSpace::with_name(Some(unsafe { objc2_core_graphics::kCGColorSpaceSRGB }))?;
    let info = CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0;
    let ctx = unsafe { CGBitmapContextCreate(buf.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&cs), info) }?;
    CGContext::draw_image(Some(&ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64)), Some(img));
    drop(ctx);
    // Ảnh màn hình đục hoàn toàn; phòng trường hợp có alpha thì bỏ premultiply.
    for p in buf.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a != 255 {
            for c in &mut p[..3] {
                *c = (*c as u32 * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
            }
        }
    }
    RgbaImage::from_raw(w as u32, h as u32, buf)
}

/// Lấy danh sách nội dung chụp được rồi gọi `f` (ở thread của ScreenCaptureKit).
fn with_content(f: impl FnOnce(Result<Retained<SCShareableContent>, String>) + Send + 'static) {
    let f = Mutex::new(Some(f));
    let block = RcBlock::new(move |content: *mut SCShareableContent, e: *mut NSError| {
        let Some(f) = f.lock().unwrap_or_else(|p| p.into_inner()).take() else { return };
        match unsafe { Retained::retain(content) } {
            Some(c) => f(Ok(c)),
            None => f(Err(ns_err(e))),
        }
    });
    unsafe { SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(false, true, &block) };
}

/// Bộ lọc: màn hình `display`, bỏ mọi cửa sổ của Chup.
fn filter_for(content: &SCShareableContent, display_id: u32) -> Option<Retained<SCContentFilter>> {
    let displays = unsafe { content.displays() };
    let display: Retained<SCDisplay> = displays.iter().find(|d| unsafe { d.displayID() } == display_id)?;
    let me = std::process::id() as i32;
    let apps: Vec<Retained<SCRunningApplication>> =
        unsafe { content.applications() }.iter().filter(|a| unsafe { a.processID() } == me).collect();
    let apps = NSArray::from_retained_slice(&apps);
    let none = NSArray::new();
    Some(unsafe {
        SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(SCContentFilter::alloc(), &display, &apps, &none)
    })
}

/// Chụp `local` (px trong màn hình `mon`) của màn hình có `display_id`.
fn shoot(
    content: &SCShareableContent,
    display_id: u32,
    mon: &MonitorInfo,
    local: Rect,
    done: impl FnOnce(Result<RgbaImage, String>) + Send + 'static,
) {
    let Some(filter) = filter_for(content, display_id) else {
        done(Err(err(t().monitor_gone_choose_again)));
        return;
    };
    let k = mon.scale;
    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setWidth(local.w as usize);
        config.setHeight(local.h as usize);
        config.setSourceRect(CGRect::new(
            CGPoint::new(local.x as f64 / k, local.y as f64 / k),
            CGSize::new(local.w as f64 / k, local.h as f64 / k),
        ));
        config.setShowsCursor(false);
        config.setCaptureResolution(SCCaptureResolutionType::Best);
        config.setColorSpaceName(objc2_core_graphics::kCGColorSpaceSRGB);
        config.setIgnoreShadowsDisplay(true);
    }
    let done = Mutex::new(Some(done));
    let block = RcBlock::new(move |img: *mut CGImage, e: *mut NSError| {
        let Some(done) = done.lock().unwrap_or_else(|p| p.into_inner()).take() else { return };
        match unsafe { img.as_ref() } {
            Some(img) => done(to_rgba(img).ok_or_else(|| err("CGImage"))),
            None => done(Err(ns_err(e))),
        }
    });
    unsafe { SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&filter, &config, Some(&block)) };
}

/// Chụp vùng `rect` (px toàn desktop, trong một màn hình). Gọi từ main thread; `done` chạy ở thread khác.
pub fn grab_region(rect: Rect, done: impl FnOnce(Result<RgbaImage, String>) + Send + 'static) {
    let Some((id, mon)) = super::util::display_for(rect) else {
        done(Err(t().monitor_gone_choose_again.to_string()));
        return;
    };
    let Some(local) = rect.intersect(&mon.rect).map(|r| r.offset(-mon.rect.x, -mon.rect.y)) else {
        done(Err(t().monitor_gone_choose_again.to_string()));
        return;
    };
    with_content(move |c| match c {
        Ok(c) => shoot(&c, id, &mon, local, done),
        Err(e) => done(Err(e)),
    });
}

/// Chụp mọi màn hình (Freeze). Gọi từ main thread; `done` chạy ở thread khác khi đủ ảnh.
pub fn grab_monitors(done: impl FnOnce(Result<Vec<MonitorShot>, String>) + Send + 'static) {
    let displays = super::util::displays();
    if displays.is_empty() {
        done(Err(t().capture_failed.to_string()));
        return;
    }
    with_content(move |c| {
        let content = match c {
            Ok(c) => c,
            Err(e) => return done(Err(e)),
        };
        let n = displays.len();
        let shots: Arc<Mutex<Vec<Option<MonitorShot>>>> = Arc::new(Mutex::new(vec![None; n]));
        let done = Arc::new(Mutex::new(Some(done)));
        for (i, (id, mon)) in displays.into_iter().enumerate() {
            let (shots, done) = (shots.clone(), done.clone());
            let full = Rect::new(0, 0, mon.rect.w, mon.rect.h);
            let rect = mon.rect;
            shoot(&content, id, &mon, full, move |r| {
                let mut s = shots.lock().unwrap_or_else(|p| p.into_inner());
                s[i] = Some((rect, r));
                if s.iter().all(Option::is_some) {
                    let all = s.drain(..).flatten().collect();
                    if let Some(d) = done.lock().unwrap_or_else(|p| p.into_inner()).take() {
                        d(Ok(all));
                    }
                }
            });
        }
    });
}
