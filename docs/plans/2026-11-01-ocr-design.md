# Thiết kế Phase 2.3: OCR (copy chữ từ ảnh)

Ngày: 2026-11-01. Đặc tả: SPEC mục 15 (cập nhật theo tài liệu này). Spike: `2026-11-01-ocr-spike.md`.

## Người dùng chốt

- 2 lựa chọn ngôn ngữ: **"Tiếng Anh"** dùng Windows OCR có sẵn (mặc định, không tải gì); **"Tiếng Việt + Anh"**
  dùng Tesseract 5 `vie_fast+eng_fast`.
- Gói tiếng Việt **chọn khi cài, không nhúng vào bộ cài**: bộ cài có ô "OCR tiếng Việt", mặc định **bỏ trống**;
  tích thì tải gói trong lúc cài.
- Không tích lúc cài thì **vẫn tải được trong app** (Cài đặt, hoặc khi bấm [T] lần đầu).
- Gói tải **từ GitHub cộng đồng** (repo Chup private và không phát hành OCR, chỉ trỏ tới nguồn):
  dữ liệu ngôn ngữ lấy ở `tesseract-ocr/tessdata_fast`.
- GitHub cộng đồng không có bản Tesseract gọn cho Windows (chỉ bộ cài 50 MB của UB Mannheim), nên engine
  **`chup-ocr.exe` (Tesseract link tĩnh) do CI của Chup build và nằm trong bộ cài**, cạnh `chup.exe` (+~1,5–2 MB nén).
  Tesseract chạy trong exe phụ đó, không nạp vào `chup.exe`.
- Các chi tiết còn lại do Claude chốt (người dùng dặn làm luôn, không hỏi).

## 1. Engine, dữ liệu và phân phối

**Engine** `chup-ocr.exe`: crate `crates/ocr-helper`, Tesseract + Leptonica link tĩnh (vcpkg `x64-windows-static`,
CRT tĩnh như `chup.exe`). CI build cùng `chup.exe`, bộ cài chép vào `$INSTDIR` kèm `NOTICE-ocr.txt` (license Tesseract
Apache-2.0, Leptonica BSD-2 và các thư viện ảnh kèm theo). Artifact CI có cả hai exe.

**Gói tiếng Việt** = 2 file dữ liệu từ [tesseract-ocr/tessdata_fast](https://github.com/tesseract-ocr/tessdata_fast)
(Apache-2.0), ghim commit `87416418657359cb625c412a48b6e1d6d41c29bd` (2024-08-01):

| File | Byte | SHA-256 |
|---|---|---|
| `vie.traineddata` | 531 275 | `79df64caf7bcfb2a27df5042ecb6121e196eada34da774956995747636d5bfa1` |
| `eng.traineddata` | 4 113 088 | `7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2` |

- URL: `https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/<commit>/<file>`. Manifest (commit, tên, kích
  thước, SHA-256) ghi cứng trong `core::ocr::pack`. Đổi dữ liệu thì tăng số gói (`vi-2`) và đổi manifest.
- **Vị trí**: `%LOCALAPPDATA%\app.chup.desktop\ocr\vi-1\`.
- **Cài**: tải từng file vào `ocr\vi-1.tmp\`, kiểm tra kích thước và SHA-256 từng file, xong hết thì rename
  `vi-1.tmp` -> `vi-1`. Lỗi bất kỳ bước nào: xoá `vi-1.tmp`. Bắt đầu tải mà còn `vi-1.tmp` cũ thì xoá trước.
- **"Đã có gói"** = thư mục `vi-1` có đủ file đúng kích thước (rẻ, kiểm tra mỗi lần OCR). SHA-256 chỉ kiểm tra lúc cài.
  **"Dùng được tiếng Việt"** = đã có gói **và** có `chup-ocr.exe` cạnh `chup.exe` (chạy `chup.exe` rời không qua bộ
  cài thì báo "Thiếu chup-ocr.exe, hãy cài Chup bằng bộ cài").
- **Dọn**: khởi động thì xoá các thư mục `ocr\vi-*` khác bản hiện tại (gói cũ sau khi nâng cấp).
- **Tải bằng WinHTTP** (crate `windows`, không thêm thư viện TLS), theo redirect. SHA-256 bằng crate `sha2` (Rust thuần;
  logic cài nằm ở core nên test được trên macOS với fetcher giả).
- **Một đường tải duy nhất**: `chup.exe --install-ocr` mở cửa sổ "Tải gói OCR tiếng Việt" (thanh tiến trình, nút Huỷ),
  tải xong thì thoát. Exit code: 0 xong, 1 lỗi, 2 huỷ. Chế độ này **bỏ qua khoá 1 instance**, không tạo tray/hotkey.
  Trong app, nút Cài đặt và câu hỏi khi bấm [T] mở **cùng cửa sổ đó** (chạy trong process app).
- **Tải xong** (cả từ bộ cài lẫn từ app): đặt `ocr.language = "vi"` trong config, vì người dùng vừa chủ động chọn.
- **Bộ cài (NSIS)**:
  - Luôn chép `chup.exe`, `chup-ocr.exe`, `NOTICE-ocr.txt`.
  - Thêm trang Components: section bắt buộc "Chup" và section tuỳ chọn "OCR tiếng Việt (tải thêm ~4,4 MB)",
    mặc định **bỏ trống** (`/o`).
  - Tích: chép file xong thì chạy `"$INSTDIR\chup.exe" --install-ocr` và đợi. Exit code 1: MessageBox
    "Không tải được gói OCR tiếng Việt. Có thể tải lại trong Cài đặt của Chup." rồi vẫn cài xong. Code 2: bỏ qua.
  - Gỡ cài đặt: xoá `chup-ocr.exe`, `NOTICE-ocr.txt` và `$LOCALAPPDATA\app.chup.desktop\ocr`.

## 2. Giao tiếp chup.exe <-> chup-ocr.exe

- **Khởi chạy** (lần đầu cần OCR tiếng Việt): `chup-ocr.exe --tessdata <thư mục gói>` (exe nằm cạnh `chup.exe`), stdin/stdout là pipe,
  `CREATE_NO_WINDOW`, gắn vào Job Object `KILL_ON_JOB_CLOSE` (Chup chết thì exe phụ chết theo).
- Exe phụ nạp model `vie+eng` **một lần**, rồi gửi lời chào `{"proto":1,"tesseract":"5.5.x"}`.
  Sai `proto` -> ghi log, báo "Hãy cài lại Chup" (exe phụ lệch bản với `chup.exe`).
- **Khung tin**: `u32` little-endian (độ dài) + nội dung. Crate `crates/ocr-proto` (không phụ thuộc OS) chứa phần này,
  dùng chung cho app và exe phụ, unit test được.
  - Yêu cầu: header nhị phân `id: u32, width: u32, height: u32` + pixel **xám 8-bit** (width × height byte).
  - Trả lời: JSON `{"id", "lines":[{"text","bbox":[x,y,w,h],"conf"}]}` hoặc `{"id","error"}`.
- Exe phụ **chỉ nhận diện**: `TessBaseAPISetImage` với buffer xám, duyệt `RIL_TEXTLINE` lấy text/bbox/conf.
  Chuyển xám, phóng ×2, sắp dòng, NFC, lọc rác đều ở phía Chup (core, thuần Rust).
- **FFI viết tay** tới C API của Tesseract (~18 hàm), không dùng bindgen. Link: Windows lấy mọi `.lib` trong thư mục
  `lib` của vcpkg (biến `VCPKG_ROOT`); macOS dùng `pkg-config tesseract` (chỉ để dev/test, Homebrew).
- `crates/ocr-helper` **nằm ngoài workspace** (`exclude`): `cargo test --workspace` không cần Tesseract.
- **Vòng đời**:
  - Giữ process sống giữa các lần OCR (không nạp lại model).
  - 5 phút không dùng: kill exe phụ (không giữ trạng thái gì nên kill là đủ).
  - Huỷ hoặc thay job tiếng Việt đang chạy: kill process; lần sau chạy lại (tốn thêm ~0,6 s nạp model).
    Job tiếng Anh (Windows OCR, vài chục ms) thì để chạy xong, bỏ kết quả.
  - Thời gian tối đa mỗi job: 15 s + 2 s mỗi megapixel ảnh gửi đi. Không phóng ×2 nếu ảnh sau phóng > 12 MP.
  - Exe phụ chạy với thư mục hiện tại là thư mục gói và `--tessdata .`: Tesseract mở file bằng `fopen` (ANSI),
    đường dẫn tuyệt đối có dấu (tên user tiếng Việt) sẽ hỏng. Exe phụ cũng có manifest code page UTF-8.
  - Crash, đứt pipe, hoặc quá thời gian: kill, ghi log, báo "OCR lỗi, hãy thử lại".
- **Windows OCR** (tiếng Anh): gọi WinRT `Windows.Media.Ocr` trong `chup.exe`, cùng worker thread. Không có
  confidence (đặt 1.0). Ảnh lớn hơn `OcrEngine.MaxImageDimension` thì thu nhỏ. Không tạo được engine en-US thì thử
  ngôn ngữ của user profile; vẫn không được: báo "Windows chưa có OCR tiếng Anh. Hãy chọn Tiếng Việt + Anh trong Cài đặt."
- **Worker OCR**: 1 thread, giữ cả 2 engine. Một ô job chờ + Condvar (không dùng channel, để Huỷ xoá được job chờ):
  job mới thay job đang chờ và dừng job đang chạy. Kết quả về UI thread bằng `win::emit(Action::OcrDone{..})`.
- **Hộp thoại tải, bấm Huỷ**: đóng ngay, không đợi thread tải (mạng chậm có thể kẹt ở bước kết nối tới vài chục
  giây); thread đó dừng ở đoạn kế tiếp và tự xoá thư mục tạm, trong lúc đó lần tải mới báo "đang dừng".

## 3. Lõi, giao diện, lỗi

### Config

```json
"ocr": { "language": "en" },
"hotkeys": { ..., "ocr": "" }
```

- `ocr.language`: `en` | `vi`. Field mới có serde default nên vẫn version 2. `hotkeys.ocr` ("Chụp text") mặc định rỗng.

### core

- `core::ocr` (thuần Rust):
  - `prepare(rgba, scale, engine) -> GrayImage`: chuyển xám; **Tesseract và scale < 1,5 thì phóng ×2** (CatmullRom).
    Windows OCR không phóng (spike: đọc chữ nhỏ vẫn đúng).
  - `postprocess(lines) -> String`: bỏ dòng rác (không có chữ/số, hoặc ≤ 2 ký tự và conf < 50); sắp từ trên xuống,
    trái sang phải, hai bbox chồng nhau theo chiều dọc > 50% thì nối cùng hàng bằng dấu cách; chuẩn hoá **NFC**
    (crate `unicode-normalization`); bỏ khoảng trắng thừa cuối dòng.
  - `pack`: manifest gói, `PackState::{Missing, Installed}`, `install(fetcher, dir, progress, cancel)` với trait
    `Fetch` (WinHTTP ở app, giả lập trong test), dọn thư mục cũ.
- `Shot` thêm `scale: f64` (scale màn hình lúc chụp) để biết có cần phóng ×2.
- `Model`:
  - `ocr_request(OcrSource) -> Vec<Effect>`: `OcrSource::{File{path, scale}, Region(Rect), Freeze(Rect)}`.
    - Đang bận job khác: thay job (worker tự huỷ job cũ).
    - `vi` mà chưa có gói: giữ `pending_ocr`, trả `Effect::AskOcrDownload`.
    - Còn lại: `Effect::StartOcr(OcrJob{id, source, lang})`, `ocr_busy = Some(id)`.
  - `ocr_done(id, Result<String, String>)`: id cũ thì bỏ qua. Có chữ: `Effect::CopyText(text)` + toast
    "Đã copy N dòng: “dòng đầu…”". Rỗng: toast "Không tìm thấy chữ" (clipboard giữ nguyên). Lỗi: toast lỗi.
  - `ocr_cancel()`: `Effect::CancelOcr`, `ocr_busy = None`.
  - `ocr_pack_installed()`: đặt `language = vi`, chạy `pending_ocr` nếu có. `ocr_pack_failed/cancelled`: bỏ `pending_ocr`.
  - Cache: nhớ kết quả cuối `(path, lang) -> text`. Bấm [T] lần 2 trên cùng file thì copy luôn, không OCR lại.
  - `FreezeAction::Ocr` -> `Effect::FreezeOcr(rect)`.
- Effect `OcrBusyChanged` để Preview vẽ nút [T] "bận".
- Hộp thoại tải (TaskDialog có thanh tiến trình) là modal: App gửi `WM_APP_OCR_DOWNLOAD` cho cửa sổ chính, cửa sổ chính
  mở hộp thoại ngoài lúc `App` bị borrow, xong thì `emit(Action::OcrPack(kết quả))`.

### Các chỗ gọi OCR (theo thứ tự làm)

1. **Preview [T]**: toolbar Preview thêm [T] đầu tiên. Đang OCR thì [T] thành vòng "bận" (đứng yên, nền màu nhấn);
   bấm lần nữa = huỷ.
   Ảnh lấy từ file đang xem (Preview đã giải mã sẵn trong memory), scale tra theo shot trong stack (không có thì 1,0).
2. **Freeze [T Chữ]**: toolbar trong overlay thêm nút "Chữ": crop từ ảnh đóng băng -> OCR. Không lưu file, không vào stack.
3. **Hotkey "Chụp text"** (mặc định chưa gán): mở overlay Freeze ở chế độ OCR; thả chuột là OCR luôn, không hiện toolbar.
4. **Toolbar Live [T]**: chụp region (`grab_region`, không `persist_shot`) -> OCR, giữ vùng chọn. Toolbar rộng thêm
   1 nút icon (252 × 44 DIP, Mờ 148 × 40).

### Cài đặt

Nhóm "Copy chữ từ ảnh (OCR)":
- Ngôn ngữ: "Tiếng Anh (có sẵn trong Windows)" / "Tiếng Việt + Anh (cần tải ~4,4 MB)".
- Trạng thái gói: "Đã cài" + nút "Xoá gói", hoặc "Chưa cài" + nút "Tải gói (~4,4 MB)".
- Hotkey "Chụp text".

### Lỗi hiện cho người dùng (tiếng Việt; chi tiết kỹ thuật chỉ ghi log)

| Tình huống | Hiện |
|---|---|
| Không có chữ | toast "Không tìm thấy chữ", clipboard giữ nguyên |
| Mất mạng / HTTP lỗi lúc tải | trong cửa sổ tải: "Không tải được gói. Kiểm tra mạng rồi thử lại." + nút Thử lại |
| Sai checksum | như trên ("File tải về bị lỗi") — đã xoá phần dở dang |
| Gói thiếu file | hỏi tải lại gói |
| Thiếu `chup-ocr.exe` / sai proto | toast "Hãy cài lại Chup bằng bộ cài" |
| Exe phụ crash / quá 15 s | toast "OCR lỗi, hãy thử lại" |
| Windows không có OCR en | toast gợi ý chọn Tiếng Việt + Anh |

## 4. Test

- **core (macOS)**: `prepare` (phóng/không phóng), `postprocess` (sắp hàng, nối hàng, lọc rác, NFC), `pack`
  (cài thành công, sai checksum, huỷ giữa chừng, dọn thư mục cũ, nhận diện gói thiếu file), Model OCR (gói thiếu ->
  hỏi -> cài xong chạy tiếp, job cũ bị bỏ qua, cache, rỗng), config.
- **ocr-proto**: encode/decode khung, header, JSON trả lời.
- **ocr-helper (macOS, Homebrew Tesseract)**: test tích hợp gửi ảnh spike qua pipe, so CER ≤ 3%.
- **CI**: build `chup-ocr.exe` (vcpkg static, có cache) trong job Windows, đưa vào artifact và bộ cài.
- **VM**: SPEC 15.8 + cài với ô OCR tích/không tích, mất mạng lúc cài, gỡ cài đặt xoá gói.

## 5. Thứ tự làm

1. `ocr-proto` + `core::ocr` (prepare, postprocess) + config.
2. `ocr-helper` (FFI, vòng lặp pipe), test trên macOS.
3. `core::ocr::pack` + Model OCR.
4. Windows: worker + Windows OCR + Tesseract client, Preview [T].
5. Windows: WinHTTP + cửa sổ tải + `--install-ocr` + Cài đặt.
6. CI build `chup-ocr.exe` + NSIS (chép exe phụ, ô OCR tiếng Việt, gỡ cài đặt).
7. Freeze [T], hotkey "Chụp text", toolbar Live [T].
8. Test trên VM.
