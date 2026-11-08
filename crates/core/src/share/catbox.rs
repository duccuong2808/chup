//! catbox.moe bằng tài khoản người dùng (`userhash`): ảnh gắn với tài khoản, không hết hạn, xoá trên trang quản lý
//! của catbox. Tài liệu: <https://catbox.moe/tools.php>. Body chứa userhash: không bao giờ ghi log body.

use super::{multipart, plain_link, HttpRequest, HttpResponse, ShareError};

pub const API: &str = "https://catbox.moe/user/api.php";
/// Trang quản lý ảnh của tài khoản (nút trong Cài đặt).
pub const MANAGE_URL: &str = "https://catbox.moe/user/view.php";

pub fn build(png: &[u8], userhash: &str, rng: &mut dyn FnMut(&mut [u8])) -> HttpRequest {
    use multipart::Part::*;
    multipart::request(
        API,
        &[
            Text("reqtype", "fileupload"),
            Text("userhash", userhash),
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
    fn body_has_userhash() {
        let r = build(b"img", "u53rh45h", &mut rng());
        assert_eq!(r.url, API);
        let body = String::from_utf8_lossy(&r.body);
        assert!(body.contains("name=\"userhash\"\r\n\r\nu53rh45h\r\n"));
        assert!(body.contains("name=\"reqtype\"\r\n\r\nfileupload\r\n"));
        let r = HttpResponse {
            status: 200,
            body: b"https://files.catbox.moe/x1y2z3.png".to_vec(),
        };
        assert_eq!(parse(&r).unwrap(), "https://files.catbox.moe/x1y2z3.png");
    }
}
