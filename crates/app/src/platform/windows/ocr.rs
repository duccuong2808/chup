//! Worker OCR (SPEC 15.4): 1 thread giữ 2 engine.
//! - Tiếng Anh: `Windows.Media.Ocr` (WinRT) ngay trong process.
//! - Tiếng Việt + Anh: exe phụ `chup-ocr.exe` (Tesseract), nói chuyện qua pipe (`chup-ocr-proto`). Exe phụ lấy ở
//!   cạnh `chup.exe` (bộ cài Full), không có hoặc là bản cũ thì dùng bản đã tải (gói `engine`, `ocr::pack`).
//!
//! Exe phụ được giữ sống giữa các lần OCR và bị kill sau `IDLE` không dùng (trả RAM). Huỷ / thay job tiếng Việt,
//! hoặc job quá thời gian, là kill process; lần sau chạy lại. Process gắn Job Object `KILL_ON_JOB_CLOSE` nên Chup chết
//! thì nó chết theo. Kết quả về UI thread bằng `emit(Action::OcrDone)`.

use chup_core::i18n::t;
use std::{
    io::{BufReader, BufWriter},
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use chup_core::{
    config::OcrLanguage,
    model::OcrViState,
    ocr::{self, pack, Line, LineWord, TextLayout},
};
use chup_ocr_proto::{read_json, write_frame, Hello, Request, Response, PROTO};
use image::{GrayImage, RgbaImage};
use windows::{
    core::HSTRING,
    Globalization::Language,
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Security::Cryptography::CryptographicBuffer,
    Win32::{
        Foundation::HANDLE,
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            WinRT::{RoInitialize, RO_INIT_MULTITHREADED},
        },
    },
};

use super::{app::Action, win::emit};

/// Không OCR trong khoảng này thì tắt exe phụ (giữ ~40 MB khi rảnh). Đo 2026-11-03 (chup-3p3): lúc đang dùng, các lần
/// OCR cách nhau < 1 phút, nên 2 phút vẫn giữ engine nóng mà trả RAM sớm hơn 3 phút so với 5 phút trước đây.
const IDLE: Duration = Duration::from_secs(2 * 60);
/// Một job (kể cả nạp model lần đầu) lâu hơn thì coi như treo (cộng thêm theo cỡ ảnh, xem `job_timeout`).
const JOB_TIMEOUT: Duration = Duration::from_secs(15);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Thư mục chứa gói dữ liệu OCR (`%LOCALAPPDATA%\app.chup.desktop\ocr`).
pub fn ocr_root(cache_dir: &Path) -> PathBuf {
    cache_dir.join("ocr")
}

/// `chup-ocr.exe` cạnh `chup.exe` (bộ cài Full đặt vào), nếu có.
fn installed_helper() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(pack::ENGINE_EXE)).filter(|p| p.is_file())
}

/// `chup-ocr.exe` đã tải (gói `engine`), nếu có đủ.
fn downloaded_helper(ocr_root: &Path) -> Option<PathBuf> {
    let e = pack::engine();
    e.is_installed(ocr_root).then(|| e.dir(ocr_root).join(pack::ENGINE_EXE))
}

/// Engine đang dùng được, cho trang Giới thiệu.
#[derive(Clone)]
pub enum EngineInfo {
    /// Cạnh `chup.exe` (bộ cài Full).
    Installed(PathBuf),
    /// Đã tải về.
    Downloaded(PathBuf),
    Missing,
}

pub fn engine_info(ocr_root: &Path) -> EngineInfo {
    if let Some(p) = installed_helper() {
        EngineInfo::Installed(p)
    } else if let Some(p) = downloaded_helper(ocr_root) {
        EngineInfo::Downloaded(p)
    } else {
        EngineInfo::Missing
    }
}

/// Các gói còn thiếu để OCR tiếng Việt chạy được: engine (khi không có bản cạnh `chup.exe`) và dữ liệu.
pub fn missing_packs(ocr_root: &Path) -> Vec<&'static pack::Pack> {
    let mut v = Vec::new();
    if installed_helper().is_none() && downloaded_helper(ocr_root).is_none() {
        v.push(pack::engine());
    }
    if !pack::data().is_installed(ocr_root) {
        v.push(pack::data());
    }
    v
}

/// Tổng byte cần tải (0 = đủ cả).
pub fn download_size(ocr_root: &Path) -> u64 {
    missing_packs(ocr_root).iter().map(|p| p.total_size()).sum()
}

/// OCR tiếng Việt dùng được chưa (kiểm tra file, rẻ).
pub fn vi_state(ocr_root: &Path) -> OcrViState {
    if missing_packs(ocr_root).is_empty() {
        OcrViState::Ready
    } else {
        OcrViState::NoPack
    }
}

pub struct Job {
    pub id: u32,
    pub lang: OcrLanguage,
    pub scale: f64,
    pub img: RgbaImage,
    /// Job nền cho Preview: huỷ thì để chạy xong rồi bỏ kết quả, không kill exe phụ (khỏi nạp lại model).
    pub background: bool,
}

/// State chia sẻ giữa worker và UI thread (một mutex + condvar).
#[derive(Default)]
struct Shared {
    /// Job chờ chạy (chỉ giữ cái mới nhất).
    pending: Option<Job>,
    /// Job worker đang chạy: chỉ worker đặt và xoá. Watchdog dựa vào đây.
    /// (id, ngôn ngữ, job nền).
    current: Option<(u32, OcrLanguage, bool)>,
    /// UI thread huỷ (hoặc thay) job đang chạy: worker bỏ kết quả.
    cancelled: bool,
    /// Exe phụ đang chạy, để UI thread / watchdog kill được trong lúc worker đang chờ đọc pipe.
    child: Option<Child>,
}

impl Shared {
    fn kill(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Dừng job đang chạy: đánh dấu huỷ; nếu là Tesseract thì kill exe phụ (Windows OCR nhanh, để chạy xong).
    /// Job nền thì không kill: người dùng không chờ nó, còn nạp lại model tốn thêm ~0,6 s cho lần OCR sau.
    fn stop_current(&mut self) {
        if let Some((_, lang, background)) = self.current {
            self.cancelled = true;
            if lang == OcrLanguage::Vi && !background {
                self.kill();
            }
        }
    }
}

type SharedRef = Arc<(Mutex<Shared>, Condvar)>;

fn lock(sh: &SharedRef) -> std::sync::MutexGuard<'_, Shared> {
    sh.0.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Worker {
    shared: SharedRef,
}

impl Worker {
    pub fn start(ocr_root: PathBuf) -> Worker {
        let shared: SharedRef = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("ocr".into())
            .spawn(move || run(sh, ocr_root))
            .expect("tạo thread OCR");
        Worker { shared }
    }

    /// Giao job mới: thay job đang chờ, và dừng job đang chạy (kết quả của nó Model cũng bỏ qua).
    pub fn submit(&self, job: Job) {
        let mut sh = lock(&self.shared);
        sh.stop_current();
        sh.pending = Some(job);
        self.shared.1.notify_one();
    }

    /// Huỷ: bỏ job đang chờ và dừng job đang chạy.
    pub fn cancel(&self) {
        let mut sh = lock(&self.shared);
        sh.pending = None;
        sh.stop_current();
    }

    /// Huỷ mọi job và tắt exe phụ (trước khi xoá gói: exe phụ chạy với thư mục hiện tại là thư mục gói,
    /// Windows không cho xoá thư mục đó khi nó còn sống).
    pub fn stop_helper(&self) {
        let mut sh = lock(&self.shared);
        sh.pending = None;
        sh.stop_current();
        sh.kill();
    }
}

/// Thời gian tối đa cho một job: 15 s + 2 s mỗi megapixel ảnh gửi đi (ảnh to chữ dày cần lâu hơn).
fn job_timeout(gray: &GrayImage) -> Duration {
    let mp = (gray.width() as u64 * gray.height() as u64) / 1_000_000;
    JOB_TIMEOUT + Duration::from_secs(2 * mp)
}

fn run(shared: SharedRef, ocr_root: PathBuf) {
    // WinRT (Windows.Media.Ocr) cần apartment trên thread này.
    if let Err(e) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        log::warn!("ocr: RoInitialize: {e}");
    }
    let mut tess: Option<Tess> = None;
    let mut win: Option<OcrEngine> = None;
    loop {
        let job = {
            let mut sh = lock(&shared);
            loop {
                if let Some(j) = sh.pending.take() {
                    sh.current = Some((j.id, j.lang, j.background));
                    sh.cancelled = false;
                    break j;
                }
                let (g, t) = shared.1.wait_timeout(sh, IDLE).unwrap_or_else(|e| e.into_inner());
                sh = g;
                if t.timed_out() && sh.pending.is_none() && sh.child.is_some() {
                    sh.kill();
                    tess = None;
                    log::info!("ocr: tắt chup-ocr.exe sau {} phút không dùng", IDLE.as_secs() / 60);
                }
            }
        };
        let (id, lang) = (job.id, job.lang);
        let t0 = Instant::now();
        let (gray, factor) = ocr::prepare(&job.img, job.scale, lang);
        drop(job.img);
        let lines = if lock(&shared).cancelled {
            Err(err("", "đã huỷ trước khi nhận diện"))
        } else {
            match lang {
                OcrLanguage::En => recognize_windows(&mut win, &gray),
                OcrLanguage::Vi => recognize_tesseract(&mut tess, &shared, &ocr_root, id, gray),
            }
        };
        let cancelled = {
            let mut sh = lock(&shared);
            sh.current = None;
            std::mem::take(&mut sh.cancelled)
        };
        log::info!("ocr: job {id} ({lang:?}) mất {:?}{}", t0.elapsed(), if cancelled { ", đã huỷ" } else { "" });
        if cancelled {
            continue;
        }
        // Dựng bố cục chữ ở đây (không phải UI thread); bbox đổi về toạ độ ảnh gốc.
        let result = lines.map(|l| TextLayout::build(l, factor)).map_err(|e| {
            log::error!("ocr: {}", e.log);
            e.user
        });
        emit(Action::OcrDone(id, result));
    }
}

/// Lỗi: câu cho người dùng + chi tiết ghi log.
struct OcrError {
    user: String,
    log: String,
}

fn err(user: &str, log: impl Into<String>) -> OcrError {
    OcrError { user: user.into(), log: log.into() }
}

// ---------- Windows OCR (tiếng Anh) ----------

fn windows_engine() -> Result<OcrEngine, OcrError> {
    let en = Language::CreateLanguage(&HSTRING::from("en-US")).and_then(|l| OcrEngine::TryCreateFromLanguage(&l));
    en.or_else(|_| OcrEngine::TryCreateFromUserProfileLanguages()).map_err(|e| {
        err(t().ocr_no_windows_en, format!("OcrEngine: {e}"))
    })
}

fn recognize_windows(engine: &mut Option<OcrEngine>, gray: &GrayImage) -> Result<Vec<Line>, OcrError> {
    let fail = |e: windows::core::Error| err(t().ocr_failed_retry, format!("Windows OCR: {e}"));
    if engine.is_none() {
        *engine = Some(windows_engine()?);
    }
    let engine = engine.as_ref().expect("vừa tạo");
    // Ảnh quá lớn so với giới hạn của Windows OCR thì thu nhỏ; bbox nhân `back` để về lại ảnh `gray`.
    let max = OcrEngine::MaxImageDimension().unwrap_or(2600).max(1);
    let (w, h) = gray.dimensions();
    let scaled;
    let mut back = 1.0f32;
    let img = if w.max(h) > max {
        let k = max as f64 / w.max(h) as f64;
        back = (1.0 / k) as f32;
        let (nw, nh) = (((w as f64 * k) as u32).max(1), ((h as f64 * k) as u32).max(1));
        scaled = image::imageops::resize(gray, nw, nh, image::imageops::FilterType::Triangle);
        &scaled
    } else {
        gray
    };
    let buf = CryptographicBuffer::CreateFromByteArray(img.as_raw()).map_err(fail)?;
    let bmp = SoftwareBitmap::CreateCopyFromBuffer(&buf, BitmapPixelFormat::Gray8, img.width() as i32, img.height() as i32)
        .map_err(fail)?;
    let result = match engine.RecognizeAsync(&bmp).and_then(|op| op.join()) {
        Ok(r) => r,
        // Phòng khi OcrEngine không nhận Gray8 (mẫu của Microsoft dùng Bgra8): đổi định dạng rồi thử lại.
        Err(e) => {
            log::warn!("ocr: Windows OCR với Gray8 lỗi ({e}), thử Bgra8");
            let bgra = SoftwareBitmap::Convert(&bmp, BitmapPixelFormat::Bgra8).map_err(fail)?;
            engine.RecognizeAsync(&bgra).and_then(|op| op.join()).map_err(fail)?
        }
    };
    let lines = result.Lines().map_err(fail)?;
    let mut out = vec![];
    for i in 0..lines.Size().map_err(fail)? {
        let line = lines.GetAt(i).map_err(fail)?;
        let text = line.Text().map_err(fail)?.to_string_lossy();
        let words = line.Words().map_err(fail)?;
        let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut ws = vec![];
        let bbox = |x: f32, y: f32, w: f32, h: f32| {
            [(x * back).round() as i32, (y * back).round() as i32, (w * back).round() as i32, (h * back).round() as i32]
        };
        for j in 0..words.Size().map_err(fail)? {
            let word = words.GetAt(j).map_err(fail)?;
            let rc = word.BoundingRect().map_err(fail)?;
            (l, t, r, b) = (l.min(rc.X), t.min(rc.Y), r.max(rc.X + rc.Width), b.max(rc.Y + rc.Height));
            let text = word.Text().map_err(fail)?.to_string_lossy();
            // Windows OCR không có confidence.
            // Không có vị trí ký tự: `TextLayout` ước lượng theo độ rộng chữ.
            ws.push(LineWord { text, bbox: bbox(rc.X, rc.Y, rc.Width, rc.Height), conf: 100.0, chars: vec![] });
        }
        if l > r {
            continue;
        }
        out.push(Line { text, bbox: bbox(l, t, r - l, b - t), conf: 100.0, words: ws });
    }
    Ok(out)
}

// ---------- Tesseract qua chup-ocr.exe (tiếng Việt + Anh) ----------

/// Kết nối tới exe phụ đang chạy. `Child` nằm trong `Shared` để UI thread kill được.
struct Tess {
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

/// Job Object dùng chung cho mọi lần chạy exe phụ. Không bao giờ đóng: Chup thoát thì Windows đóng handle
/// và kill exe phụ.
fn kill_on_close_job() -> Option<HANDLE> {
    static JOB: std::sync::OnceLock<Option<isize>> = std::sync::OnceLock::new();
    let raw = *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, None).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .ok()?;
        Some(job.0 as isize)
    });
    raw.map(|h| HANDLE(h as *mut _))
}

/// Kill exe phụ nếu job `id` vẫn đang chạy sau `timeout`.
fn watchdog(shared: &SharedRef, id: u32, timeout: Duration) {
    let sh = shared.clone();
    std::thread::spawn(move || {
        std::thread::sleep(timeout);
        let mut s = lock(&sh);
        if s.current.is_some_and(|(cur, _, _)| cur == id) && s.child.is_some() {
            log::warn!("ocr: job {id} quá {}s, kill chup-ocr.exe", timeout.as_secs());
            s.kill();
        }
    });
}

/// Chạy exe phụ: bản cạnh `chup.exe` trước; không có, hoặc là bản cũ (khác PROTO, do chỉ thay `chup.exe`),
/// thì chạy bản đã tải.
fn start_tess(shared: &SharedRef, ocr_root: &Path) -> Result<Tess, OcrError> {
    let downloaded = downloaded_helper(ocr_root);
    if let Some(exe) = installed_helper() {
        match spawn_tess(shared, ocr_root, &exe) {
            Err(StartError::Proto(e)) if downloaded.is_some() => log::warn!("ocr: {}, dùng bản đã tải", e.log),
            r => return r.map_err(StartError::into_ocr),
        }
    }
    let exe = downloaded.ok_or_else(|| err(t().ocr_engine_missing, "không thấy chup-ocr.exe"))?;
    spawn_tess(shared, ocr_root, &exe).map_err(StartError::into_ocr)
}


enum StartError {
    /// Exe phụ khác PROTO: thử bản khác được.
    Proto(OcrError),
    Other(OcrError),
}

impl StartError {
    fn into_ocr(self) -> OcrError {
        match self {
            StartError::Proto(e) | StartError::Other(e) => e,
        }
    }
}

fn spawn_tess(shared: &SharedRef, ocr_root: &Path, exe: &Path) -> Result<Tess, StartError> {
    // Tesseract mở traineddata bằng fopen (ANSI): đường dẫn có dấu (tên user tiếng Việt) sẽ hỏng. Chạy với thư mục
    // hiện tại là thư mục gói (CreateProcessW đặt bằng Unicode) và truyền đường dẫn tương đối ".".
    let mut child = Command::new(exe)
        .current_dir(pack::data().dir(ocr_root))
        .arg("--tessdata")
        .arg(".")
        .arg("--langs")
        .arg(pack::TESSERACT_LANGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| {
            StartError::Other(err(
                t().ocr_engine_blocked,
                format!("chạy {}: {e}", exe.display()),
            ))
        })?;
    if let Some(job) = kill_on_close_job() {
        if let Err(e) = unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())) } {
            log::warn!("ocr: AssignProcessToJobObject: {e}");
        }
    }
    let stdin = BufWriter::new(child.stdin.take().expect("stdin piped"));
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
    lock(shared).child = Some(child);
    let hello: Hello = match read_json(&mut stdout) {
        Ok(Some(h)) => h,
        // Exe phụ thoát ngay (exit 2): thường do dữ liệu tessdata hỏng.
        other => {
            lock(shared).kill();
            return Err(StartError::Other(err(
                t().ocr_pack_broken,
                format!("chup-ocr.exe không chào: {:?}", other.err()),
            )));
        }
    };
    if hello.proto != PROTO {
        lock(shared).kill();
        return Err(StartError::Proto(err(
            t().ocr_engine_mismatch,
            format!("{} proto {} khác {PROTO}", exe.display(), hello.proto),
        )));
    }
    log::info!("ocr: {} sẵn sàng (tesseract {})", exe.display(), hello.tesseract);
    Ok(Tess { stdin, stdout })
}

fn recognize_tesseract(
    tess: &mut Option<Tess>,
    shared: &SharedRef,
    ocr_root: &Path,
    id: u32,
    gray: GrayImage,
) -> Result<Vec<Line>, OcrError> {
    watchdog(shared, id, job_timeout(&gray));
    // Exe phụ đã bị kill (huỷ, timeout) từ lần trước thì chạy lại.
    if lock(shared).child.is_none() {
        *tess = None;
    }
    if tess.is_none() {
        *tess = Some(start_tess(shared, ocr_root)?);
    }
    // Bị huỷ trong lúc chuẩn bị ảnh / nạp model: không gửi ảnh nữa.
    if lock(shared).cancelled {
        return Err(err("", "đã huỷ"));
    }
    let conn = tess.as_mut().expect("vừa tạo");
    let (width, height) = gray.dimensions();
    let req = Request { id, width, height, pixels: gray.into_raw() };
    let io = |e: std::io::Error| err(t().ocr_failed_retry, format!("pipe chup-ocr.exe: {e}"));
    let resp = write_frame(&mut conn.stdin, &req.encode())
        .and_then(|()| read_json::<Response>(&mut conn.stdout))
        .map_err(io)
        .and_then(|r| r.ok_or_else(|| err(t().ocr_failed_retry, "chup-ocr.exe đóng pipe")));
    match resp {
        Ok(Response::Ok { id: rid, lines }) if rid == id => Ok(lines),
        Ok(Response::Err { error, .. }) => Err(err(t().ocr_failed_retry, format!("chup-ocr.exe: {error}"))),
        Ok(other) => {
            *tess = None;
            lock(shared).kill();
            Err(err(t().ocr_failed_retry, format!("trả lời lệch job: {} != {id}", other.id())))
        }
        Err(e) => {
            // Pipe hỏng (crash, bị kill): bỏ kết nối, lần sau chạy lại.
            *tess = None;
            lock(shared).kill();
            Err(e)
        }
    }
}
