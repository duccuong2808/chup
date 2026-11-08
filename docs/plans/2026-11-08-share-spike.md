# Spike: chia sẻ ảnh (SPEC 19.4)

Ngày: 2026-11-08. Beads: `chup-share.1`. Công cụ: `tools/share-spike/` (ngoài workspace, HTTP qua `/usr/bin/curl`).
Chỉ gửi ảnh thử sinh ra (dải màu, nhiễu), không có nội dung thật. Máy dev macOS, mạng Việt Nam.

## Kết quả

| Việc | Kết quả |
|---|---|
| Litterbox `time=1h`, PNG 9,9 KB | HTTP 200 sau 0,66 s, body là link `https://litter.catbox.moe/pcfyx4.png`. Link trả `content-type: image/png`, `access-control-allow-origin: *` |
| privatebin.net 2.1.1, hạn 5/10 phút | HTTP 200 sau ~0,8 s: `{"status":0,"id":…,"url":"/?…","deletetoken":…}`. Tải lại (`GET ?pasteid=<id>` + `X-Requested-With: JSONHttpRequest`), giải mã: khớp từng byte |
| Mở link trong Chrome (headless) | Trang PrivateBin giải mã, hiện ảnh, "Download attachment (chup.png, 9.9 kB)", "expire in 9 minutes". Dòng "+++ no document text +++" vì `paste` rỗng (chấp nhận) |
| Burn after reading | Link dùng tiền tố `#-` thay `#` (trang hỏi xác nhận trước khi tải, nên bot xem trước link của app chat không đốt paste). `GET` lần 1 có dữ liệu, lần 2: `status 1`, "Document does not exist, has expired or has been deleted." |
| Cỡ tối đa privatebin.net | Body JSON ≤ 2 974 264 byte: qua; ≥ 3 378 412 byte: nginx **HTTP 413** (HTML, không phải JSON). Có lẽ `client_max_body_size 3m`. Body ≈ 1,29 × PNG ⇒ **PNG tối đa ~2,4 MB** |
| Giới hạn tần suất | `status 1`, "Please wait 60 seconds between each post." (lỗi 413 của nginx không tính) |
| Thời gian mã hoá | PBKDF2 100 000 vòng + deflate + AES-GCM: 9 ms (10 KB), 110 ms (2 MB), 1,2 s (26 MB), bản release |
| Cỡ ảnh thật trên máy dev | 55 PNG: trung vị 1,36 MB, p90 1,63 MB, lớn nhất 5,2 MB; 1/55 vượt 2,4 MB |
| catbox có tài khoản | Không chạy (không có userhash thử). Theo `catbox.moe/tools.php`: `POST https://catbox.moe/user/api.php`, `reqtype=fileupload`, `userhash`, `fileToUpload`; xoá `reqtype=deletefiles`, `files="a.png b.png"` |

## Định dạng PrivateBin 2.x đã chốt (đọc từ `privatebin.js` 2.1.1, kiểm bằng trang thật)

- Khoá: 32 byte ngẫu nhiên; link `<base>/?<id>#<base58(khoá)>` (bảng chữ base58 Bitcoin, crate `bs58` mặc định khớp);
  burn: `#-<base58>`. Trang đọc khoá bằng `base58decode(...).padStart(32, '\0')`: khoá bắt đầu bằng byte 0 vẫn đúng.
- `spec = [b64(iv 16 byte), b64(salt 8 byte), 100000, 256, 128, "aes", "gcm", "zlib"]`.
- `adata = [spec, "plaintext", 0 /*thảo luận*/, burn 0|1]`; AAD = `JSON.stringify(adata)` (serde_json `to_string`
  không khoảng trắng, khớp).
- Plaintext: `{"paste":"","attachment":["data:image/png;base64,…"],"attachment_name":["chup.png"]}`, **mảng** từ 2.x.
- "zlib" thực ra là **deflate raw** (zlib.js tạo context với `NO_ZLIB_HEADER = -1`): `flate2::DeflateEncoder`.
- Khoá AES = PBKDF2-HMAC-SHA256(khoá, salt, 100 000, 32 byte). AES-256-GCM với **nonce 16 byte**:
  `AesGcm<aes::Aes256, U16>` (không phải `Aes256Gcm` 12 byte).
- Body: `{"v":2,"adata":…,"ct":"<b64>","meta":{"expire":"5min|10min|…|1hour|12hour|1day|3day"}}`; header
  `X-Requested-With: JSONHttpRequest`, `Content-Type: application/json`; POST vào gốc instance.

Crate dùng trong spike (compile được, dùng lại cho core): `aes-gcm =0.11.1`, `aes =0.9.4`, `pbkdf2 =0.13.0`,
`sha2 =0.11.1` (đã có), `bs58 =0.5.1`, `base64 =0.23.1`, `flate2 =1.1.10` (đã có), `getrandom =0.4.3` (đã có).

## Vector test

`tools/share-spike/vector-privatebin-v2.json`: paste thật (đã hết hạn) của ảnh dải màu 800×500, gồm `key58`, `adata`,
`ct`, SHA-256 của PNG. `share-spike decrypt` giải mã ra đúng SHA-256. Chép vào test của `core::share::privatebin`.

## Hệ quả cho thiết kế

1. Instance mặc định nhận PNG tới ~2,4 MB. Đa số ảnh chụp lọt, nhưng ảnh toàn màn hình Retina/4K có thể vượt: cần
   cách xử lý (quyết định ở design).
2. Phân biệt lỗi: HTTP 413 (HTML) → "ảnh quá lớn với instance này"; `status 1` + `message` → hiện `message` của server
   (thường là "đợi 60 giây"), không tự thử lại.
3. Burn: link `#-`. Không tự `GET` lại sau khi tạo (sẽ đốt paste).
4. Chup không cần đọc lại paste; `parse` chỉ cần `status`, `id`.
