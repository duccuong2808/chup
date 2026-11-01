//! Chụp vùng và lưu shot (SPEC 6.8, 6.9).
//! `grab_region` và `persist_shot` cố ý tách riêng: Phase 2 (Freeze, OCR) dùng lại.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{DateTime, Local};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ExtendedColorType, ImageEncoder, RgbaImage,
};

use crate::{clipboard, coords::Rect};

/// Thumbnail rộng tối đa 320 px: đủ nét cho thẻ 160 CSS px ở scale 200%.
pub const THUMB_MAX_WIDTH: u32 = 320;

#[derive(Clone, Debug)]
pub struct Shot {
    pub id: String,
    pub path: PathBuf,
    /// File PNG của thumbnail trong cache, dùng làm ảnh khi kéo thả.
    pub thumb_path: PathBuf,
    /// Thumbnail đã giải mã, để stack vẽ thẳng không phải đọc lại file.
    pub thumb: Arc<RgbaImage>,
    pub width: u32,
    pub height: u32,
    pub created_at: DateTime<Local>,
}

/// Chụp `rect` (physical px toàn desktop). Vùng được cắt theo màn hình chứa tâm của nó.
pub fn grab_region(rect: Rect) -> Result<RgbaImage, String> {
    let (cx, cy) = rect.center();
    let mon = xcap::Monitor::from_point(cx, cy).map_err(|e| format!("Không tìm thấy màn hình của vùng chụp: {e}"))?;
    let m = Rect::new(
        mon.x().map_err(|e| e.to_string())?,
        mon.y().map_err(|e| e.to_string())?,
        mon.width().map_err(|e| e.to_string())?,
        mon.height().map_err(|e| e.to_string())?,
    );
    let r = rect
        .intersect(&m)
        .ok_or_else(|| "Vùng chụp nằm ngoài màn hình".to_string())?;
    mon.capture_region((r.x - m.x) as u32, (r.y - m.y) as u32, r.w, r.h)
        .map_err(|e| format!("Chụp màn hình thất bại: {e}"))
}

/// Ảnh của 1 màn hình trong lần chụp Freeze: rect physical của màn hình kèm ảnh (hoặc lỗi).
pub type MonitorShot = (Rect, Result<RgbaImage, String>);

/// Chụp mọi màn hình song song (Freeze, SPEC 14.3).
/// Mỗi thread tự lấy `Monitor` của mình, không chia sẻ đối tượng xcap giữa các thread.
pub fn grab_monitors() -> Result<Vec<MonitorShot>, String> {
    let mons = xcap::Monitor::all().map_err(|e| format!("Không liệt kê được màn hình: {e}"))?;
    let rects: Vec<Rect> = mons
        .iter()
        .filter_map(|m| Some(Rect::new(m.x().ok()?, m.y().ok()?, m.width().ok()?, m.height().ok()?)))
        .collect();
    drop(mons);
    Ok(std::thread::scope(|s| {
        let jobs: Vec<_> = rects
            .iter()
            .map(|r| {
                s.spawn(move || {
                    let (cx, cy) = r.center();
                    xcap::Monitor::from_point(cx, cy)
                        .and_then(|m| m.capture_image())
                        .map_err(|e| format!("Chụp màn hình thất bại: {e}"))
                })
            })
            .collect();
        rects
            .iter()
            .zip(jobs)
            .map(|(r, j)| (*r, j.join().unwrap_or_else(|_| Err("Thread chụp bị panic".into()))))
            .collect()
    }))
}

pub struct PersistOptions {
    pub save_dir: PathBuf,
    pub fallback_dir: PathBuf,
    pub pattern: String,
    pub copy_to_clipboard: bool,
    pub thumbs_dir: PathBuf,
}

pub struct PersistOutcome {
    pub shot: Shot,
    /// Cảnh báo không chặn (đã lưu vào folder dự phòng, clipboard lỗi...).
    pub warnings: Vec<String>,
}

pub fn persist_shot(img: &RgbaImage, opts: &PersistOptions) -> Result<PersistOutcome, String> {
    let now = Local::now();
    let id = ulid::Ulid::generate().to_string();
    let (w, h) = img.dimensions();

    // Thumbnail chạy song song suốt. PNG chỉ mã hoá 1 lần, rồi ghi file và clipboard
    // song song trên cùng mảng byte đó.
    let (saved, thumb, clip) = std::thread::scope(|s| {
        let thumb = s.spawn(|| make_thumbnail(img, &opts.thumbs_dir, &id));
        let (saved, clip) = match encode_png(img) {
            Err(e) => (Err(e), None),
            Ok(png) => std::thread::scope(|s2| {
                let clip = opts.copy_to_clipboard.then(|| s2.spawn(|| clipboard::copy_image(img, Some(&png))));
                let saved = save_png(&png, opts, &now);
                (saved, clip.map(|h| h.join()))
            }),
        };
        (saved, thumb.join(), clip)
    });

    let (path, mut warnings) = saved?;
    let (thumb_path, thumb) = thumb
        .map_err(|_| "Tạo thumbnail bị lỗi".to_string())?
        .map_err(|e| format!("Tạo thumbnail bị lỗi: {e}"))?;
    match clip {
        Some(Ok(Err(e))) => warnings.push(format!("Không copy được vào clipboard: {e}")),
        Some(Err(_)) => warnings.push("Không copy được vào clipboard".into()),
        _ => {}
    }

    Ok(PersistOutcome {
        shot: Shot {
            id,
            path,
            thumb_path,
            thumb: Arc::new(thumb),
            width: w,
            height: h,
            created_at: now,
        },
        warnings,
    })
}

pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut buf = Vec::with_capacity((img.width() * img.height()) as usize);
    PngEncoder::new_with_quality(&mut buf, CompressionType::Fast, FilterType::Adaptive)
        .write_image(img.as_raw(), img.width(), img.height(), ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Ghi PNG vào `save_dir`; lỗi thì ghi vào `fallback_dir` và kèm cảnh báo.
fn save_png(bytes: &[u8], opts: &PersistOptions, now: &DateTime<Local>) -> Result<(PathBuf, Vec<String>), String> {
    let base = render_file_name(&opts.pattern, now);
    match write_unique(&opts.save_dir, &base, bytes) {
        Ok(p) => Ok((p, vec![])),
        Err(primary_err) => {
            if opts.fallback_dir == opts.save_dir {
                return Err(format!("Không lưu được ảnh vào {}: {primary_err}", opts.save_dir.display()));
            }
            let p = write_unique(&opts.fallback_dir, &base, bytes).map_err(|e| {
                format!(
                    "Không lưu được ảnh vào {} ({primary_err}) lẫn {} ({e})",
                    opts.save_dir.display(),
                    opts.fallback_dir.display()
                )
            })?;
            Ok((
                p.clone(),
                vec![format!(
                    "Không ghi được vào {} ({primary_err}). Ảnh đã lưu tại {}",
                    opts.save_dir.display(),
                    p.display()
                )],
            ))
        }
    }
}

/// Tạo file mới, không bao giờ ghi đè: thêm hậu tố -1, -2...
fn write_unique(dir: &Path, base: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for i in 0..1000 {
        let name = if i == 0 { format!("{base}.png") } else { format!("{base}-{i}.png") };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(bytes).and_then(|_| f.flush()) {
                    drop(f);
                    let _ = fs::remove_file(&path);
                    return Err(e.to_string());
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Quá nhiều file trùng tên".into())
}

/// Pattern: phần trong `{...}` là định dạng ngày giờ kiểu `yyyyMMdd-HHmmss-SSS`.
pub fn render_file_name(pattern: &str, now: &DateTime<Local>) -> String {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        match rest[start..].find('}') {
            Some(end_rel) => {
                let token = &rest[start + 1..start + end_rel];
                out.push_str(&now.format(&date_token_to_strftime(token)).to_string());
                rest = &rest[start + end_rel + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    let cleaned = sanitize_file_name(&out);
    if cleaned.is_empty() {
        "chup".into()
    } else {
        cleaned
    }
}

fn date_token_to_strftime(token: &str) -> String {
    const MAP: [(&str, &str); 7] = [
        ("yyyy", "%Y"),
        ("SSS", "%3f"),
        ("MM", "%m"),
        ("dd", "%d"),
        ("HH", "%H"),
        ("mm", "%M"),
        ("ss", "%S"),
    ];
    let mut out = String::new();
    let mut i = 0;
    let bytes = token.as_bytes();
    'outer: while i < bytes.len() {
        for (k, v) in MAP {
            if token[i..].starts_with(k) {
                out.push_str(v);
                i += k.len();
                continue 'outer;
            }
        }
        let ch = token[i..].chars().next().unwrap();
        if ch == '%' {
            out.push_str("%%");
        } else {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    out
}

fn sanitize_file_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c })
        .collect();
    cleaned.trim().trim_end_matches('.').to_string()
}

fn make_thumbnail(img: &RgbaImage, dir: &Path, id: &str) -> Result<(PathBuf, RgbaImage), String> {
    let (w, h) = img.dimensions();
    let tw = w.clamp(1, THUMB_MAX_WIDTH);
    let th = ((h as u64 * tw as u64) / w.max(1) as u64).max(1) as u32;
    let thumb = image::imageops::thumbnail(img, tw, th);
    let bytes = encode_png(&thumb)?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{id}.png"));
    fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok((path, thumb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 31, 9, 5, 7).unwrap() + chrono::Duration::milliseconds(42)
    }

    #[test]
    fn default_pattern_renders() {
        assert_eq!(render_file_name("chup-{yyyyMMdd-HHmmss-SSS}", &t()), "chup-20261031-090507-042");
    }

    #[test]
    fn pattern_edge_cases() {
        assert_eq!(render_file_name("{yyyy}/{MM}", &t()), "2026_10");
        assert_eq!(render_file_name("a{b", &t()), "a{b");
        assert_eq!(render_file_name("x{%}", &t()), "x%");
        assert_eq!(render_file_name("   ", &t()), "chup");
        assert_eq!(render_file_name("ảnh {dd}.", &t()), "ảnh 31");
    }

    #[test]
    fn write_unique_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("chup-w-{}", ulid::Ulid::generate()));
        let a = write_unique(&dir, "x", b"1").unwrap();
        let b = write_unique(&dir, "x", b"2").unwrap();
        assert_ne!(a, b);
        assert!(b.ends_with("x-1.png"));
        assert_eq!(fs::read(&a).unwrap(), b"1");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persist_falls_back_and_makes_thumbnail() {
        let root = std::env::temp_dir().join(format!("chup-p-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&root).unwrap();
        // save_dir là 1 FILE -> không tạo được thư mục -> phải rơi về fallback.
        let blocker = root.join("blocker");
        fs::write(&blocker, b"").unwrap();
        let opts = PersistOptions {
            save_dir: blocker.join("sub"),
            fallback_dir: root.join("fallback"),
            pattern: "t-{SSS}".into(),
            copy_to_clipboard: false,
            thumbs_dir: root.join("thumbs"),
        };
        let img = RgbaImage::from_pixel(640, 100, image::Rgba([10, 20, 30, 255]));
        let out = persist_shot(&img, &opts).unwrap();
        assert!(out.shot.path.starts_with(root.join("fallback")));
        assert_eq!(out.warnings.len(), 1);
        let thumb = image::open(&out.shot.thumb_path).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (320, 50));
        assert_eq!(out.shot.thumb.dimensions(), (320, 50));
        let saved = image::open(&out.shot.path).unwrap().to_rgba8();
        assert_eq!(saved.dimensions(), (640, 100));
        assert_eq!(saved.get_pixel(5, 5), &image::Rgba([10, 20, 30, 255]));
        fs::remove_dir_all(root).unwrap();
    }
}
