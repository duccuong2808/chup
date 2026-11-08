//! PrivateBin định dạng v2 (đã chốt bằng spike: `docs/plans/2026-11-08-share-spike.md`, privatebin.net 2.1.1).
//!
//! Chup mã hoá trên máy; server chỉ thấy bản mã. Khoá 32 byte nằm sau `#` của link (trình duyệt không gửi phần này
//! lên server). Bẫy: nonce GCM 16 byte (không phải 12), "zlib" thực ra là deflate raw, `attachment` là mảng,
//! burn dùng `#-` và không được đọc lại paste sau khi tạo (đọc là mất).

use std::io::{Read, Write};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use base64::Engine;
use serde_json::{json, Value};

use super::{server_text, Expiry, HttpRequest, HttpResponse, ShareError};

type Cipher = aes_gcm::AesGcm<aes::Aes256, aes_gcm::aead::consts::U16>;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const ITERATIONS: u32 = 100_000;

/// Request đã mã hoá cùng khoá (để ghép link sau khi server trả `id`).
pub struct Sealed {
    pub req: HttpRequest,
    pub key: [u8; 32],
}

fn derive(key: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(key, salt, iterations, &mut out);
    out
}

/// Mã hoá ảnh `data` (MIME `mime`, tên `name`) thành paste gửi tới `base` (đã chuẩn hoá, kết thúc bằng `/`).
pub fn build(
    data: &[u8],
    mime: &str,
    name: &str,
    base: &str,
    expiry: Expiry,
    burn: bool,
    rng: &mut dyn FnMut(&mut [u8]),
) -> Sealed {
    let (mut key, mut iv, mut salt) = ([0u8; 32], [0u8; 16], [0u8; 8]);
    rng(&mut key);
    rng(&mut iv);
    rng(&mut salt);
    let spec = json!([
        B64.encode(iv),
        B64.encode(salt),
        ITERATIONS,
        256,
        128,
        "aes",
        "gcm",
        "zlib"
    ]);
    // [spec, định dạng, cho thảo luận, xoá sau lần xem đầu]
    let adata = json!([spec, "plaintext", 0, burn as u8]);
    let message = json!({
        "paste": "",
        "attachment": [format!("data:{mime};base64,{}", B64.encode(data))],
        "attachment_name": [name],
    });
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(7));
    z.write_all(message.to_string().as_bytes())
        .expect("ghi vào Vec");
    let plain = z.finish().expect("ghi vào Vec");
    let cipher = Cipher::new_from_slice(&derive(&key, &salt, ITERATIONS)).expect("khoá 32 byte");
    let nonce = aes_gcm::aead::Nonce::<Cipher>::try_from(&iv[..]).expect("IV 16 byte");
    // AAD = adata dạng JSON không khoảng trắng, đúng như JSON.stringify của trang xem.
    let aad = adata.to_string();
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plain,
                aad: aad.as_bytes(),
            },
        )
        .expect("AES-GCM không lỗi khi mã hoá");
    let body = json!({ "v": 2, "adata": adata, "ct": B64.encode(ct), "meta": { "expire": expiry.privatebin() } });
    Sealed {
        req: HttpRequest {
            url: base.into(),
            headers: vec![
                ("X-Requested-With", "JSONHttpRequest".into()),
                ("Content-Type", "application/json".into()),
            ],
            body: body.to_string().into_bytes(),
        },
        key,
    }
}

/// Đọc kết quả, ghép link `<base>?<id>#<base58(khoá)>` (`#-` nếu xoá sau lần xem đầu: trang hỏi xác nhận trước khi
/// tải, nên bot xem trước link của app chat không làm mất paste).
pub fn parse(
    res: &HttpResponse,
    base: &str,
    key: &[u8; 32],
    burn: bool,
) -> Result<String, ShareError> {
    if res.status == 413 {
        return Err(ShareError::TooLarge);
    }
    let Ok(v) = serde_json::from_slice::<Value>(&res.body) else {
        return Err(ShareError::Server(server_text(res)));
    };
    match (v["status"].as_i64(), v["id"].as_str()) {
        (Some(0), Some(id)) if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) => {
            Ok(format!(
                "{base}?{id}{}{}",
                if burn { "#-" } else { "#" },
                bs58::encode(key).into_string()
            ))
        }
        _ => Err(ShareError::Server(
            v["message"]
                .as_str()
                .map_or_else(|| server_text(res), str::to_string),
        )),
    }
}

/// `id` của paste trong link (để ghi log mà không lộ khoá).
pub fn paste_id(link: &str) -> &str {
    let q = link.split_once('?').map_or("", |(_, q)| q);
    q.split('#').next().unwrap_or("")
}

/// Khoá trong link (sau `#` hoặc `#-`). Trang xem đệm 0 ở đầu tới 32 byte.
pub fn key_of(link: &str) -> Option<[u8; 32]> {
    let frag = link.split_once('#')?.1;
    let frag = frag.strip_prefix('-').unwrap_or(frag);
    let raw = bs58::decode(frag).into_vec().ok()?;
    if raw.len() > 32 {
        return None;
    }
    let mut key = [0u8; 32];
    key[32 - raw.len()..].copy_from_slice(&raw);
    Some(key)
}

/// Giải mã paste (`{adata, ct}`) thành (MIME, tên file, dữ liệu) của attachment đầu. Dùng cho test và kiểm tra.
pub fn decrypt(paste: &Value, key: &[u8; 32]) -> Result<(String, String, Vec<u8>), String> {
    let spec = &paste["adata"][0];
    let b64 = |v: &Value| {
        B64.decode(v.as_str().ok_or("thiếu trường")?)
            .map_err(|e| e.to_string())
    };
    let (iv, salt, ct) = (b64(&spec[0])?, b64(&spec[1])?, b64(&paste["ct"])?);
    let iterations = spec[2].as_u64().ok_or("thiếu số vòng")? as u32;
    let cipher =
        Cipher::new_from_slice(&derive(key, &salt, iterations)).map_err(|e| e.to_string())?;
    let nonce = aes_gcm::aead::Nonce::<Cipher>::try_from(&iv[..]).map_err(|_| "IV sai cỡ")?;
    let aad = paste["adata"].to_string();
    let plain = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &ct,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| "sai khoá hoặc dữ liệu hỏng")?;
    let mut s = String::new();
    flate2::read::DeflateDecoder::new(&plain[..])
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    let m: Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    let url = m["attachment"][0].as_str().ok_or("không có attachment")?;
    let (head, data) = url.split_once(',').ok_or("data URL sai")?;
    let mime = head
        .strip_prefix("data:")
        .and_then(|h| h.strip_suffix(";base64"))
        .ok_or("data URL sai")?;
    let name = m["attachment_name"][0].as_str().unwrap_or("").to_string();
    Ok((
        mime.to_string(),
        name,
        B64.decode(data).map_err(|e| e.to_string())?,
    ))
}

#[cfg(test)]
mod tests {
    use super::super::testing::rng;
    use super::*;

    const BASE: &str = "https://privatebin.net/";

    fn res(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn round_trip_and_request_shape() {
        let s = build(
            b"\x89PNG anh",
            "image/png",
            "chup.png",
            BASE,
            Expiry::H12,
            false,
            &mut rng(),
        );
        assert_eq!(s.req.url, BASE);
        assert!(s
            .req
            .headers
            .contains(&("X-Requested-With", "JSONHttpRequest".into())));
        let body: Value = serde_json::from_slice(&s.req.body).unwrap();
        assert_eq!(body["v"], 2);
        assert_eq!(body["meta"]["expire"], "12hour");
        assert_eq!(body["adata"][1], "plaintext");
        assert_eq!(body["adata"][3], 0);
        let spec = &body["adata"][0];
        assert_eq!(spec[2], 100_000);
        assert_eq!(
            &spec.as_array().unwrap()[3..],
            &[
                json!(256),
                json!(128),
                json!("aes"),
                json!("gcm"),
                json!("zlib")
            ]
        );
        assert_eq!(
            B64.decode(spec[0].as_str().unwrap()).unwrap().len(),
            16,
            "IV 16 byte"
        );
        assert_eq!(
            B64.decode(spec[1].as_str().unwrap()).unwrap().len(),
            8,
            "salt 8 byte"
        );
        let (mime, name, data) = decrypt(&body, &s.key).unwrap();
        assert_eq!(
            (mime.as_str(), name.as_str(), data.as_slice()),
            ("image/png", "chup.png", &b"\x89PNG anh"[..])
        );
        // Sai khoá thì không giải mã được.
        let mut bad = s.key;
        bad[0] ^= 1;
        assert!(decrypt(&body, &bad).is_err());
    }

    #[test]
    fn decrypts_paste_made_by_privatebin_net() {
        // Paste thật tạo trong spike (2026-11-08, ảnh dải màu 800×500), trang privatebin.net đã hiện được ảnh.
        let v: Value = serde_json::from_str(include_str!("vector-privatebin-v2.json")).unwrap();
        let key = key_of(&format!(
            "https://privatebin.net/?x#{}",
            v["key58"].as_str().unwrap()
        ))
        .unwrap();
        let (mime, name, png) = decrypt(&v["paste"], &key).unwrap();
        assert_eq!((mime.as_str(), name.as_str()), ("image/png", "chup.png"));
        use sha2::Digest;
        let hex: String = sha2::Sha256::digest(&png)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, v["png_sha256"].as_str().unwrap());
    }

    #[test]
    fn parse_builds_link_with_key() {
        let s = build(
            b"x",
            "image/png",
            "chup.png",
            BASE,
            Expiry::D1,
            false,
            &mut rng(),
        );
        let link = parse(&res(200, r#"{"status":0,"id":"a80573fbc66b3627","url":"\/?a80573fbc66b3627","deletetoken":"t"}"#), BASE, &s.key, false).unwrap();
        assert!(link.starts_with("https://privatebin.net/?a80573fbc66b3627#"));
        assert_eq!(key_of(&link), Some(s.key));
        assert_eq!(paste_id(&link), "a80573fbc66b3627");
        let burn = parse(&res(200, r#"{"status":0,"id":"ab12"}"#), BASE, &s.key, true).unwrap();
        assert!(burn.contains("?ab12#-"), "burn: tiền tố #-");
        assert_eq!(key_of(&burn), Some(s.key));
    }

    #[test]
    fn key_with_leading_zero_byte_round_trips() {
        let mut key = [9u8; 32];
        key[0] = 0;
        let link = parse(&res(200, r#"{"status":0,"id":"ab"}"#), BASE, &key, false).unwrap();
        assert_eq!(key_of(&link), Some(key));
    }

    #[test]
    fn parse_errors() {
        let k = [1u8; 32];
        assert_eq!(
            parse(&res(413, "<html>413</html>"), BASE, &k, false),
            Err(ShareError::TooLarge)
        );
        assert_eq!(
            parse(
                &res(
                    200,
                    r#"{"status":1,"message":"Please wait 60 seconds between each post."}"#
                ),
                BASE,
                &k,
                false
            ),
            Err(ShareError::Server(
                "Please wait 60 seconds between each post.".into()
            ))
        );
        assert_eq!(
            parse(&res(502, "<html>Bad Gateway</html>"), BASE, &k, false),
            Err(ShareError::Server("HTTP 502".into()))
        );
        assert!(
            parse(&res(200, r#"{"status":0,"id":"../evil"}"#), BASE, &k, false).is_err(),
            "id lạ"
        );
    }
}
