# Thiết kế: Chia sẻ ảnh qua link (upload)

Ngày: 2026-11-08. Beads: epic `chup-share`. Đặc tả: [SPEC mục 19](../../SPEC.md#19-chia-sẻ-ảnh-qua-link).
Trước đây SPEC mục 2 xếp "Upload cloud / share link" ngoài phạm vi; tài liệu này đưa nó vào.

## Người dùng chốt (2026-11-08)

- **Hai kiểu, chọn mỗi lần**:
  - **Link ảnh**: Litterbox (catbox.moe), link ảnh trực tiếp, chat (Slack, GitHub, Zalo) hiện ảnh ngay. Chỉ Litterbox,
    luôn có hạn; không có lựa chọn lâu dài, không provider khác.
  - **Link riêng tư**: PrivateBin (mặc định `https://privatebin.net`, đổi được instance). Chup mã hoá trước khi gửi,
    khoá nằm sau `#` của link; server không đọc được ảnh. Người nhận xem trên trang của PrivateBin.
- **Link lâu dài** (bổ sung cùng ngày): catbox.moe bằng tài khoản của người dùng (`userhash`). Ảnh gắn với tài khoản,
  giữ vĩnh viễn, xoá được trên trang quản lý của catbox. Chỉ hiện trong menu khi đã nhập userhash. Userhash lưu trong
  Credential Manager (Windows) / Keychain (macOS), không nằm trong config. Chup không lưu lịch sử, không xoá từ Chup.
- **Chỗ bấm**: nút Upload ở hàng nút hover của thumbnail (Stack) và toolbar Preview (cộng menu chuột phải Preview).
  Bấm mở menu chọn kiểu.
- **Không** tự host trang xem (ví dụ GitHub Pages của repo): domain của tác giả thành nơi ai cũng có thể dùng để phát
  tán nội dung đã mã hoá (không chỉ từ Chup), báo cáo vi phạm sẽ đổ lên tài khoản, và ai sửa được trang thì lấy được
  khoá. PrivateBin là dự án có sẵn, trang xem do bên vận hành instance chịu trách nhiệm.

## Khảo sát (2026-11-08)

| Provider | Key | Hạn | Ghi chú |
|---|---|---|---|
| Litterbox | Không | 1h / 12h / 24h / 72h | ≤ 1 GB, link `litter.catbox.moe`, có `access-control-allow-origin: *` |
| catbox.moe ẩn danh | Không | Xoá sau 2 năm không ai truy cập | Không xoá chủ động được: loại |
| catbox.moe có tài khoản | `userhash` | Vĩnh viễn | Xoá qua API `deletefiles` hoặc trang quản lý; catbox cam kết không thoả thuận với công ty AI tạo sinh: **chọn cho Link lâu dài** |
| 0x0.st | Không | 30 ngày–1 năm | Lúc khảo sát trả HTTP 500, dịch vụ cá nhân: loại |
| Imgur | Client-ID | Không | Chặn IP UK từ 9/2025; Client-ID trong repo public dễ bị lạm dụng rồi khoá: loại |
| ImgBB, freeimage | API key | Tuỳ | Người dùng phải tự lấy key: loại |
| PrivateBin | Không | 5 phút–3 ngày (privatebin.net) | Mã hoá đầu cuối, có "Burn after reading". Danh bạ privatebin.info/directory ~180 instance, nhiều instance bật upload file (privatebin.net 2.1.1, A+) |
| Send (send.vis.ee) | Không | ≤ 7 ngày | Mã hoá đầu cuối nhưng người nhận phải tải file; giao thức websocket + ECE phức tạp: không chọn |

Hạn 1h / 12h / 1 ngày / 3 ngày khớp cả Litterbox (`1h`/`12h`/`24h`/`72h`) và PrivateBin (`1hour`/`12hour`/`1day`/
`3day`), nên dùng một Cài đặt chung.

## 1. Hành vi

- Bấm Upload, menu: "Link ảnh" và "Link riêng tư" (kèm hạn, ví dụ "· 1 ngày"), thêm "Link lâu dài (catbox)" khi đã
  có userhash; mục đầu là kiểu mặc định trong Cài đặt.
- Toast "Đang upload…"; xong: link vào clipboard, toast "Đã copy link · hết hạn sau {}". Lỗi: toast ngắn, chi tiết log.
- Mỗi lúc một job; bấm khi đang bận: toast "Đang upload ảnh khác".
- **Lần đầu mỗi kiểu**: hộp thoại nói rõ ảnh đi đâu, ai xem được ("Ảnh được gửi lên Litterbox (catbox.moe). Ai có
  link đều xem được tới khi hết hạn; app chat có thể giữ bản xem trước lâu hơn." / "Ảnh được mã hoá trên máy rồi gửi
  lên {instance}. Server không đọc được ảnh, nhưng ai có link đều xem được."). Đồng ý thì nhớ, không hỏi lại.
- **Cài đặt, trang mới "Chia sẻ"**: thời hạn (1 giờ / 12 giờ / **1 ngày** / 3 ngày); kiểu mặc định; Link riêng tư:
  "Xoá sau lần xem đầu" (mặc định tắt), địa chỉ instance PrivateBin (chỉ nhận `https://`); Link lâu dài: ô userhash
  (ô mật khẩu, nút Lưu / Xoá, hiện "Đã lưu" chứ không hiện lại giá trị), nút "Quản lý ảnh trên catbox" mở
  `https://catbox.moe/user/view.php`.
- Không gửi gì nếu người dùng chưa chọn mục trong menu. Không thống kê, không tự upload.
- **Không làm**: lịch sử upload, xoá link sớm (`deletetoken` chỉ ghi log debug), provider khác, mật khẩu, phím tắt
  "chụp rồi upload".

## 2. Core: `crates/core/src/share/`

Không biết gì về OS; mỗi provider là hai hàm thuần.

```rust
pub enum ShareKind { Image, Private, Permanent }
pub enum Expiry { H1, H12, D1, D3 }
pub struct HttpRequest { pub url: String, pub headers: Vec<(String, String)>, pub body: Vec<u8> }

// litterbox.rs
pub fn build(png: &[u8], expiry: Expiry) -> HttpRequest;          // multipart: reqtype=fileupload, time, fileToUpload
pub fn parse(status: u16, body: &[u8]) -> Result<String, ShareError>; // body là link text

// catbox.rs: cùng multipart với litterbox, thêm userhash; URL https://catbox.moe/user/api.php
pub fn build(png: &[u8], userhash: &str) -> HttpRequest;
pub fn parse(status: u16, body: &[u8]) -> Result<String, ShareError>; // link https://files.catbox.moe/…

// privatebin.rs
pub struct Sealed { pub req: HttpRequest, pub key: [u8; 32] }
pub fn build(png: &[u8], base: &str, expiry: Expiry, burn: bool, rng: &mut dyn FnMut(&mut [u8])) -> Sealed;
pub fn parse(status: u16, body: &[u8], base: &str, key: &[u8; 32]) -> Result<String, ShareError>;
```

PrivateBin định dạng v2 (đã chốt bằng spike, [2026-11-08-share-spike.md](2026-11-08-share-spike.md)):
- Khoá 32 byte, salt 8 byte, IV 16 byte từ `rng` (platform truyền `getrandom`; test truyền RNG cố định).
- Plaintext: JSON `{"paste":"","attachment":["data:image/png;base64,…"],"attachment_name":["chup.png"]}` (mảng),
  nén **deflate raw** (`flate2::DeflateEncoder`, mức 7).
- Khoá AES = PBKDF2-HMAC-SHA256(khoá, salt, 100 000 vòng, 32 byte). AES-256-GCM **nonce 16 byte**
  (`AesGcm<aes::Aes256, U16>`), tag 128 bit.
- `adata = [[b64(iv), b64(salt), 100000, 256, 128, "aes", "gcm", "zlib"], "plaintext", 0, burn as u8]`,
  AAD = `serde_json::to_string(adata)`.
- Body: `{"v":2,"adata":…,"ct":"<base64>","meta":{"expire":"1hour|12hour|1day|3day"}}`, header
  `X-Requested-With: JSONHttpRequest`, `Content-Type: application/json`, POST vào gốc instance.
- Kết quả `{"status":0,"id":…}` → link `<base>/?<id>#<base58(khoá)>`; burn: `#-<base58>` (trang hỏi xác nhận trước khi
  tải, bot xem trước link của app chat không đốt paste). Chup **không** đọc lại paste sau khi tạo.
- Lỗi: `status: 1` → `ShareError::Server(message)` hiện nguyên câu của server (privatebin.net: "Please wait 60 seconds
  between each post."), không tự thử lại. HTTP 413 (nginx trả HTML) → `TooLarge`.
- **Ảnh quá cỡ** (privatebin.net nhận body ≤ ~3 MiB, tức PNG ≤ ~2,4 MB): gặp 413 thì đổi ảnh sang **WebP lossless**
  (`image` feature `webp`, `WebPEncoder::new_lossless`; ảnh chụp giảm 35–50%, ~20 ms) và gửi lại **một lần**
  (`data:image/webp`, tên `chup.webp`). Vẫn 413 → toast "Ảnh quá lớn với {instance}. Dùng Link ảnh hoặc đổi instance".
  413 của nginx không tính vào giới hạn 60 giây (spike). Không đoán trước giới hạn: instance khác có thể nhận nhiều hơn.

`ShareError { Network, TooLarge, Server(String) }` → chuỗi toast qua `i18n`. Kiểm tra cỡ trước khi gửi
(Litterbox 1 GB; PrivateBin theo số đo spike).

Crate mới (đã compile trong spike): `aes-gcm =0.11.1`, `aes =0.9.4`, `pbkdf2 =0.13.0`, `bs58 =0.5.1`,
`base64 =0.23.1`; bật feature `webp` của `image`. `flate2`, `getrandom`, `sha2` đã có trong lockfile.

### Model

- `Action::Upload { path, kind }`: chưa đồng ý kiểu này → `Effect::AskShareConsent(kind)`; đang bận → toast; còn lại →
  `Effect::StartUpload(UploadJob { id, path, kind, expiry, burn, privatebin_url })`.
- `Action::ShareConsent(kind, bool)`: lưu config, đồng ý thì chạy tiếp như `Upload`.
- `Action::UploadDone(id, Result<String, ShareError>)`: id cũ thì bỏ; Ok → `Effect::CopyText(link)` + toast; Err → toast.
- Config (field mới có serde default, **không tăng version**, như `preview`, `ocr`): `"share": { "expiry": "1d", "defaultKind": "image", "burnAfterReading": false,
  "privatebinUrl": "https://privatebin.net", "catboxAccount": false,
  "consent": { "image": false, "private": false, "permanent": false } }`. `catboxAccount` chỉ cho biết đã lưu
  userhash (để hiện mục menu); bản thân userhash không vào `Model` hay config: worker đọc từ kho bí mật lúc upload.
  Kho không có (bị xoá ngoài Chup) → `ShareError::NoAccount`, toast mời nhập lại, đặt `catboxAccount = false`. Config cũ thiếu
  `share` thì lấy mặc định; địa chỉ instance hỏng trong file thì về mặc định.

## 3. UI: `crates/ui`

- `Icon::Upload` (mũi tên lên trên khay, 24×24, nét 2 như icon khác).
- Stack: thêm nút vào hàng nút hover (Copy, Folder, **Upload**, ✕); hit-test và test vẽ.
- Preview toolbar: `[T] [Ghim] [Upload] [Copy] [Folder] [✕]`. Đang upload thì nút Upload quay bận như [T].

## 4. Platform

- `Http::post(&HttpRequest) -> Result<(u16, Vec<u8>), String>`, timeout 30 s, chỉ `https://`, không theo redirect
  sang host khác.
  - Windows: mở rộng WinHTTP ở `ocr_download.rs` (tách phần mở kết nối dùng chung, thêm POST).
  - macOS: `/usr/bin/curl --fail-with-body -sS --max-time 30 --proto =https --data-binary @-` + `-H`, body qua stdin.
- Worker thread: đọc PNG → `build` → `post` → `parse` → `emit(Action::UploadDone)`. Mã hoá chạy trong worker.
- Menu chọn kiểu: menu native (`TrackPopupMenu` / `NSMenu`) như menu chuột phải Preview. Hộp thoại đồng ý:
  `MessageBox` / `NSAlert`.
- Cài đặt: trang "Chia sẻ" ở cả hai vỏ.
- Kho bí mật `secret::{get, set, delete}("catbox-userhash")`:
  - Windows: `CredWriteW` / `CredReadW` / `CredDeleteW`, `CRED_TYPE_GENERIC`, target `app.chup.desktop/catbox`
    (feature `Win32_Security_Credentials` của crate `windows`).
  - macOS: Keychain generic password, service `app.chup.desktop`, account `catbox` (`security-framework` hoặc
    `SecItem*` qua objc2; chọn lúc làm, pin `=`).
  - `CHUP_SELFTEST`: kho giả trong thư mục self-test, không đụng Keychain thật.
- Log: chỉ ghi kiểu, provider, cỡ, thời gian, `id`/link của Link ảnh và Link lâu dài. **Không bao giờ** ghi khoá,
  userhash hay link riêng tư đầy đủ (body request của catbox chứa userhash: không log body).

## 5. Spike (xong 2026-11-08, kết quả ở [2026-11-08-share-spike.md](2026-11-08-share-spike.md))

Công cụ ở `tools/share-spike/` (ngoài workspace). Chỉ gửi PNG thử không có nội dung thật.
1. Litterbox `time=1h`: kiểm tra link, `Content-Type`.
2. privatebin.net hạn 5 phút: mở link trong Chrome, ảnh hiện được. Chốt dạng `attachment` (chuỗi / mảng).
3. Đo cỡ tối đa của privatebin.net (PNG 5 / 10 / 20 MB), ghi mã lỗi khi vượt.
4. Giữ một paste thật (khoá + ciphertext) làm vector test giải mã.
5. catbox có tài khoản: chỉ chạy nếu user cung cấp userhash thử; upload, kiểm tra ảnh hiện trong trang quản lý, xoá
   bằng `deletefiles`. Không có userhash thì dựa vào tài liệu `catbox.moe/tools.php` + test `#[ignore]`.
Kết quả: `docs/plans/2026-11-08-share-spike.md`.

## 6. Test

- `litterbox`: body multipart đúng; `parse` nhận link, từ chối HTML / chuỗi lạ.
- `privatebin`: round-trip với RNG cố định (mã hoá rồi giải mã lại, so byte); giải mã vector
  `tools/share-spike/vector-privatebin-v2.json` (chép vào fixture của core); `status 0/1`; 413; base58; link `#-` khi burn;
  WebP lossless giải mã lại đúng từng pixel.
- `catbox`: body có `userhash`, đúng endpoint; `parse` như litterbox.
- `Model`: hỏi đồng ý lần đầu; bận; `UploadDone` Ok/Err; id cũ bị bỏ; config cũ thiếu `share`; không có
  `catboxAccount` thì `Upload{Permanent}` bị từ chối; `NoAccount` tắt `catboxAccount`.
- `ui`: nút Upload trong stack và Preview, hit-test.
- Test mạng `#[ignore]` (`cargo test -- --ignored share_live`) gọi thật cả hai provider.
- Nghiệm thu thủ công: SPEC 19.5, kết quả ghi `docs/testing.md`.
