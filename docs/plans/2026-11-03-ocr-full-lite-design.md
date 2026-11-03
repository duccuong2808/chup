# Thiết kế: bộ cài Full / Lite, engine OCR tải từ GitHub Release (chup-0oa)

Ngày 2026-11-03. Thay cho `2026-11-03-ocr-embed-design.md` (nhúng engine vào `chup.exe`, chup-7bc).

## Yêu cầu (user)

- **Full**: có đủ như hiện tại, gồm OCR tiếng Việt.
- **Lite**: lúc cài có ô tải OCR tiếng Việt; không tick thì tải được sau trong Cài đặt.
- Engine đặt ở chính repo Chup (đã chuyển public).
- Không muốn `chup.exe` nặng 8 MB vì mang engine bên trong.

## Quyết định

- **Một `chup.exe` cho cả hai bộ cài**, không nhúng engine (~1,5 MB). Khác nhau chỉ ở bộ cài.
- **Engine phát hành tay** ở Release `ocr-engine-vN` của repo Chup, không build lại mỗi lần. Lý do: build MSVC không
  lặp lại được từng byte, mà app cần biết trước kích thước và SHA-256 để kiểm file tải về. Engine hiếm khi đổi
  (chỉ khi sửa `crates/ocr-helper` / `ocr-proto`). Quy trình ở README, mục "Phát hành engine OCR".
- **Manifest JSON** `crates/core/src/ocr/manifest.json` là nguồn duy nhất cho hai gói `data` và `engine`
  (thư mục, URL gốc, tên/kích thước/SHA-256 từng file). Core `include_str!` + serde; CI và `installer/fetch-ocr.ps1`
  đọc cùng file. Bộ cài Full lấy đúng file trong manifest (kiểm SHA-256), không lấy engine vừa build, nên Full và Lite
  luôn dùng cùng một engine.
- **Full**: `chup-ocr.exe` + `NOTICE-ocr.txt` cạnh `chup.exe`; dữ liệu chép thẳng vào
  `%LOCALAPPDATA%\app.chup.desktop\ocr\vi-<N>\`. Không tự chuyển ngôn ngữ OCR sang tiếng Việt (người dùng chọn trong
  Cài đặt). **Lite**: ô "OCR tiếng Việt" (mặc định bỏ trống) chạy `chup.exe --install-ocr`; xong thì chọn tiếng Việt.
  Lite cài đè Full thì xoá engine cạnh exe (Lite luôn dùng engine theo manifest của chính nó).
- **App** (`ocr.rs`): thiếu gì thì tải nấy (`missing_packs`): engine khi không có bản cạnh `chup.exe`, dữ liệu khi chưa
  có. Nút trong Cài đặt ghi dung lượng còn thiếu. Chạy engine: bản cạnh exe trước; khác PROTO thì bản đã tải.
  Bỏ trạng thái `NoHelper`: thiếu engine giờ cũng chỉ là "cần tải".
- `pack::cleanup_old` giữ đúng hai thư mục hiện tại (`vi-N`, `engine-vN`), dọn `helper-*` của bản nhúng cũ.
- Tải bằng WinHTTP: link Release trả 302 sang `release-assets.githubusercontent.com`, WinHTTP tự theo (đã thử).

## Đã thử / chưa thử

Đã thử trên Windows 11 x64 thật: `--install-ocr` tải engine đúng SHA-256, engine chạy từ `%LOCALAPPDATA%`.
Chưa thử: hai bộ cài NSIS (CI build), Lite tick OCR khi cài, Full trên máy không mạng.
