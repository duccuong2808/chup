//! Tự cập nhật trên macOS (SPEC 18): tải `Chup-<v>.dmg` (kiểm tra SHA-256), mount, chép `Chup.app` mới cạnh bản đang
//! chạy rồi đổi chỗ, mở lại. Tải bằng `/usr/bin/curl` (có sẵn; file tải về không bị gắn cờ quarantine). Chạy ngoài
//! `.app` (dev) hoặc không ghi được thư mục chứa app thì chỉ mở trang Release.
//!
//! Bản `.dmg` phải ký bằng cùng chứng chỉ với bản đang dùng, quyền Ghi màn hình / Trợ năng mới giữ được (README).

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use chup_core::{
    i18n::{fill, t},
    ocr::pack::Fetch,
    update::{Package, Update, Version},
};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSApplication};
use objc2_foundation::NSString;

use super::{app::Action, update, util, win::emit};

/// Tải HTTPS bằng curl (theo redirect của GitHub Release, proxy hệ thống qua biến môi trường).
struct Curl;

impl Fetch for Curl {
    fn get(&mut self, url: &str, on_chunk: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), String> {
        let agent = format!("Chup/{}", env!("CARGO_PKG_VERSION"));
        let mut child = Command::new("/usr/bin/curl")
            .args(["-fsSL", "--proto", "=https", "--connect-timeout", "15", "--max-time", "900", "-A", &agent, url])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("chạy curl: {e}"))?;
        let mut out = child.stdout.take().ok_or("curl: không có stdout")?;
        let mut buf = vec![0u8; 64 << 10];
        loop {
            let n = out.read(&mut buf).map_err(|e| format!("đọc curl: {e}"))?;
            if n == 0 {
                break;
            }
            if !on_chunk(&buf[..n]) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("dừng giữa chừng".into());
            }
        }
        let st = child.wait().map_err(|e| e.to_string())?;
        if st.success() {
            return Ok(());
        }
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        Err(format!("curl {st}: {} ({url})", err.trim()))
    }
}

/// `…/Chup.app` đang chạy.
fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

/// Thay được `Chup.app`: đang chạy từ bundle và ghi được thư mục chứa nó.
fn can_install() -> bool {
    let Some(dir) = bundle().and_then(|b| b.parent().map(Path::to_path_buf)) else { return false };
    let probe = dir.join(format!(".chup-write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn root() -> PathBuf {
    update::root(&util::dirs().cache_dir)
}

/// Hỏi GitHub ở thread nền rồi báo `Action::UpdateChecked`.
pub fn check(manual: bool) {
    let spawned = std::thread::Builder::new().name("update".into()).spawn(move || {
        let r = update::check(&mut Curl, Package::MacDmg, &root());
        emit(Action::UpdateChecked(r, manual));
    });
    if let Err(e) = spawned {
        emit(Action::UpdateChecked(Err(e.to_string()), manual));
    }
}

/// `chup --check-update`: hỏi GitHub ngay trên thread này.
pub fn check_blocking(root: &Path) -> Result<Update, String> {
    update::check(&mut Curl, Package::MacDmg, root)
}

/// Hỏi người dùng (NSAlert modal: gọi ngoài lúc App bận, qua `win::after(0, …)`).
pub fn ask(u: Update) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let t = t();
    let auto = u.pack.is_some() && can_install();
    let cur = Version::current().to_string();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(t.upd_title));
    let text = fill(if auto { t.upd_ask } else { t.upd_ask_page }, &[&u.version.to_string(), &cur]);
    alert.setInformativeText(&NSString::from_str(&text));
    alert.addButtonWithTitle(&NSString::from_str(if auto { t.upd_now } else { t.upd_open_page }));
    alert.addButtonWithTitle(&NSString::from_str(t.upd_later));
    NSApplication::sharedApplication(mtm).activate();
    if alert.runModal() != NSAlertFirstButtonReturn {
        log::info!("update: người dùng để sau bản {}", u.version);
    } else if auto {
        emit(Action::UpdateAccepted(u));
    } else {
        emit(Action::Open(PathBuf::from(u.page)));
    }
}

/// Tải `.dmg` rồi thay `Chup.app` ở thread nền, báo `Action::UpdateReady` (Ok: bundle mới, sẵn sàng mở lại).
pub fn download_and_install(u: Update) {
    let spawned = std::thread::Builder::new().name("update".into()).spawn(move || {
        let r = update::download(&mut Curl, &u, &root()).and_then(|dmg| {
            let app = bundle().ok_or("không chạy từ Chup.app")?;
            replace_bundle(&dmg, &app).map(|()| app)
        });
        emit(Action::UpdateReady(r));
    });
    if let Err(e) = spawned {
        emit(Action::UpdateReady(Err(e.to_string())));
    }
}

fn run(cmd: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let out = Command::new(cmd).args(args).stdin(Stdio::null()).output().map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} {}: {}", out.status, String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Mount `dmg`, chép `Chup.app` trong đó thành `.Chup.app.new` cạnh `app`, kiểm tra chữ ký, rồi đổi chỗ (bản cũ đổi
/// tên trước, lỗi thì trả lại). Bản đang chạy vẫn chạy được sau khi bị đổi tên.
fn replace_bundle(dmg: &Path, app: &Path) -> Result<(), String> {
    let dir = app.parent().ok_or("app không có thư mục cha")?;
    let mnt = dmg.with_extension("mnt");
    let new = dir.join(".Chup.app.new");
    let old = dir.join(".Chup.app.old");
    for p in [&new, &old] {
        let _ = std::fs::remove_dir_all(p);
    }
    let _ = std::fs::create_dir_all(&mnt);
    run(
        "/usr/bin/hdiutil",
        &["attach".as_ref(), "-nobrowse".as_ref(), "-readonly".as_ref(), "-noautoopen".as_ref(), "-mountpoint".as_ref(), mnt.as_os_str(), dmg.as_os_str()],
    )?;
    let copied = run("/usr/bin/ditto", &[mnt.join("Chup.app").as_os_str(), new.as_os_str()]);
    let _ = run("/usr/bin/hdiutil", &["detach".as_ref(), "-quiet".as_ref(), mnt.as_os_str()]);
    copied?;
    if let Err(e) = run("/usr/bin/codesign", &["--verify".as_ref(), "--deep".as_ref(), "--strict".as_ref(), new.as_os_str()]) {
        let _ = std::fs::remove_dir_all(&new);
        return Err(format!("chữ ký bản mới: {e}"));
    }
    std::fs::rename(app, &old).map_err(|e| format!("đổi tên bản cũ: {e}"))?;
    if let Err(e) = std::fs::rename(&new, app) {
        let _ = std::fs::rename(&old, app);
        return Err(format!("đặt bản mới: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    log::info!("update: đã thay {}", app.display());
    Ok(())
}

/// Mở lại `app` sau khi process này thoát (shell nền chờ PID rồi `open`). Gọi xong thì App thoát.
pub fn relaunch(app: &Path) -> Result<(), String> {
    let script = r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; /usr/bin/open "$2""#;
    Command::new("/bin/sh")
        .args(["-c", script, "sh", &std::process::id().to_string()])
        .arg(app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("mở lại Chup: {e}"))
}
