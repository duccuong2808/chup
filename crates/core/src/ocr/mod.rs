//! OCR (SPEC 15): tiền xử lý ảnh và hậu xử lý kết quả, không phụ thuộc engine hay OS.
//! Engine thật nằm ở lớp platform (Windows OCR, `chup-ocr.exe`); gói dữ liệu tiếng Việt ở [`pack`].

pub mod pack;

use image::{imageops, GrayImage, RgbaImage};
use unicode_normalization::UnicodeNormalization;

pub use chup_ocr_proto::Line;

use crate::config::OcrLanguage;

/// Scale màn hình lúc chụp nhỏ hơn số này thì phóng ảnh ×2 trước khi đưa vào Tesseract
/// (spike: chữ ~12 px ở màn hình 100% đọc kém hẳn nếu không phóng).
pub const UPSCALE_BELOW_SCALE: f64 = 1.5;
/// Không phóng nếu sau khi phóng cạnh dài vượt số này, hoặc diện tích vượt `MAX_UPSCALED_PIXELS`
/// (giữ thời gian OCR và RAM trong ngân sách; ảnh lớn như vậy thường đã đủ nét).
const MAX_UPSCALED_SIDE: u32 = 8192;
const MAX_UPSCALED_PIXELS: u64 = 12_000_000;
/// Dòng ngắn (≤ 2 ký tự) có confidence dưới ngưỡng này là rác (ô checkbox, viền...).
const JUNK_CONF: f32 = 50.0;
/// Độ dài tối đa của dòng trích trong toast.
const TOAST_SNIPPET_CHARS: usize = 40;

/// Ảnh xám đưa cho engine. Chỉ phóng ×2 khi dùng Tesseract và chữ có khả năng nhỏ.
pub fn prepare(img: &RgbaImage, scale: f64, lang: OcrLanguage) -> GrayImage {
    let gray = imageops::grayscale(img);
    let (w, h) = gray.dimensions();
    let upscale = lang == OcrLanguage::Vi
        && scale < UPSCALE_BELOW_SCALE
        && w.max(h) * 2 <= MAX_UPSCALED_SIDE
        && w as u64 * h as u64 * 4 <= MAX_UPSCALED_PIXELS;
    if upscale {
        imageops::resize(&gray, w * 2, h * 2, imageops::FilterType::CatmullRom)
    } else {
        gray
    }
}

fn is_junk(l: &Line) -> bool {
    let t = l.text.trim();
    !t.chars().any(char::is_alphanumeric) || (t.chars().count() <= 2 && l.conf < JUNK_CONF)
}

/// Hai dòng cùng hàng khi phần chồng nhau theo chiều dọc > 50% dòng thấp hơn.
fn same_row(row: (i32, i32), l: &Line) -> bool {
    let (top, bottom) = (l.bbox[1], l.bbox[1] + l.bbox[3]);
    let overlap = row.1.min(bottom) - row.0.max(top);
    let min_h = (row.1 - row.0).min(l.bbox[3]).max(1);
    overlap * 2 > min_h
}

/// Ghép các dòng engine trả về thành text để copy: bỏ rác, sắp trên xuống dưới / trái sang phải,
/// nối các dòng cùng hàng bằng dấu cách, chuẩn hoá NFC. Không có chữ thì trả chuỗi rỗng.
pub fn postprocess(lines: Vec<Line>) -> String {
    let mut lines: Vec<Line> = lines.into_iter().filter(|l| !is_junk(l)).collect();
    lines.sort_by_key(|l| (l.bbox[1], l.bbox[0]));
    let mut rows: Vec<((i32, i32), Vec<Line>)> = vec![];
    for l in lines {
        match rows.last_mut() {
            Some((span, row)) if same_row(*span, &l) => {
                *span = (span.0.min(l.bbox[1]), span.1.max(l.bbox[1] + l.bbox[3]));
                row.push(l);
            }
            _ => rows.push(((l.bbox[1], l.bbox[1] + l.bbox[3]), vec![l])),
        }
    }
    let text: Vec<String> = rows
        .into_iter()
        .map(|(_, mut row)| {
            row.sort_by_key(|l| l.bbox[0]);
            row.iter().map(|l| l.text.trim()).collect::<Vec<_>>().join(" ")
        })
        .collect();
    text.join("\n").nfc().collect()
}

/// Câu toast sau khi copy: "Đã copy 3 dòng: “dòng đầu…”".
pub fn copied_message(text: &str) -> String {
    let n = text.lines().count();
    let first = text.lines().next().unwrap_or_default();
    let mut snippet: String = first.chars().take(TOAST_SNIPPET_CHARS).collect();
    if first.chars().count() > TOAST_SNIPPET_CHARS || n > 1 {
        snippet.push('…');
    }
    format!("Đã copy {n} dòng: “{snippet}”")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, x: i32, y: i32, w: i32, h: i32, conf: f32) -> Line {
        Line { text: text.into(), bbox: [x, y, w, h], conf }
    }

    #[test]
    fn prepare_upscales_only_small_text_for_tesseract() {
        let img = RgbaImage::from_pixel(100, 40, image::Rgba([255, 0, 0, 255]));
        assert_eq!(prepare(&img, 1.0, OcrLanguage::Vi).dimensions(), (200, 80));
        assert_eq!(prepare(&img, 1.25, OcrLanguage::Vi).dimensions(), (200, 80));
        assert_eq!(prepare(&img, 1.5, OcrLanguage::Vi).dimensions(), (100, 40));
        assert_eq!(prepare(&img, 1.0, OcrLanguage::En).dimensions(), (100, 40), "Windows OCR không phóng");
        let wide = RgbaImage::new(5000, 10);
        assert_eq!(prepare(&wide, 1.0, OcrLanguage::Vi).dimensions(), (5000, 10), "quá dài thì không phóng");
        let big = RgbaImage::new(1920, 1080);
        assert_eq!(prepare(&big, 1.0, OcrLanguage::Vi).dimensions(), (3840, 2160), "8,3 MP sau khi phóng: vẫn phóng");
        let huge = RgbaImage::new(2560, 1440);
        assert_eq!(prepare(&huge, 1.0, OcrLanguage::Vi).dimensions(), (2560, 1440), "14,7 MP: không phóng");
        let px = prepare(&img, 2.0, OcrLanguage::Vi).get_pixel(0, 0).0[0];
        assert!(px > 40 && px < 100, "đỏ thuần ra xám tối vừa: {px}");
    }

    #[test]
    fn postprocess_sorts_rows_and_merges_same_row() {
        let lines = vec![
            line("dòng hai\n", 0, 40, 200, 20, 90.0),
            line("cột phải", 300, 2, 100, 20, 90.0),
            line("Dòng một", 0, 0, 200, 22, 90.0),
            line("dòng ba", 10, 80, 150, 20, 90.0),
        ];
        assert_eq!(postprocess(lines), "Dòng một cột phải\ndòng hai\ndòng ba");
    }

    #[test]
    fn rows_overlapping_less_than_half_stay_separate() {
        let lines = vec![line("trên", 0, 0, 100, 20, 90.0), line("dưới", 0, 12, 100, 20, 90.0)];
        assert_eq!(postprocess(lines), "trên\ndưới");
    }

    #[test]
    fn junk_lines_are_dropped() {
        let lines = vec![
            line("| ~", 0, 0, 10, 10, 95.0),
            line("☐", 0, 20, 10, 10, 30.0),
            line("x1", 0, 40, 10, 10, 20.0),
            line("OK", 0, 60, 20, 10, 80.0),
            line("a", 0, 80, 10, 10, 90.0),
        ];
        assert_eq!(postprocess(lines), "OK\na");
        assert_eq!(postprocess(vec![line("  ", 0, 0, 1, 1, 99.0)]), "");
        assert_eq!(postprocess(vec![]), "");
    }

    #[test]
    fn output_is_nfc() {
        // "Việt" ở dạng tổ hợp (NFD): e + dấu mũ + dấu nặng.
        let nfd = "Vie\u{0302}\u{0323}t";
        let out = postprocess(vec![line(nfd, 0, 0, 50, 10, 90.0)]);
        assert_eq!(out, "Vi\u{1EC7}t");
        assert_eq!(out.chars().count(), 4);
    }

    #[test]
    fn copied_message_shows_count_and_snippet() {
        assert_eq!(copied_message("Xin chào"), "Đã copy 1 dòng: “Xin chào”");
        assert_eq!(copied_message("a\nb\nc"), "Đã copy 3 dòng: “a…”");
        let long = "x".repeat(50);
        assert_eq!(copied_message(&long), format!("Đã copy 1 dòng: “{}…”", "x".repeat(40)));
    }
}
