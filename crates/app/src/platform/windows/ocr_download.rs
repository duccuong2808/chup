//! Tải những gì OCR tiếng Việt còn thiếu (`core::ocr::pack`): dữ liệu tessdata_fast (GitHub cộng đồng) và, nếu không có
//! `chup-ocr.exe` cạnh `chup.exe` (bộ cài Lite, chạy exe rời), engine từ Release `ocr-engine-vN` của repo Chup.
//!
//! Một đường tải duy nhất cho cả bộ cài (`chup.exe --install-ocr`) và app (nút Cài đặt, câu hỏi khi bấm [T]):
//! TaskDialog có thanh tiến trình và nút Huỷ; lỗi thì hỏi Thử lại. Các hộp thoại ở đây là modal, nên app gọi
//! chúng ngoài lúc `App` đang bị borrow (xem `WM_APP_OCR_DOWNLOAD`).

use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use chup_core::{
    config,
    ocr::pack::{self, Fetch, InstallError, Pack},
};
use windows::{
    core::{w, HRESULT, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, S_FALSE, S_OK, WPARAM},
        Networking::WinHttp::{
            WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
            WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
            WINHTTP_QUERY_STATUS_CODE,
        },
        UI::{
            Controls::{
                TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOG_NOTIFICATIONS, TDCBF_CANCEL_BUTTON, TDE_CONTENT,
                TDF_CALLBACK_TIMER, TDF_SHOW_PROGRESS_BAR, TDM_CLICK_BUTTON, TDM_UPDATE_ELEMENT_TEXT,
                TDM_SET_PROGRESS_BAR_POS, TDM_SET_PROGRESS_BAR_RANGE, TDN_BUTTON_CLICKED, TDN_CREATED, TDN_TIMER,
            },
            WindowsAndMessaging::{
                MessageBoxW, SendMessageW, IDCANCEL, IDRETRY, IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_RETRYCANCEL,
                MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
            },
        },
    },
};

use super::{ocr, util::wide};

const TITLE: PCWSTR = w!("Chup – OCR tiếng Việt");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Installed,
    /// Lỗi và người dùng không thử lại.
    Failed,
    /// Người dùng bấm Huỷ lúc đang tải.
    Cancelled,
    /// Người dùng chọn không tải.
    Declined,
}

/// Đang có hộp thoại tải mở: lần yêu cầu thứ hai bị bỏ qua.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Thread tải còn chạy (kể cả sau khi hộp thoại đã đóng vì Huỷ): không cho lần tải mới đụng vào thư mục tạm.
static FETCHING: AtomicBool = AtomicBool::new(false);

/// Hỏi (nếu `ask`) rồi tải và cài gói vào `ocr_root`.
pub fn run(ocr_root: &Path, ask: bool) -> Option<Outcome> {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return None;
    }
    let out = if ask && !confirm(ocr_root) { Outcome::Declined } else { install_with_retry(ocr_root) };
    RUNNING.store(false, Ordering::Release);
    log::info!("ocr: tải gói tiếng Việt: {out:?}");
    Some(out)
}

/// `chup.exe --install-ocr` (bộ cài Lite gọi): tải những gì còn thiếu, xong thì chọn OCR tiếng Việt.
/// Exit code 0 xong / 1 lỗi / 2 huỷ.
pub fn install_mode(ocr_root: &Path, config_file: &Path, default_save_dir: &Path) -> i32 {
    match install_with_retry(ocr_root) {
        Outcome::Installed => {
            let mut cfg = config::load(config_file, default_save_dir).config;
            cfg.ocr.language = config::OcrLanguage::Vi;
            if let Err(e) = config::save(config_file, &cfg) {
                log::error!("ocr: không lưu được config sau khi tải gói: {e}");
            }
            0
        }
        Outcome::Cancelled | Outcome::Declined => 2,
        Outcome::Failed => 1,
    }
}

fn confirm(ocr_root: &Path) -> bool {
    let text = wide(&format!(
        "OCR tiếng Việt cần tải thêm {} từ GitHub.\n\nTải về bây giờ?",
        pack::size_label(ocr::download_size(ocr_root))
    ));
    let r = unsafe {
        MessageBoxW(None, PCWSTR(text.as_ptr()), TITLE, MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND)
    };
    r == IDYES
}

fn install_with_retry(ocr_root: &Path) -> Outcome {
    loop {
        match download_dialog(ocr_root) {
            Ok(()) => return Outcome::Installed,
            Err(InstallError::Cancelled) => return Outcome::Cancelled,
            Err(e) => {
                log::error!("ocr: tải gói lỗi: {e:?}");
                let text = wide(e.user_message());
                let r = unsafe {
                    MessageBoxW(None, PCWSTR(text.as_ptr()), TITLE, MB_RETRYCANCEL | MB_ICONERROR | MB_TOPMOST)
                };
                if r != IDRETRY {
                    return Outcome::Failed;
                }
            }
        }
    }
}

// ---------- hộp thoại tiến trình ----------

struct Progress {
    done: AtomicU64,
    total: u64,
    cancel: AtomicBool,
    result: Mutex<Option<Result<(), InstallError>>>,
}

const RANGE: u64 = 1000;

fn mb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_048_576.0).replace('.', ",")
}

unsafe extern "system" fn callback(
    hwnd: HWND,
    msg: TASKDIALOG_NOTIFICATIONS,
    _wp: WPARAM,
    _lp: LPARAM,
    data: isize,
) -> HRESULT {
    // SAFETY: `data` là con trỏ tới `Progress` do `download_dialog` giữ sống suốt lúc hộp thoại mở.
    let p = unsafe { &*(data as *const Progress) };
    let finished = || p.result.lock().map(|r| r.is_some()).unwrap_or(true);
    match msg {
        TDN_CREATED => unsafe {
            SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_RANGE.0 as u32, None, Some(LPARAM((RANGE as isize) << 16)));
        },
        TDN_TIMER => {
            let done = p.done.load(Ordering::Relaxed);
            let total = p.total.max(1);
            let text = wide(&format!("Đã tải {} / {} MB", mb(done), mb(total)));
            unsafe {
                SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_POS.0 as u32, Some(WPARAM((done * RANGE / total) as usize)), None);
                // UPDATE (không SET): đổi chữ mà không dàn lại hộp thoại mỗi 200 ms.
                SendMessageW(hwnd, TDM_UPDATE_ELEMENT_TEXT.0 as u32, Some(WPARAM(TDE_CONTENT.0 as usize)), Some(LPARAM(text.as_ptr() as isize)));
                if finished() {
                    // Tải xong (hoặc lỗi): đóng hộp thoại; TDN_BUTTON_CLICKED phân biệt với người dùng bấm Huỷ.
                    SendMessageW(hwnd, TDM_CLICK_BUTTON.0 as u32, Some(WPARAM(IDCANCEL.0 as usize)), None);
                }
            }
        }
        TDN_BUTTON_CLICKED => {
            if !finished() {
                p.cancel.store(true, Ordering::Relaxed);
            }
        }
        _ => return S_FALSE,
    }
    S_OK
}

fn download_dialog(ocr_root: &Path) -> Result<(), InstallError> {
    if FETCHING.swap(true, Ordering::AcqRel) {
        return Err(InstallError::Busy);
    }
    let packs = ocr::missing_packs(ocr_root);
    let total = packs.iter().map(|p| p.total_size()).sum();
    let progress =
        Arc::new(Progress { done: AtomicU64::new(0), total, cancel: AtomicBool::new(false), result: Mutex::new(None) });
    let mut worker = Some({
        let p = progress.clone();
        let root = ocr_root.to_path_buf();
        std::thread::spawn(move || {
            let r = install_all(&packs, &root, &p);
            *p.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            FETCHING.store(false, Ordering::Release);
        })
    });
    let instruction = wide("Đang tải OCR tiếng Việt");
    let content = wide("Đang kết nối…");
    let cfg = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        dwFlags: TDF_SHOW_PROGRESS_BAR | TDF_CALLBACK_TIMER,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: TITLE,
        pszMainInstruction: PCWSTR(instruction.as_ptr()),
        pszContent: PCWSTR(content.as_ptr()),
        pfCallback: Some(callback),
        lpCallbackData: Arc::as_ptr(&progress) as isize,
        ..Default::default()
    };
    if let Err(e) = unsafe { TaskDialogIndirect(&cfg, None, None, None) } {
        // Không hiện được hộp thoại (thiếu Common Controls v6?): vẫn tải, chỉ không có tiến trình.
        log::warn!("ocr: TaskDialogIndirect: {e}");
        if let Some(w) = worker.take() {
            let _ = w.join();
        }
    }
    let result = progress.result.lock().unwrap_or_else(|e| e.into_inner()).take();
    match result {
        Some(r) => {
            if let Some(w) = worker.take() {
                let _ = w.join();
            }
            r
        }
        // Người dùng huỷ: không đợi thread tải (mạng chậm có thể kẹt tới vài chục giây ở bước kết nối, làm đơ cả app).
        // Nó dừng ở đoạn kế tiếp và tự xoá thư mục tạm; `FETCHING` chặn lần tải mới cho tới lúc đó.
        None => Err(InstallError::Cancelled),
    }
}

/// Cài lần lượt các gói, tiến trình cộng dồn.
fn install_all(packs: &[&'static Pack], root: &Path, p: &Progress) -> Result<(), InstallError> {
    let mut before = 0;
    for pack in packs {
        log::info!("ocr: tải gói {} ({})", pack.dir, pack.source);
        pack.install(&mut WinHttp, root, &mut |d| p.done.store(before + d, Ordering::Relaxed), &p.cancel)?;
        before += pack.total_size();
    }
    Ok(())
}

// ---------- WinHTTP ----------

/// Handle WinHTTP tự đóng.
struct Inet(*mut core::ffi::c_void);

impl Drop for Inet {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn non_null(h: *mut core::ffi::c_void, what: &str) -> Result<Inet, String> {
    if h.is_null() {
        Err(format!("{what}: {}", windows::core::Error::from_thread()))
    } else {
        Ok(Inet(h))
    }
}

/// Tải HTTPS bằng WinHTTP (có sẵn trong Windows, dùng proxy hệ thống, tự theo redirect).
struct WinHttp;

impl Fetch for WinHttp {
    fn get(&mut self, url: &str, on_chunk: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), String> {
        let rest = url.strip_prefix("https://").ok_or("chỉ hỗ trợ https")?;
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let (host, path) = (wide(host), wide(if path.is_empty() { "/" } else { path }));
        unsafe {
            let agent = wide(&format!("Chup/{}", env!("CARGO_PKG_VERSION")));
            let session = non_null(
                WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0),
                "WinHttpOpen",
            )?;
            WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 15_000).map_err(|e| e.to_string())?;
            let conn = non_null(
                WinHttpConnect(session.0, PCWSTR(host.as_ptr()), INTERNET_DEFAULT_HTTPS_PORT, 0),
                "WinHttpConnect",
            )?;
            let req = non_null(
                WinHttpOpenRequest(
                    conn.0,
                    w!("GET"),
                    PCWSTR(path.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null(),
                    WINHTTP_FLAG_SECURE,
                ),
                "WinHttpOpenRequest",
            )?;
            WinHttpSendRequest(req.0, None, None, 0, 0, 0).map_err(|e| format!("gửi yêu cầu: {e}"))?;
            WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|e| format!("nhận trả lời: {e}"))?;
            let mut code = 0u32;
            let mut size = 4u32;
            WinHttpQueryHeaders(
                req.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut code as *mut u32 as *mut _),
                &mut size,
                std::ptr::null_mut(),
            )
            .map_err(|e| format!("đọc mã HTTP: {e}"))?;
            if code != 200 {
                return Err(format!("HTTP {code} cho {url}"));
            }
            let mut buf = vec![0u8; 64 << 10];
            loop {
                let mut read = 0u32;
                WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read)
                    .map_err(|e| format!("đọc dữ liệu: {e}"))?;
                if read == 0 {
                    return Ok(());
                }
                if !on_chunk(&buf[..read as usize]) {
                    return Err("dừng giữa chừng".into());
                }
            }
        }
    }
}
