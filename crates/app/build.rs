//! Windows: nhúng manifest (DPI Per-Monitor v2, Common Controls v6) và icon của file exe.
//! Cho trang Giới thiệu:
//! - `CHUP_COMMIT`: 7 ký tự đầu của commit. Lấy theo thứ tự: `GITHUB_SHA` (CI); `CHUP_GIT_SHA` (build tay từ bản
//!   chép không có `.git`, người build tự đặt); hỏi git (thêm "+" nếu có thay đổi chưa commit); không có thì rỗng.
//! - `CHUP_LOCAL_BUILD`: "1" nếu không build trên CI.
//! - `CHUP_BUILD_TIME`: thời điểm build, "YYYY-MM-DD HH:MM" theo UTC.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=chup.rc");
    // Thời điểm build chỉ cập nhật khi build script chạy lại: chạy lại mỗi khi mã nguồn app/core đổi.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../core/src");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=CHUP_GIT_SHA");

    let env = |k: &str| std::env::var(k).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let ci = env("GITHUB_SHA");
    let commit = match &ci {
        Some(sha) => sha.chars().take(7).collect(),
        None => env("CHUP_GIT_SHA").or_else(git_head).unwrap_or_default(),
    };
    println!("cargo:rustc-env=CHUP_COMMIT={commit}");
    println!("cargo:rustc-env=CHUP_LOCAL_BUILD={}", if ci.is_some() { "" } else { "1" });
    println!("cargo:rustc-env=CHUP_BUILD_TIME={}", utc_now());

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // embed-manifest tự sinh object file, không cần rc.exe: check được cả khi cross-compile từ macOS.
    use embed_manifest::manifest::{ActiveCodePage, DpiAwareness, Setting, SupportedOS::*};
    let manifest = embed_manifest::new_manifest("Chup")
        .dpi_awareness(DpiAwareness::PerMonitorV2)
        .active_code_page(ActiveCodePage::Utf8)
        .long_path_aware(Setting::Enabled)
        .supported_os(Windows10..=Windows10);
    embed_manifest::embed_manifest(manifest).expect("không nhúng được manifest");

    // Icon của exe cần resource compiler (rc.exe trên Windows). Máy khác (cross-check) thì bỏ qua.
    if cfg!(windows) {
        let _ = embed_resource::compile("chup.rc", embed_resource::NONE);
    }
}

/// Commit HEAD của git (build tay), 7 ký tự, thêm "+" nếu file đã theo dõi có thay đổi chưa commit.
/// Đăng ký rerun theo HEAD và ref của nhánh để commit mới thì cập nhật.
fn git_head() -> Option<String> {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").args(args).output().ok()?;
        let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (out.status.success() && !s.is_empty()).then_some(s)
    };
    let sha = git(&["rev-parse", "--short=7", "HEAD"])?;
    // `git status` thành công mà không in gì = sạch.
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
    let mut watch = vec![git(&["rev-parse", "--git-path", "HEAD"])];
    if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
        watch.push(git(&["rev-parse", "--git-path", &r]));
        watch.push(git(&["rev-parse", "--git-path", "packed-refs"]));
    }
    // File không tồn tại thì Cargo coi là luôn đổi (chạy lại mỗi lần build): chỉ theo dõi file có thật.
    for p in watch.into_iter().flatten().filter(|p| Path::new(p).is_file()) {
        println!("cargo:rerun-if-changed={p}");
    }
    Some(if dirty { sha + "+" } else { sha })
}

/// "YYYY-MM-DD HH:MM" theo UTC, không cần crate ngày giờ (thuật toán civil_from_days của H. Hinnant).
fn utc_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3_600, rem % 3_600 / 60)
}
