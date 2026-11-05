//! OCR bằng Apple Vision (SPEC 15, thiết kế macOS mục 3): có sẵn trong macOS, có tiếng Việt (macOS 26), không tải
//! gói nào. Chạy ở worker thread trong process (không có exe phụ). Vision cho vị trí từng ký tự thật nên chọn chữ
//! trên Preview (Live Text) chính xác hơn Windows OCR (phải ước lượng).
//!
//! Kết quả đổi sang `chup_core::ocr::Line` rồi `TextLayout::build` như hai engine của Windows.

use std::sync::{
    atomic::{AtomicU32, Ordering},
    mpsc::{channel, Receiver, Sender},
    Arc,
};

use chup_core::{
    config::OcrLanguage,
    i18n::t,
    ocr::{Line, LineWord, TextLayout},
};
use image::RgbaImage;
use objc2::{msg_send, rc::Retained, AnyThread};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSRange, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedText, VNRectangleObservation, VNRequest,
    VNRequestTextRecognitionLevel,
};

use super::{app::Action, win::emit};

pub struct Job {
    pub id: u32,
    pub lang: OcrLanguage,
    pub img: RgbaImage,
}

pub struct Worker {
    tx: Sender<Job>,
    /// Job bị huỷ: bỏ kết quả có id ≤ giá trị này.
    cancelled: Arc<AtomicU32>,
}

impl Worker {
    pub fn start() -> Worker {
        let (tx, rx) = channel::<Job>();
        let cancelled = Arc::new(AtomicU32::new(0));
        let c = cancelled.clone();
        std::thread::Builder::new().name("ocr".into()).spawn(move || run(rx, c)).ok();
        Worker { tx, cancelled }
    }

    /// Job mới thay job đang chờ (job đang chạy vẫn chạy nốt, kết quả bị bỏ nếu đã huỷ).
    pub fn submit(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    /// Huỷ job đang chạy / đang chờ: không báo kết quả.
    pub fn cancel(&self) {
        self.cancelled.store(u32::MAX, Ordering::Release);
    }
}

fn run(rx: Receiver<Job>, cancelled: Arc<AtomicU32>) {
    while let Ok(mut job) = rx.recv() {
        // Chỉ làm job mới nhất.
        while let Ok(j) = rx.try_recv() {
            job = j;
        }
        cancelled.store(0, Ordering::Release);
        let t0 = std::time::Instant::now();
        let r = objc2::rc::autoreleasepool(|_| recognize(&job.img, job.lang));
        log::info!("ocr (Vision): {}×{} mất {:?}", job.img.width(), job.img.height(), t0.elapsed());
        if cancelled.load(Ordering::Acquire) != 0 {
            log::info!("ocr: job {} đã huỷ, bỏ kết quả", job.id);
            continue;
        }
        let r = r.map(|lines| TextLayout::build(lines, 1.0)).map_err(|e| {
            log::error!("ocr: {e}");
            t().ocr_failed_retry.to_string()
        });
        emit(Action::OcrDone(job.id, r));
    }
}

/// Ngôn ngữ cho Vision: tiếng Việt dùng mã mà Vision báo hỗ trợ (bắt đầu bằng "vi").
fn languages(req: &VNRecognizeTextRequest, lang: OcrLanguage) -> Vec<String> {
    let supported: Vec<String> = unsafe { req.supportedRecognitionLanguagesAndReturnError() }
        .map(|a| a.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let mut out = vec![];
    if lang == OcrLanguage::Vi {
        match supported.iter().find(|l| l.starts_with("vi")) {
            Some(vi) => out.push(vi.clone()),
            None => log::warn!("ocr: Vision không có tiếng Việt (hỗ trợ: {supported:?})"),
        }
    }
    out.push("en-US".into());
    out
}

pub fn recognize(img: &RgbaImage, lang: OcrLanguage) -> Result<Vec<Line>, String> {
    let (w, h) = (img.width() as f64, img.height() as f64);
    let cg = rgba_cgimage(img).ok_or("CGImage")?;
    let handler =
        unsafe { VNImageRequestHandler::initWithCGImage_options(VNImageRequestHandler::alloc(), &cg, &NSDictionary::new()) };
    let req = VNRecognizeTextRequest::new();
    req.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    req.setUsesLanguageCorrection(true);
    let langs = languages(&req, lang);
    log::debug!("ocr: ngôn ngữ {langs:?}");
    let langs: Vec<Retained<NSString>> = langs.iter().map(|s| NSString::from_str(s)).collect();
    req.setRecognitionLanguages(&NSArray::from_retained_slice(&langs));
    let reqs: Retained<NSArray<VNRequest>> = NSArray::from_slice(&[req.as_ref() as &VNRequest]);
    handler.performRequests_error(&reqs).map_err(|e| e.localizedDescription().to_string())?;
    let Some(results) = req.results() else { return Ok(vec![]) };

    // Toạ độ Vision chuẩn hoá 0..1, gốc dưới-trái -> px ảnh, gốc trên-trái.
    let to_px = |r: CGRect| -> [i32; 4] {
        let x = (r.origin.x * w).round() as i32;
        let y = ((1.0 - r.origin.y - r.size.height) * h).round() as i32;
        [x, y, (r.size.width * w).round().max(1.0) as i32, (r.size.height * h).round().max(1.0) as i32]
    };
    let mut lines = vec![];
    for obs in results.iter() {
        let cands = obs.topCandidates(1);
        let Some(text) = cands.firstObject() else { continue };
        let s = text.string().to_string();
        if s.trim().is_empty() {
            continue;
        }
        let bbox = to_px(unsafe { obs.boundingBox() });
        let conf = text.confidence() * 100.0;
        let words = words_of(&text, &s, &to_px, conf);
        lines.push(Line { text: s, bbox, conf, words });
    }
    Ok(lines)
}

/// Bbox của đoạn `range` (UTF-16) trong `text`.
fn range_box(text: &VNRecognizedText, range: NSRange) -> Option<CGRect> {
    let r: Result<Retained<VNRectangleObservation>, Retained<NSError>> =
        unsafe { msg_send![text, boundingBoxForRange: range, error: _] };
    r.ok().map(|o| unsafe { o.boundingBox() })
}

/// Tách dòng thành từ (theo khoảng trắng), mỗi từ có bbox và vị trí từng ký tự.
fn words_of(text: &VNRecognizedText, s: &str, to_px: &impl Fn(CGRect) -> [i32; 4], conf: f32) -> Vec<LineWord> {
    let mut out = vec![];
    // Vị trí UTF-16 của từng ký tự.
    let mut u16_pos = 0usize;
    let mut word: Vec<(char, usize, usize)> = vec![];
    let flush = |word: &mut Vec<(char, usize, usize)>, out: &mut Vec<LineWord>| {
        if word.is_empty() {
            return;
        }
        let start = word[0].1;
        let end = word.last().map(|c| c.1 + c.2).unwrap_or(start);
        let wtext: String = word.iter().map(|c| c.0).collect();
        if let Some(b) = range_box(text, NSRange::new(start, end - start)) {
            let bbox = to_px(b);
            let chars: Vec<[i32; 2]> = word
                .iter()
                .filter_map(|&(_, p, l)| range_box(text, NSRange::new(p, l)).map(to_px).map(|r| [r[0], r[2]]))
                .collect();
            // Thiếu vị trí ký tự nào thì để Chup tự ước lượng.
            let chars = if chars.len() == word.len() { chars } else { vec![] };
            out.push(LineWord { text: wtext, bbox, conf, chars });
        }
        word.clear();
    };
    for c in s.chars() {
        let n = c.len_utf16();
        if c.is_whitespace() {
            flush(&mut word, &mut out);
        } else {
            word.push((c, u16_pos, n));
        }
        u16_pos += n;
    }
    flush(&mut word, &mut out);
    out
}

fn rgba_cgimage(img: &RgbaImage) -> Option<objc2_core_foundation::CFRetained<objc2_core_graphics::CGImage>> {
    let (w, h) = img.dimensions();
    let buf = chup_ui::pixels::premultiply(img);
    super::gfx::image_from_bgra(&buf, w as usize, w as usize, h as usize)
}
