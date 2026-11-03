# Thiết kế: nhúng `chup-ocr.exe` vào `chup.exe` (chup-7bc)

Ngày 2026-11-03. Bổ sung cho `2026-11-01-ocr-design.md`.

## Vấn đề

Chạy `chup.exe` rời (lấy từ artifact CI, không qua bộ cài) thì không có `chup-ocr.exe` bên cạnh. Chọn OCR tiếng Việt
lúc đó chỉ nhận được câu "Hãy cài Chup bằng bộ cài". Người dùng muốn app tự lấy những gì còn thiếu, không phải cài lại.

## Chọn cách

| Cách | Được | Mất |
|---|---|---|
| Tải `chup-ocr.exe` từ GitHub Release | Đúng nghĩa "tải về"; `chup.exe` không nặng thêm | Repo Chup **private**: tải ẩn danh không được. Phải dựng thêm một repo công khai và cho CI đẩy sang đó (việc công khai ra ngoài, user tự quyết). Lại phải ghim hash theo từng bản `chup.exe` |
| **Nhúng vào `chup.exe` lúc build (chọn)** | Không cần mạng, không cần chỗ phát hành; hai exe luôn cùng PROTO; hash có sẵn lúc build | `chup.exe` nặng thêm ~6,4 MB (chỉ là dữ liệu, không nạp vào RAM nếu không dùng); bộ cài chứa engine hai lần |

Dữ liệu `vie`/`eng.traineddata` vẫn tải từ `tesseract-ocr/tessdata_fast` như cũ (`core::ocr::pack`).
Sau này nếu repo public thì đổi sang tải chỉ là thay chỗ lấy byte trong `Embedded::ensure`.

## Cách làm

- **Build** (`crates/app/build.rs`): biến `CHUP_OCR_DIR` = thư mục có `chup-ocr.exe` + `NOTICE-ocr.txt` (CI đặt
  `ocr-dist`). Có thì sinh `$OUT_DIR/ocr_embed.rs` với `include_bytes!` cả hai file và SHA-256 của exe; không đặt thì
  `EMBEDDED = None` (build tay, clippy, job `check`). Đặt mà thiếu file thì build lỗi.
- **Lõi** (`core::ocr::helper::Embedded`): `ensure(ocr_root)` giải nén vào `<ocr>/helper-<16 hex đầu của hash>/` qua
  thư mục `.tmp`, đọc lại kiểm SHA-256 (phần mềm diệt virus có thể cắt file), rồi rename. Đã có đủ (so kích thước) thì
  thôi. `pack::cleanup_old` nhận thêm danh sách giữ lại để không xoá thư mục engine của bản đang chạy; engine của bản cũ
  thì bị dọn khi khởi động.
- **App** (`ocr.rs`):
  - `vi_state`: có `chup-ocr.exe` cạnh exe **hoặc** có bản nhúng thì không còn `NoHelper`.
  - Chạy engine: bản cạnh `chup.exe` trước (bộ cài đặt ở `Program Files`-kiểu thư mục, ít bị chính sách chặn hơn
    `%LOCALAPPDATA%`). Không có, hoặc nó trả PROTO khác (chỉ thay mỗi `chup.exe`), thì giải nén bản nhúng rồi chạy.
  - Giải nén **lúc cần** (job OCR tiếng Việt đầu tiên, trên thread OCR), không làm lúc khởi động: người không dùng
    tiếng Việt không bị ghi 6 MB.
  - Giải nén lỗi: toast "Không giải nén được engine OCR tiếng Việt. Phần mềm diệt virus có thể đã chặn, hãy thử lại."
- **Gỡ cài đặt**: đã xoá cả `%LOCALAPPDATA%\app.chup.desktop\ocr`, gồm thư mục engine.
- **License**: `NOTICE-ocr.txt` luôn được giải nén cạnh exe.

## Chưa kiểm chứng

- Chạy thật với `chup-ocr.exe` của CI (máy dev không có Tesseract). Test lõi dùng byte giả.
- Symantec Endpoint (máy test) có chặn exe vừa ghi vào `%LOCALAPPDATA%` không.
