//! Chia sẻ ảnh qua link trên Windows (SPEC 19): menu chọn kiểu, hộp thoại đồng ý lần đầu, kho userhash catbox
//! (Credential Manager), thread upload. Logic ở `chup_core::share` và `Model`.

use std::cell::RefCell;

use chup_core::{
    i18n::{fill, t},
    share::{self, ShareError, ShareKind, UploadJob},
};
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Security::Credentials::{
            CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE,
            CRED_TYPE_GENERIC,
        },
        UI::WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, DestroyMenu, MessageBoxW, PostMessageW, SetForegroundWindow,
            TrackPopupMenu, IDOK, MB_ICONINFORMATION, MB_OKCANCEL, MB_SETFOREGROUND, MB_TOPMOST, MF_STRING,
            TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NULL,
        },
    },
};

use super::{
    app::Action,
    ocr_download::WinHttp,
    util::wide,
    win::{self, emit},
};

/// Tên mục trong Credential Manager.
const CRED_TARGET: &str = "app.chup.desktop/catbox";
/// Id lệnh menu = 1 + chỉ số trong danh sách (0 = không chọn).
const CMD_BASE: usize = 1;

thread_local! {
    /// Các mục menu Upload hiện hành (kiểu + chữ), App đặt lại khi config / ngôn ngữ đổi.
    static MENU: RefCell<Vec<(ShareKind, String)>> = const { RefCell::new(Vec::new()) };
    /// Kiểu đang chờ hỏi đồng ý và instance PrivateBin (xem `WM_APP_SHARE_CONSENT`).
    static ASK: RefCell<Option<(ShareKind, String)>> = const { RefCell::new(None) };
}

pub fn set_menu(items: Vec<(ShareKind, String)>) {
    MENU.with(|m| *m.borrow_mut() = items);
}

/// Các mục menu Upload (stack, Preview thêm vào menu của mình).
pub fn menu_items() -> Vec<(ShareKind, String)> {
    MENU.with(|m| m.borrow().clone())
}

/// Thêm các mục Upload vào `menu` với id `base + i`.
pub fn append_items(menu: windows::Win32::UI::WindowsAndMessaging::HMENU, base: usize) {
    for (i, (_, label)) in menu_items().iter().enumerate() {
        let w = wide(label);
        unsafe {
            let _ = AppendMenuW(menu, MF_STRING, base + i, PCWSTR(w.as_ptr()));
        }
    }
}

/// Kiểu ứng với id lệnh `cmd` của các mục do `append_items(_, base)` thêm.
pub fn kind_of(cmd: usize, base: usize) -> Option<ShareKind> {
    cmd.checked_sub(base).and_then(|i| menu_items().get(i).map(|(k, _)| *k))
}

/// Menu chọn kiểu tại điểm màn hình `(x, y)` (chạy vòng message riêng: không giữ borrow khi gọi).
/// `owner = None`: dùng cửa sổ ẩn của app, như menu tray (stack có `WS_EX_NOACTIVATE`, không làm foreground được,
/// mà menu chỉ tự đóng khi bấm ra ngoài nếu cửa sổ sở hữu nó là foreground).
pub fn pick(owner: Option<HWND>, x: i32, y: i32) -> Option<ShareKind> {
    let owner = owner.unwrap_or_else(win::app_hwnd);
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        append_items(menu, CMD_BASE);
        let _ = SetForegroundWindow(owner);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, None, owner, None);
        let _ = DestroyMenu(menu);
        // Theo tài liệu TrackPopupMenu: gửi một message để lần mở menu sau không bị đóng ngay.
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        kind_of(cmd.0 as usize, CMD_BASE)
    }
}

/// Hỏi đồng ý (hộp thoại modal, chạy ngoài lúc App bận).
pub fn post_consent(kind: ShareKind, instance: String) {
    ASK.with(|c| *c.borrow_mut() = Some((kind, instance)));
    unsafe {
        let _ = PostMessageW(Some(win::app_hwnd()), win::WM_APP_SHARE_CONSENT, WPARAM(0), LPARAM(0));
    }
}

/// Xử lý `WM_APP_SHARE_CONSENT`.
pub fn ask_pending() {
    let Some((kind, instance)) = ASK.with(|c| c.borrow_mut().take()) else { return };
    let s = t();
    let text = match kind {
        ShareKind::Image => s.share_consent_image.to_string(),
        ShareKind::Private => fill(s.share_consent_private, &[&share::host_of(&instance)]),
        ShareKind::Permanent => s.share_consent_permanent.to_string(),
    };
    let (text, title) = (wide(&text), wide(s.share_consent_title));
    let r = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OKCANCEL | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };
    emit(Action::ShareConsent(kind, r == IDOK));
}

/// Upload ở thread nền rồi báo `Action::UploadDone`.
pub fn start(job: UploadJob) {
    let id = job.id;
    let spawned = std::thread::Builder::new().name("share".into()).spawn(move || {
        let hash = if job.kind == ShareKind::Permanent { userhash() } else { None };
        let r = share::run(&job, &mut WinHttp, hash.as_deref(), &mut share::system_rng);
        emit(Action::UploadDone(id, r));
    });
    if let Err(e) = spawned {
        emit(Action::UploadDone(id, Err(ShareError::Network(e.to_string()))));
    }
}

// ---------- userhash catbox trong Credential Manager ----------

pub fn userhash() -> Option<String> {
    let target = wide(CRED_TARGET);
    unsafe {
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut p).ok()?;
        let c = &*p;
        let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
        let s = String::from_utf8(blob.to_vec()).ok();
        CredFree(p as *const _);
        s.filter(|s| !s.is_empty())
    }
}

/// Lưu (`Some`) hoặc xoá (`None`) userhash.
pub fn set_userhash(hash: Option<&str>) -> Result<(), String> {
    let target = wide(CRED_TARGET);
    unsafe {
        match hash {
            None => match CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) {
                Ok(()) => Ok(()),
                // Chưa có mục nào: coi như đã xoá.
                Err(e) if e.code() == windows::Win32::Foundation::ERROR_NOT_FOUND.to_hresult() => Ok(()),
                Err(e) => Err(e.to_string()),
            },
            Some(h) => {
                let mut blob = h.trim().as_bytes().to_vec();
                let mut user = wide("catbox");
                let mut target = target;
                let cred = CREDENTIALW {
                    Flags: CRED_FLAGS(0),
                    Type: CRED_TYPE_GENERIC,
                    TargetName: PWSTR(target.as_mut_ptr()),
                    CredentialBlobSize: blob.len() as u32,
                    CredentialBlob: blob.as_mut_ptr(),
                    Persist: CRED_PERSIST_LOCAL_MACHINE,
                    UserName: PWSTR(user.as_mut_ptr()),
                    ..Default::default()
                };
                CredWriteW(&cred, 0).map_err(|e| e.to_string())
            }
        }
    }
}
