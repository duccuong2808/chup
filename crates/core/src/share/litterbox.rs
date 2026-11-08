//! Litterbox (litterbox.catbox.moe): upload ẩn danh, có hạn 1h–72h, ≤ 1 GB. Kết quả là link text một dòng.

use super::{multipart, plain_link, Expiry, HttpRequest, HttpResponse, ShareError};

pub const API: &str = "https://litterbox.catbox.moe/resources/internals/api.php";

/// Tên file gửi lên luôn là `chup.png`: không lộ tên file thật (thường có ngày giờ).
pub fn build(png: &[u8], expiry: Expiry, rng: &mut dyn FnMut(&mut [u8])) -> HttpRequest {
    use multipart::Part::*;
    multipart::request(
        API,
        &[
            Text("reqtype", "fileupload"),
            Text("time", expiry.litterbox()),
            File("fileToUpload", "chup.png", "image/png", png),
        ],
        rng,
    )
}

pub fn parse(res: &HttpResponse) -> Result<String, ShareError> {
    plain_link(res)
}

#[cfg(test)]
mod tests {
    use super::super::testing::rng;
    use super::*;

    #[test]
    fn multipart_body_has_fields_and_file() {
        let r = build(b"\x89PNGdata", Expiry::H12, &mut rng());
        assert_eq!(r.url, API);
        let ct = &r
            .headers
            .iter()
            .find(|(k, _)| *k == "Content-Type")
            .unwrap()
            .1;
        let boundary = ct.strip_prefix("multipart/form-data; boundary=").unwrap();
        let body = String::from_utf8_lossy(&r.body);
        assert!(body.starts_with(&format!("--{boundary}\r\n")));
        assert!(body.contains("name=\"reqtype\"\r\n\r\nfileupload\r\n"));
        assert!(body.contains("name=\"time\"\r\n\r\n12h\r\n"));
        assert!(body.contains("name=\"fileToUpload\"; filename=\"chup.png\"\r\nContent-Type: image/png\r\n\r\n\u{FFFD}PNGdata\r\n"));
        assert!(body.ends_with(&format!("--{boundary}--\r\n")));
    }

    #[test]
    fn parse_link_or_error() {
        let r = |s: u16, b: &str| HttpResponse {
            status: s,
            body: b.as_bytes().to_vec(),
        };
        assert_eq!(
            parse(&r(200, "https://litter.catbox.moe/pcfyx4.png")).unwrap(),
            "https://litter.catbox.moe/pcfyx4.png"
        );
        assert_eq!(parse(&r(413, "")), Err(ShareError::TooLarge));
        assert_eq!(
            parse(&r(200, "<html>maintenance</html>")),
            Err(ShareError::Server("HTTP 200".into()))
        );
        assert_eq!(
            parse(&r(412, "No expire time specified.")),
            Err(ShareError::Server("No expire time specified.".into()))
        );
        assert!(parse(&r(200, "http://x.png")).is_err(), "chỉ nhận https");
    }
}
