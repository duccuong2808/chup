//! Chia sẻ ảnh qua link (SPEC 19, thiết kế `docs/plans/2026-11-08-share-design.md`).
//!
//! Ba kiểu: Link ảnh (Litterbox, có hạn), Link riêng tư (PrivateBin v2, mã hoá trên máy), Link lâu dài (catbox bằng
//! userhash của người dùng). Mỗi provider là hai hàm thuần `build` / `parse`; [`run`] ghép lại, lớp platform chỉ cần
//! cài [`Http::post`] và gọi `run` ở thread nền. Không biết gì về OS nên test được hết.

pub mod catbox;
pub mod litterbox;
mod multipart;
pub mod privatebin;

use std::{path::PathBuf, time::Instant};

use serde::{Deserialize, Serialize};

use crate::i18n::{fill, t};

/// Kiểu link người dùng chọn trong menu Upload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareKind {
    /// Litterbox: link ảnh trực tiếp, app chat hiện ảnh ngay. Litterbox xem được ảnh.
    #[default]
    Image,
    /// PrivateBin: mã hoá trên máy, khoá nằm sau `#` của link.
    Private,
    /// catbox bằng tài khoản người dùng: không hết hạn.
    Permanent,
}

/// Thời hạn link (dùng chung cho Litterbox và PrivateBin; catbox không hết hạn).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expiry {
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "12h")]
    H12,
    #[default]
    #[serde(rename = "1d")]
    D1,
    #[serde(rename = "3d")]
    D3,
}

impl Expiry {
    pub const ALL: [Expiry; 4] = [Expiry::H1, Expiry::H12, Expiry::D1, Expiry::D3];

    /// Giá trị `time` của Litterbox.
    pub fn litterbox(self) -> &'static str {
        match self {
            Expiry::H1 => "1h",
            Expiry::H12 => "12h",
            Expiry::D1 => "24h",
            Expiry::D3 => "72h",
        }
    }

    /// Giá trị `meta.expire` của PrivateBin (privatebin.net có đủ 4 mức này).
    pub fn privatebin(self) -> &'static str {
        match self {
            Expiry::H1 => "1hour",
            Expiry::H12 => "12hour",
            Expiry::D1 => "1day",
            Expiry::D3 => "3day",
        }
    }

    /// "1 giờ", "1 ngày"… theo ngôn ngữ giao diện.
    pub fn label(self) -> &'static str {
        let s = t();
        match self {
            Expiry::H1 => s.share_exp_h1,
            Expiry::H12 => s.share_exp_h12,
            Expiry::D1 => s.share_exp_d1,
            Expiry::D3 => s.share_exp_d3,
        }
    }
}

/// Instance PrivateBin mặc định.
pub const DEFAULT_PRIVATEBIN: &str = "https://privatebin.net/";

/// Chuẩn hoá địa chỉ instance PrivateBin người dùng nhập: bắt buộc `https://`, có host, không có `?` / `#` /
/// khoảng trắng; luôn kết thúc bằng `/` (POST vào đó, link là `<url>?<id>#<khoá>`).
pub fn normalize_privatebin_url(s: &str) -> Option<String> {
    let s = s.trim();
    let rest = s.strip_prefix("https://")?;
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || s.contains(['?', '#']) || s.chars().any(char::is_whitespace) {
        return None;
    }
    Some(if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    })
}

/// Host của URL (để báo lỗi gọn: "privatebin.net").
pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// Một request HTTPS POST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Gửi request (lớp platform: WinHTTP / curl). Chỉ `https://`, timeout ~60 s, không theo redirect sang host khác.
/// `Err`: lỗi mạng (không có response).
pub trait Http {
    fn post(&mut self, req: &HttpRequest) -> Result<HttpResponse, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareError {
    /// Không đọc được file ảnh.
    Io(String),
    /// Lỗi mạng (chi tiết chỉ ghi log).
    Network(String),
    /// Ảnh quá lớn với dịch vụ (biết trước, hoặc server trả 413).
    TooLarge,
    /// Server báo lỗi; chuỗi là câu của server (PrivateBin `message`) hoặc "HTTP 500".
    Server(String),
    /// Link lâu dài mà không còn userhash trong kho bí mật.
    NoAccount,
}

impl ShareError {
    /// Câu báo cho người dùng. `job`: để nói tên dịch vụ.
    pub fn user_message(&self, job: &UploadJob) -> String {
        let s = t();
        match self {
            ShareError::Io(_) => s.share_err_read.into(),
            ShareError::Network(_) => s.share_err_network.into(),
            ShareError::TooLarge => match job.kind {
                ShareKind::Private => fill(
                    s.share_err_too_large_private,
                    &[&host_of(&job.privatebin_url)],
                ),
                _ => s.share_err_too_large.into(),
            },
            ShareError::Server(m) => fill(s.share_err_server, &[m]),
            ShareError::NoAccount => s.share_err_no_account.into(),
        }
    }
}

/// Việc upload giao cho thread nền.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadJob {
    pub id: u32,
    pub path: PathBuf,
    pub kind: ShareKind,
    pub expiry: Expiry,
    /// PrivateBin: xoá sau lần xem đầu.
    pub burn: bool,
    /// Đã chuẩn hoá, kết thúc bằng `/`.
    pub privatebin_url: String,
}

/// Cỡ tối đa biết trước (byte ảnh), để không gửi vô ích. PrivateBin không đoán trước: instance tự báo 413.
const LITTERBOX_MAX: usize = 1 << 30;
const CATBOX_MAX: usize = 200 << 20;

/// Làm cả việc upload: đọc file, dựng request, gửi, đọc kết quả. Trả về link.
/// `userhash`: chỉ cho Link lâu dài (lớp platform đọc từ kho bí mật). `rng`: CSPRNG (test truyền RNG cố định).
pub fn run(
    job: &UploadJob,
    http: &mut dyn Http,
    userhash: Option<&str>,
    rng: &mut dyn FnMut(&mut [u8]),
) -> Result<String, ShareError> {
    let png = std::fs::read(&job.path).map_err(|e| ShareError::Io(e.to_string()))?;
    let start = Instant::now();
    let result = match job.kind {
        ShareKind::Image => {
            if png.len() > LITTERBOX_MAX {
                return Err(ShareError::TooLarge);
            }
            let res = http
                .post(&litterbox::build(&png, job.expiry, rng))
                .map_err(ShareError::Network)?;
            litterbox::parse(&res)
        }
        ShareKind::Permanent => {
            let hash = userhash
                .filter(|h| !h.trim().is_empty())
                .ok_or(ShareError::NoAccount)?;
            if png.len() > CATBOX_MAX {
                return Err(ShareError::TooLarge);
            }
            let res = http
                .post(&catbox::build(&png, hash.trim(), rng))
                .map_err(ShareError::Network)?;
            catbox::parse(&res)
        }
        ShareKind::Private => private(job, http, &png, rng),
    };
    match &result {
        // Link riêng tư không bao giờ vào log (phần sau `#` là khoá).
        Ok(link) if job.kind == ShareKind::Private => {
            log::info!(
                "share: private {} byte -> {} sau {:?}",
                png.len(),
                privatebin::paste_id(link),
                start.elapsed()
            )
        }
        Ok(link) => log::info!(
            "share: {:?} {} byte -> {link} sau {:?}",
            job.kind,
            png.len(),
            start.elapsed()
        ),
        Err(e) => log::warn!(
            "share: {:?} {} byte lỗi {e:?} sau {:?}",
            job.kind,
            png.len(),
            start.elapsed()
        ),
    }
    result
}

/// PrivateBin: gửi PNG; instance trả 413 thì đổi sang WebP lossless (nhỏ hơn 35–50%) và gửi lại một lần.
/// 413 do nginx chặn trước PHP nên không tính vào giới hạn "60 giây một lần" của privatebin.net (spike).
fn private(
    job: &UploadJob,
    http: &mut dyn Http,
    png: &[u8],
    rng: &mut dyn FnMut(&mut [u8]),
) -> Result<String, ShareError> {
    let attempt = |http: &mut dyn Http,
                   data: &[u8],
                   mime: &str,
                   name: &str,
                   rng: &mut dyn FnMut(&mut [u8])| {
        let sealed = privatebin::build(
            data,
            mime,
            name,
            &job.privatebin_url,
            job.expiry,
            job.burn,
            rng,
        );
        let res = http.post(&sealed.req).map_err(ShareError::Network)?;
        privatebin::parse(&res, &job.privatebin_url, &sealed.key, job.burn)
    };
    match attempt(http, png, "image/png", "chup.png", rng) {
        Err(ShareError::TooLarge) => {
            let webp = to_webp_lossless(png).map_err(|e| {
                log::warn!("share: không đổi được sang WebP: {e}");
                ShareError::TooLarge
            })?;
            log::info!(
                "share: 413 với PNG {} byte, thử WebP lossless {} byte",
                png.len(),
                webp.len()
            );
            attempt(http, &webp, "image/webp", "chup.webp", rng)
        }
        r => r,
    }
}

/// PNG -> WebP lossless (giữ nguyên từng pixel).
pub fn to_webp_lossless(png: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(png)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out)
        .encode(
            &img,
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Câu ngắn của server để hiện cho người dùng: bỏ HTML, cắt dài.
fn server_text(res: &HttpResponse) -> String {
    let body = String::from_utf8_lossy(&res.body);
    let body = body.trim();
    if body.is_empty() || body.starts_with('<') {
        return format!("HTTP {}", res.status);
    }
    let mut s: String = body.chars().take(160).collect();
    if body.chars().count() > 160 {
        s.push('…');
    }
    s
}

/// Link `https://…` một dòng (kết quả của Litterbox / catbox).
fn plain_link(res: &HttpResponse) -> Result<String, ShareError> {
    if res.status == 413 {
        return Err(ShareError::TooLarge);
    }
    let body = String::from_utf8_lossy(&res.body);
    let link = body.trim();
    if res.status == 200
        && link.starts_with("https://")
        && !link.contains(char::is_whitespace)
        && !link.contains('<')
    {
        return Ok(link.to_string());
    }
    Err(ShareError::Server(server_text(res)))
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// RNG cố định cho test.
    pub fn rng() -> impl FnMut(&mut [u8]) {
        let mut n = 7u8;
        move |b: &mut [u8]| {
            for x in b {
                n = n.wrapping_mul(31).wrapping_add(17);
                *x = n;
            }
        }
    }

    /// HTTP giả: trả lần lượt các response, ghi lại request.
    pub struct FakeHttp {
        pub responses: Vec<Result<HttpResponse, String>>,
        pub sent: Vec<HttpRequest>,
    }

    impl FakeHttp {
        pub fn new(responses: Vec<Result<HttpResponse, String>>) -> Self {
            Self {
                responses,
                sent: vec![],
            }
        }
    }

    impl Http for FakeHttp {
        fn post(&mut self, req: &HttpRequest) -> Result<HttpResponse, String> {
            self.sent.push(req.clone());
            self.responses.remove(0)
        }
    }

    pub fn ok(status: u16, body: &str) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status,
            body: body.as_bytes().to_vec(),
        })
    }

    /// PNG thật nhỏ (dải màu) ghi ra file tạm.
    pub fn png_file(w: u32, h: u32) -> PathBuf {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x * 7) as u8, (y * 5) as u8, ((x ^ y) * 3) as u8, 255])
        });
        let dir = std::env::temp_dir().join(format!("chup-share-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.png");
        img.save(&p).unwrap();
        p
    }

    pub fn job(kind: ShareKind, path: PathBuf) -> UploadJob {
        UploadJob {
            id: 1,
            path,
            kind,
            expiry: Expiry::D1,
            burn: false,
            privatebin_url: DEFAULT_PRIVATEBIN.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn privatebin_url_is_https_only_and_ends_with_slash() {
        assert_eq!(
            normalize_privatebin_url(" https://privatebin.net ").as_deref(),
            Some("https://privatebin.net/")
        );
        assert_eq!(
            normalize_privatebin_url("https://x.org/paste/").as_deref(),
            Some("https://x.org/paste/")
        );
        for bad in [
            "http://privatebin.net",
            "privatebin.net",
            "https://",
            "https:///x",
            "https://a b.org",
            "https://a.org/?x",
            "https://a.org/#k",
        ] {
            assert_eq!(normalize_privatebin_url(bad), None, "{bad}");
        }
        assert_eq!(host_of("https://privatebin.net/"), "privatebin.net");
    }

    #[test]
    fn expiry_maps_to_both_services() {
        assert_eq!(
            Expiry::ALL.map(Expiry::litterbox),
            ["1h", "12h", "24h", "72h"]
        );
        assert_eq!(
            Expiry::ALL.map(Expiry::privatebin),
            ["1hour", "12hour", "1day", "3day"]
        );
        assert_eq!(serde_json::to_string(&Expiry::D1).unwrap(), "\"1d\"");
        assert_eq!(
            serde_json::from_str::<ShareKind>("\"permanent\"").unwrap(),
            ShareKind::Permanent
        );
    }

    #[test]
    fn run_image_and_network_error() {
        let p = png_file(20, 10);
        let mut http = FakeHttp::new(vec![
            ok(200, "https://litter.catbox.moe/abc123.png\n"),
            Err("timeout".into()),
        ]);
        let j = job(ShareKind::Image, p);
        assert_eq!(
            run(&j, &mut http, None, &mut rng()).unwrap(),
            "https://litter.catbox.moe/abc123.png"
        );
        assert!(http.sent[0]
            .url
            .starts_with("https://litterbox.catbox.moe/"));
        assert_eq!(
            run(&j, &mut http, None, &mut rng()),
            Err(ShareError::Network("timeout".into()))
        );
    }

    #[test]
    fn run_missing_file() {
        let j = job(ShareKind::Image, PathBuf::from("/khong/co/file.png"));
        let mut http = FakeHttp::new(vec![]);
        assert!(matches!(
            run(&j, &mut http, None, &mut rng()),
            Err(ShareError::Io(_))
        ));
        assert!(http.sent.is_empty());
    }

    #[test]
    fn run_permanent_needs_userhash() {
        let j = job(ShareKind::Permanent, png_file(8, 8));
        let mut http = FakeHttp::new(vec![ok(200, "https://files.catbox.moe/x1y2z3.png")]);
        assert_eq!(
            run(&j, &mut http, None, &mut rng()),
            Err(ShareError::NoAccount)
        );
        assert_eq!(
            run(&j, &mut http, Some("  "), &mut rng()),
            Err(ShareError::NoAccount)
        );
        assert!(http.sent.is_empty(), "không có userhash thì không gửi");
        assert_eq!(
            run(&j, &mut http, Some("abc"), &mut rng()).unwrap(),
            "https://files.catbox.moe/x1y2z3.png"
        );
    }

    #[test]
    fn run_private_retries_once_as_webp_after_413() {
        let p = png_file(64, 48);
        let j = job(ShareKind::Private, p.clone());
        let too_big = ok(
            413,
            "<html><head><title>413 Request Entity Too Large</title></head></html>",
        );
        let mut http = FakeHttp::new(vec![
            too_big.clone(),
            ok(200, r#"{"status":0,"id":"a1b2","url":"/?a1b2"}"#),
        ]);
        let link = run(&j, &mut http, None, &mut rng()).unwrap();
        assert!(link.starts_with("https://privatebin.net/?a1b2#"));
        assert_eq!(http.sent.len(), 2);
        // Lần hai là WebP: giải mã ra đúng ảnh.
        let key = privatebin::key_of(&link).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&http.sent[1].body).unwrap();
        let (mime, name, data) = privatebin::decrypt(&body, &key).unwrap();
        assert_eq!((mime.as_str(), name.as_str()), ("image/webp", "chup.webp"));
        let back = image::load_from_memory(&data).unwrap().to_rgba8();
        let orig = image::open(&p).unwrap().to_rgba8();
        assert_eq!(
            back.as_raw(),
            orig.as_raw(),
            "lossless: từng pixel giữ nguyên"
        );

        // WebP vẫn quá cỡ: báo lỗi, không thử thêm.
        let mut http = FakeHttp::new(vec![too_big.clone(), too_big]);
        let e = run(&j, &mut http, None, &mut rng()).unwrap_err();
        assert_eq!(e, ShareError::TooLarge);
        assert_eq!(http.sent.len(), 2);
        assert!(e.user_message(&j).contains("privatebin.net"));
    }

    #[test]
    fn server_text_hides_html_and_truncates() {
        let r = |s: u16, b: &str| HttpResponse {
            status: s,
            body: b.as_bytes().to_vec(),
        };
        assert_eq!(server_text(&r(502, "<html>bad gateway</html>")), "HTTP 502");
        assert_eq!(server_text(&r(500, "")), "HTTP 500");
        assert_eq!(
            server_text(&r(200, " No expire time specified. ")),
            "No expire time specified."
        );
        assert_eq!(server_text(&r(200, &"x".repeat(300))).chars().count(), 161);
    }
}

/// Gọi dịch vụ thật (gửi ảnh thử dải màu, hạn 1 giờ). Chạy tay:
/// `cargo test -p chup-core share_live -- --ignored --nocapture`.
#[cfg(all(test, unix))]
mod live {
    use super::testing::*;
    use super::*;
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};

    struct Curl;

    impl Http for Curl {
        fn post(&mut self, req: &HttpRequest) -> Result<HttpResponse, String> {
            let mut c = Command::new("/usr/bin/curl");
            c.args([
                "-sS",
                "--max-time",
                "60",
                "--proto",
                "=https",
                "-w",
                "\n%{http_code}",
                "--data-binary",
                "@-",
            ]);
            for (k, v) in &req.headers {
                c.arg("-H").arg(format!("{k}: {v}"));
            }
            let mut ch = c
                .arg(&req.url)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            ch.stdin
                .take()
                .unwrap()
                .write_all(&req.body)
                .map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            ch.stdout
                .take()
                .unwrap()
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            ch.wait().map_err(|e| e.to_string())?;
            let cut = out
                .iter()
                .rposition(|&b| b == b'\n')
                .ok_or("curl không trả mã HTTP")?;
            let status = String::from_utf8_lossy(&out[cut + 1..])
                .trim()
                .parse()
                .map_err(|_| "mã HTTP lạ")?;
            out.truncate(cut);
            Ok(HttpResponse { status, body: out })
        }
    }

    #[test]
    #[ignore]
    fn share_live() {
        let rng = &mut |b: &mut [u8]| getrandom::fill(b).unwrap();
        let p = png_file(320, 200);
        let mut j = job(ShareKind::Image, p.clone());
        j.expiry = Expiry::H1;
        let link = run(&j, &mut Curl, None, rng).unwrap();
        println!("litterbox: {link}");
        j.kind = ShareKind::Private;
        let link = run(&j, &mut Curl, None, rng).unwrap();
        println!("privatebin id: {}", privatebin::paste_id(&link));
        assert!(privatebin::key_of(&link).is_some());
    }
}
