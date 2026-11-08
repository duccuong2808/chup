//! Chia sẻ ảnh qua link trên macOS (SPEC 19): menu chọn kiểu (NSMenu), hộp thoại đồng ý (NSAlert), userhash catbox
//! trong Keychain, thread upload gọi `/usr/bin/curl`. Logic ở `chup_core::share` và `Model`.

use std::{
    cell::RefCell,
    io::{Read, Write},
    process::{Command, Stdio},
};

use chup_core::{
    i18n::{fill, t},
    share::{self, HttpRequest, HttpResponse, ShareError, ShareKind, UploadJob},
};
use objc2::{define_class, msg_send, rc::Retained, runtime::AnyObject, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSApplication, NSMenu, NSMenuItem, NSView};
use objc2_foundation::{NSObject, NSPoint, NSString};

use super::{app::Action, win::emit};

/// Mục Keychain (generic password).
const KEYCHAIN_SERVICE: &str = "app.chup.desktop";
const KEYCHAIN_ACCOUNT: &str = "catbox";
/// `errSecItemNotFound`.
const ERR_NOT_FOUND: i32 = -25300;
/// Tag của mục Upload: `TAG_BASE + i`.
const TAG_BASE: isize = 100;

/// Việc làm khi người dùng chọn một mục menu.
type OnPick = Box<dyn Fn(ShareKind)>;

thread_local! {
    /// Các mục menu Upload hiện hành (kiểu + chữ), App đặt lại khi config / ngôn ngữ đổi.
    static MENU: RefCell<Vec<(ShareKind, String)>> = const { RefCell::new(Vec::new()) };
    /// Việc làm khi người dùng chọn một mục (đặt lúc dựng menu).
    static ON_PICK: RefCell<Option<OnPick>> = const { RefCell::new(None) };
    static TARGET: RefCell<Option<Retained<ShareTarget>>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: NSObject không có yêu cầu riêng khi kế thừa; không cài Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ChupShareMenuTarget"]
    pub struct ShareTarget;

    impl ShareTarget {
        #[unsafe(method(onShare:))]
        fn on_share(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let kind = usize::try_from(tag - TAG_BASE).ok().and_then(|i| MENU.with(|m| m.borrow().get(i).map(|(k, _)| *k)));
            // Lấy callback ra trước khi gọi: callback có thể dựng menu mới.
            let cb = ON_PICK.with(|c| c.borrow_mut().take());
            if let (Some(k), Some(cb)) = (kind, cb) {
                cb(k);
            }
        }
    }
);

fn target(mtm: MainThreadMarker) -> Retained<ShareTarget> {
    TARGET.with(|t| {
        t.borrow_mut()
            .get_or_insert_with(|| unsafe { msg_send![ShareTarget::alloc(mtm), init] })
            .clone()
    })
}

pub fn set_menu(items: Vec<(ShareKind, String)>) {
    MENU.with(|m| *m.borrow_mut() = items);
}

/// Thêm các mục Upload vào `menu`; chọn mục nào thì gọi `on_pick` với kiểu đó.
pub fn add_items(menu: &NSMenu, mtm: MainThreadMarker, on_pick: impl Fn(ShareKind) + 'static) {
    ON_PICK.with(|c| *c.borrow_mut() = Some(Box::new(on_pick)));
    let target = target(mtm);
    let items = MENU.with(|m| m.borrow().clone());
    for (i, (_, label)) in items.iter().enumerate() {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(label),
                Some(sel!(onShare:)),
                &NSString::from_str(""),
            )
        };
        unsafe { item.setTarget(Some(target.as_ref())) };
        item.setTag(TAG_BASE + i as isize);
        menu.addItem(&item);
    }
}

/// Menu chọn kiểu tại `at` (point, toạ độ của `view`). Không giữ borrow khi gọi: menu chạy vòng lặp riêng.
pub fn popup(view: &NSView, at: NSPoint, mtm: MainThreadMarker, on_pick: impl Fn(ShareKind) + 'static) {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    add_items(&menu, mtm, on_pick);
    menu.popUpMenuPositioningItem_atLocation_inView(None, at, Some(view));
}

/// Hỏi đồng ý gửi ảnh ra ngoài (gọi ngoài lúc App bận) rồi báo `Action::ShareConsent`.
pub fn ask(kind: ShareKind, instance: String) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let s = t();
    let text = match kind {
        ShareKind::Image => s.share_consent_image.to_string(),
        ShareKind::Private => fill(s.share_consent_private, &[&share::host_of(&instance)]),
        ShareKind::Permanent => s.share_consent_permanent.to_string(),
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(s.share_consent_title));
    alert.setInformativeText(&NSString::from_str(&text));
    alert.addButtonWithTitle(&NSString::from_str(s.share_consent_ok));
    alert.addButtonWithTitle(&NSString::from_str(s.share_consent_cancel));
    NSApplication::sharedApplication(mtm).activate();
    let ok = alert.runModal() == NSAlertFirstButtonReturn;
    emit(Action::ShareConsent(kind, ok));
}

/// POST bằng curl: body qua stdin, không theo redirect, chỉ https; mã HTTP ở dòng cuối (`-w`).
struct Curl;

impl share::Http for Curl {
    fn post(&mut self, req: &HttpRequest) -> Result<HttpResponse, String> {
        let mut cmd = Command::new("/usr/bin/curl");
        cmd.args(["-sS", "--proto", "=https", "--connect-timeout", "15", "--max-time", "120", "-w", "\n%{http_code}"]);
        cmd.args(["-A", &format!("Chup/{}", env!("CARGO_PKG_VERSION")), "--data-binary", "@-"]);
        for (k, v) in &req.headers {
            cmd.arg("-H").arg(format!("{k}: {v}"));
        }
        let mut child = cmd
            .arg(&req.url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("chạy curl: {e}"))?;
        // Ghi body ở thread riêng: body lớn có thể đầy pipe trong lúc curl chờ ta đọc stdout.
        let mut stdin = child.stdin.take().ok_or("curl: không có stdin")?;
        let body = req.body.clone();
        let writer = std::thread::spawn(move || stdin.write_all(&body));
        let mut out = Vec::new();
        child.stdout.take().ok_or("curl: không có stdout")?.read_to_end(&mut out).map_err(|e| e.to_string())?;
        let _ = writer.join();
        let st = child.wait().map_err(|e| e.to_string())?;
        let cut = out.iter().rposition(|&b| b == b'\n');
        let status: u16 = cut.and_then(|c| String::from_utf8_lossy(&out[c + 1..]).trim().parse().ok()).unwrap_or(0);
        if !st.success() || status == 0 {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err);
            }
            return Err(format!("curl {st}: {}", err.trim()));
        }
        out.truncate(cut.unwrap_or(0));
        Ok(HttpResponse { status, body: out })
    }
}

/// Upload ở thread nền rồi báo `Action::UploadDone`.
pub fn start(job: UploadJob) {
    let id = job.id;
    let spawned = std::thread::Builder::new().name("share".into()).spawn(move || {
        let hash = if job.kind == ShareKind::Permanent { userhash() } else { None };
        let r = share::run(&job, &mut Curl, hash.as_deref(), &mut share::system_rng);
        emit(Action::UploadDone(id, r));
    });
    if let Err(e) = spawned {
        emit(Action::UploadDone(id, Err(ShareError::Network(e.to_string()))));
    }
}

// ---------- userhash catbox trong Keychain ----------

/// `CHUP_SELFTEST`: kho giả trong thư mục tự kiểm tra, không đụng Keychain thật.
fn selftest_file() -> Option<std::path::PathBuf> {
    super::selftest::dir().map(|d| d.join("catbox-userhash"))
}

pub fn userhash() -> Option<String> {
    let raw = match selftest_file() {
        Some(f) => std::fs::read(f).ok()?,
        None => security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).ok()?,
    };
    String::from_utf8(raw).ok().filter(|s| !s.is_empty())
}

/// Lưu (`Some`) hoặc xoá (`None`) userhash.
pub fn set_userhash(hash: Option<&str>) -> Result<(), String> {
    use security_framework::passwords::{delete_generic_password, set_generic_password};
    if let Some(f) = selftest_file() {
        return match hash {
            Some(h) => std::fs::write(f, h.trim()),
            None => std::fs::remove_file(f).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }),
        }
        .map_err(|e| e.to_string());
    }
    match hash {
        Some(h) => set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, h.trim().as_bytes()).map_err(|e| e.to_string()),
        None => match delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERR_NOT_FOUND => Ok(()),
            Err(e) => Err(e.to_string()),
        },
    }
}
