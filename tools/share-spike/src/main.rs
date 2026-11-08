//! Spike SPEC 19.4 (beads chup-share.1). Chỉ gửi ảnh thử sinh ngẫu nhiên, không có nội dung thật.
//!
//! share-spike png <out.png> <w> <h> [noise]        sinh ảnh thử (noise: nhiễu, nén kém -> đo giới hạn cỡ)
//! share-spike litterbox <png> <1h|12h|24h|72h>      upload Litterbox, in link
//! share-spike privatebin <base> <png> <expire> [burn] [vector.json]
//!                                                   mã hoá + upload, in link; tải lại, giải mã, so byte
//! share-spike webp <png>...                      đo cỡ WebP lossless (không gửi đi đâu)
//! share-spike decrypt <vector.json>                 giải mã vector đã lưu (không cần mạng)

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use base64::Engine;
use serde_json::{json, Value};

type Aes256Gcm16 = aes_gcm::AesGcm<aes::Aes256, aes_gcm::aead::consts::U16>;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("png") => make_png(&a[2], a[3].parse().unwrap(), a[4].parse().unwrap(), a.get(5).is_some()),
        Some("litterbox") => litterbox(&a[2], &a[3]),
        Some("privatebin") => privatebin(&a[2], &a[3], &a[4], a.get(5).map(String::as_str) == Some("burn"), a.get(6)),
        Some("webp") => {
            for f in &a[2..] {
                let img = image::open(f).unwrap().to_rgba8();
                let mut out = Vec::new();
                let t = std::time::Instant::now();
                image::codecs::webp::WebPEncoder::new_lossless(&mut out)
                    .encode(&img, img.width(), img.height(), image::ExtendedColorType::Rgba8)
                    .unwrap();
                let png = std::fs::metadata(f).unwrap().len();
                println!("{}x{} png {png} -> webp {} ({:.0}%) trong {:?}", img.width(), img.height(), out.len(), out.len() as f64 * 100.0 / png as f64, t.elapsed());
            }
        }
        Some("decrypt") => {
            let v: Value = serde_json::from_slice(&std::fs::read(&a[2]).unwrap()).unwrap();
            let key = bs58::decode(v["key58"].as_str().unwrap()).into_vec().unwrap();
            let png = decrypt(&key, &v["paste"]);
            println!("giải mã được {} byte, sha256 {}", png.len(), hex(&sha(&png)));
        }
        _ => eprintln!("xem đầu file main.rs"),
    }
}

fn make_png(out: &str, w: u32, h: u32, noise: bool) {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        if noise {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let b = seed.to_le_bytes();
            image::Rgba([b[0], b[1], b[2], 255])
        } else {
            image::Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, 160, 255])
        }
    });
    img.save(out).unwrap();
    println!("{out}: {} byte", std::fs::metadata(out).unwrap().len());
}

fn curl(args: &[&str], stdin: Option<&[u8]>) -> (u16, Vec<u8>) {
    let mut c = Command::new("/usr/bin/curl")
        .args(["-sS", "--max-time", "120", "--proto", "=https", "-w", "\n%{http_code}"])
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(b) = stdin {
        c.stdin.take().unwrap().write_all(b).unwrap();
    }
    let mut out = Vec::new();
    c.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    c.wait().unwrap();
    let cut = out.iter().rposition(|&b| b == b'\n').unwrap_or(0);
    let code = String::from_utf8_lossy(&out[cut..]).trim().parse().unwrap_or(0);
    out.truncate(cut);
    (code, out)
}

fn litterbox(png: &str, time: &str) {
    let t = std::time::Instant::now();
    let (code, body) = curl(
        &[
            "-F", "reqtype=fileupload",
            "-F", &format!("time={time}"),
            "-F", &format!("fileToUpload=@{png};filename=chup.png;type=image/png"),
            "https://litterbox.catbox.moe/resources/internals/api.php",
        ],
        None,
    );
    println!("HTTP {code} sau {:?}: {}", t.elapsed(), String::from_utf8_lossy(&body));
}

fn rand(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    getrandom::fill(&mut v).unwrap();
    v
}

fn derive(key: &[u8], salt: &[u8], iter: u32) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(key, salt, iter, &mut out);
    out
}

fn deflate_raw(b: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(7));
    e.write_all(b).unwrap();
    e.finish().unwrap()
}

fn privatebin(base: &str, png_path: &str, expire: &str, burn: bool, vector: Option<&String>) {
    let png = std::fs::read(png_path).unwrap();
    let key = rand(32);
    let (iv, salt) = (rand(16), rand(8));
    let spec = json!([B64.encode(&iv), B64.encode(&salt), 100000, 256, 128, "aes", "gcm", "zlib"]);
    let adata = json!([spec, "plaintext", 0, burn as u8]);
    let msg = json!({
        "paste": "",
        "attachment": [format!("data:image/png;base64,{}", B64.encode(&png))],
        "attachment_name": ["chup.png"],
    });
    let t = std::time::Instant::now();
    let plain = deflate_raw(msg.to_string().as_bytes());
    let cipher = Aes256Gcm16::new_from_slice(&derive(&key, &salt, 100000)).unwrap();
    let nonce = aes_gcm::aead::Nonce::<Aes256Gcm16>::try_from(&iv[..]).unwrap();
    let aad = adata.to_string();
    let ct = cipher.encrypt(&nonce, Payload { msg: &plain, aad: aad.as_bytes() }).unwrap();
    let body = json!({ "v": 2, "adata": adata, "ct": B64.encode(&ct), "meta": { "expire": expire } });
    let body = body.to_string();
    println!("mã hoá {:?}; png {} byte -> body {} byte", t.elapsed(), png.len(), body.len());

    let t = std::time::Instant::now();
    let (code, resp) = curl(
        &[
            "-X", "POST",
            "-H", "X-Requested-With: JSONHttpRequest",
            "-H", "Content-Type: application/json",
            "--data-binary", "@-",
            base,
        ],
        Some(body.as_bytes()),
    );
    println!("HTTP {code} sau {:?}: {}", t.elapsed(), String::from_utf8_lossy(&resp).chars().take(300).collect::<String>());
    let Ok(r) = serde_json::from_slice::<Value>(&resp) else { return };
    if r["status"] != 0 {
        return;
    }
    let id = r["id"].as_str().unwrap();
    let key58 = bs58::encode(&key).into_string();
    let base_q = format!("{}?", base.trim_end_matches('/').to_string() + "/");
    println!("LINK {base_q}{id}{}{key58}", if burn { "#-" } else { "#" });

    // Tải lại đúng như trang xem (GET ?pasteid kèm header JSON), giải mã, so byte. Burn: bỏ qua (đọc là mất).
    if burn {
        return;
    }
    let (code, got) = curl(&["-H", "X-Requested-With: JSONHttpRequest", &format!("{base_q}pasteid={id}")], None);
    let paste: Value = serde_json::from_slice(&got).unwrap();
    println!("GET HTTP {code}, status {}", paste["status"]);
    let back = decrypt(&key, &paste);
    println!("round-trip: {}", if back == png { "KHỚP" } else { "SAI" });
    if let Some(v) = vector {
        let keep = json!({ "key58": key58, "png_sha256": hex(&sha(&png)), "paste": { "adata": paste["adata"], "ct": paste["ct"] } });
        std::fs::write(v, serde_json::to_string_pretty(&keep).unwrap()).unwrap();
        println!("đã lưu vector {v}");
    }
}

fn decrypt(key: &[u8], paste: &Value) -> Vec<u8> {
    let spec = &paste["adata"][0];
    let iv = B64.decode(spec[0].as_str().unwrap()).unwrap();
    let salt = B64.decode(spec[1].as_str().unwrap()).unwrap();
    let ct = B64.decode(paste["ct"].as_str().unwrap()).unwrap();
    let cipher = Aes256Gcm16::new_from_slice(&derive(key, &salt, spec[2].as_u64().unwrap() as u32)).unwrap();
    let nonce = aes_gcm::aead::Nonce::<Aes256Gcm16>::try_from(&iv[..]).unwrap();
    let aad = paste["adata"].to_string();
    let plain = cipher.decrypt(&nonce, Payload { msg: &ct, aad: aad.as_bytes() }).expect("giải mã hỏng");
    let mut s = String::new();
    flate2::read::DeflateDecoder::new(&plain[..]).read_to_string(&mut s).unwrap();
    let m: Value = serde_json::from_str(&s).unwrap();
    let url = m["attachment"][0].as_str().unwrap();
    B64.decode(url.split_once(',').unwrap().1).unwrap()
}

fn sha(b: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    sha2::Sha256::digest(b).to_vec()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
