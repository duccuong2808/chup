# Thiết kế: Preview chọn chữ trên ảnh (kiểu Live Text)

Ngày: 2026-11-01. Beads: chup-4fx. Đặc tả: SPEC 15.2, 15.4, 17.2 (cập nhật theo tài liệu này).

## Người dùng chốt

- Trong Preview, chữ trên ảnh chọn được bằng chuột rồi copy (Ctrl+C, chuột phải), thay vì chỉ copy cả khối.
- **Giữ** các đường copy thẳng: [T] trên Preview và toolbar Live, Freeze "Chữ", hotkey "Chụp text".
- Cài đặt "Tự nhận diện chữ khi mở Preview", **mặc định bật** (`ocr.autoPreview`, config cũ cũng thành bật).
  Tắt thì chỉ nhận diện khi bấm [T] trong Preview.
- Kéo bắt đầu trên chữ là chọn chữ, ở chỗ khác là di chuyển cửa sổ; `Alt` + kéo luôn di chuyển.
- **Chọn theo ký tự như Live Text trên macOS** (người dùng yêu cầu khi đang làm; bản đầu chọn theo từ).
- Chi tiết còn lại do Claude chốt.

## Phương án

Vẽ vùng chọn ngay trên cửa sổ Preview (layered window, cùng chỗ vẽ toolbar). Hit-test, chọn và ghép chữ nằm ở
core (unit test được). Loại: phủ ô EDIT/RichEdit trong suốt (chữ lệch ảnh, khó đúng khi zoom); ô kết quả riêng
cạnh Preview (không phải chọn trên ảnh).

Chọn theo **ký tự**: con trỏ chữ là `Pos { word, ch }` (trước ký tự `ch` của từ `word`).
- Tesseract trả vị trí từng ký tự (`RIL_SYMBOL`). Ảnh test tiếng Việt: 98/98 từ khớp số ký tự sau NFC.
- Windows OCR chỉ có bbox từ: chia bề rộng từ theo độ rộng ước lượng của ký tự (i, l, dấu câu hẹp; m, w rộng;
  chữ hoa rộng hơn). Tesseract trả lệch số ký tự cũng dùng cách này.

## 1. Engine và dữ liệu

- `ocr-proto`: `Line` thêm `words: Vec<Word{text, bbox, conf, chars}>` (serde default rỗng; `chars` = `[x, w]` từng
  ký tự, rỗng nếu engine không có). `PROTO` = 2
  (`chup-ocr.exe` luôn đi cùng bộ cài, không cần tương thích bản cũ).
- `chup-ocr.exe`: duyệt `RIL_SYMBOL`; `IsAtBeginningOf(RIL_TEXTLINE)` thì mở dòng mới, `IsAtBeginningOf(RIL_WORD)`
  thì mở từ mới (text, conf, bbox lấy ở mức dòng / từ). Bbox từ tiếng Việt có dấu cao hơn bbox dòng.
- Windows OCR: `Words()` (đã có), mỗi từ một `Word`.
- **Bbox về toạ độ ảnh gốc**: `ocr::prepare` trả thêm hệ số phóng (2 hoặc 1); Windows OCR tự thu nhỏ ảnh quá
  `MaxImageDimension` thì nhân bbox ngược lại về ảnh đã chuẩn bị. `TextLayout::build(lines, factor)` chia hệ số.
- `core::ocr::TextLayout` (thuần Rust):
  - `build`: bỏ dòng rác (luật cũ), gom hàng (luật cũ: chồng dọc > 50%), sắp hàng trên xuống, dòng trong hàng
    trái sang phải; mỗi từ NFC; dòng không có `words` thì cả dòng là một từ.
  - `all_text()`: từ trong hàng nối bằng dấu cách, hàng nối bằng `\n`. `postprocess(lines)` = `build(lines, 1).all_text()`.
  - Mỗi từ có `edges`: x các ranh giới ký tự (số ký tự + 1).
  - `hit(p, pad) -> Pos`: điểm nằm trong bbox một dòng (nới `pad`) thì trả ranh giới ký tự gần nhất của từ gần nhất.
  - `nearest(p) -> Pos`: hàng gần nhất theo y, rồi từ và ranh giới gần nhất theo x (dùng khi đang kéo).
  - `text(a, b)`: chữ giữa hai `Pos` (thứ tự bất kỳ, lấy được một phần của từ).
  - `highlight(a, b)`: mỗi hàng một rect, từ con trỏ đầu tới con trỏ cuối trong hàng.
  - `word_range`, `row_range`, `all`: bấm đúp, bấm 3 lần, Ctrl+A.

## 2. Model

- `OcrJob.copy`: `true` = người dùng yêu cầu (xong thì copy, nút [T] quay "bận"); `false` = job nền cho Preview.
- `Model.preview_open: Option<PathBuf>`: `preview_shown(path)` (ảnh Preview đã giải mã xong), `preview_closed()`.
- `ocr_done(id, Result<TextLayout, String>)`. Cache `(file, ngôn ngữ) -> Arc<TextLayout>`.
- `Effect::PreviewText(path, Arc<TextLayout>)`: Preview nhận các từ của file đó.
- Luồng:
  - `preview_shown`: cache có thì `PreviewText`; không thì nếu `autoPreview` và chạy được im lặng (tiếng Anh,
    hoặc tiếng Việt `Ready`) và không có job người dùng đang chạy: job nền. Tiếng Việt thiếu gói hay
    `chup-ocr.exe`: không hỏi, không báo.
  - [T] (`ocr_request(Preview)`) lúc job nền của đúng file đang chạy: chỉ đổi `copy = true` (hiện "bận").
  - Job người dùng (Freeze, Chụp text, [T]) thay job nền. Mọi job xong mà Preview còn mở, chưa có chữ: chạy lại
    job nền (`autoPreview` bật). Huỷ bằng [T] thì không chạy lại.
  - `preview_closed`: huỷ job nền đang chạy (không huỷ job người dùng).
  - Job nền lỗi: không toast (worker đã ghi log). Job nền xong không có chữ: không báo gì.
- `OcrBusyChanged` chỉ khi trạng thái "bận" nhìn thấy được (job `copy`) đổi.

## 3. Preview

- State: `text: Option<Arc<TextLayout>>` (của file đang xem), `sel: Option<(Pos, Pos)>` (neo, đang kéo tới),
  `text_press: Option<(Pos, (i32, i32))>` (neo, điểm bấm), `last_dbl` (nhận ra bấm 3 lần). Đổi ảnh hoặc đóng: bỏ hết.
- Toạ độ cửa sổ -> ảnh: trừ `View::image_rect`, chia `zoom`. Nới hit-test 4 DIP.
- Chuột:
  - Ưu tiên: nút toolbar > mép (đổi cỡ) > chữ (không giữ `Alt`) > di chuyển cửa sổ.
  - Bấm trên chữ: `SetCapture`; kéo quá `SM_CXDRAG`/`SM_CYDRAG` thì bắt đầu chọn từ anchor tới `nearest`.
    Thả mà chưa kéo: bỏ vùng chọn. Bấm ra ngoài chữ (để di chuyển) cũng bỏ vùng chọn.
  - Bấm đúp trên chữ: chọn từ đó; bấm thêm lần nữa (trong thời gian bấm đúp): chọn cả hàng.
    Bấm đúp chỗ khác: như bấm đơn.
  - `Shift` + bấm trên chữ: nới vùng chọn đang có tới điểm bấm (kéo tiếp được).
  - Con trỏ: I-beam trên chữ (không giữ `Alt`).
  - Chuột phải: menu "Copy chữ" (mờ khi chưa chọn), "Chọn tất cả chữ" (mờ khi chưa có chữ), "Copy ảnh".
- Phím: `Ctrl+C` copy chữ đã chọn, chưa chọn thì copy ảnh. `Ctrl+A` chọn tất cả chữ. `Esc` bỏ vùng chọn,
  không có vùng chọn thì đóng.
- Vẽ: mỗi rect của `highlight` (đổi sang toạ độ cửa sổ) tô màu nhấn alpha 0,35, góc bo 2 DIP, trước toolbar.
- Copy chữ: `Action::CopyText(text)` -> `Effect::CopyText` (clipboard + toast "Đã copy N dòng…" như [T]).

## 4. Cài đặt

Nhóm OCR thêm ô "Tự nhận diện chữ khi mở Preview" (`ocr.autoPreview`, serde default `true`, vẫn config v2).

## 5. Test

- Core (macOS): `TextLayout` (gom hàng, hit/nearest ra đúng ký tự, text một phần từ xuôi/ngược, highlight, hệ số
  phóng, ước lượng ranh giới, NFC giữ đúng số ký tự, dòng không có words), `prepare` trả hệ số, Model (job nền, [T] nâng job nền, job người dùng thay rồi chạy lại
  job nền, đóng Preview huỷ job nền, tiếng Việt thiếu gói không hỏi, autoPreview tắt), config.
- `ocr-proto`: `Line` có/không có `words`.
- `ocr-helper` (macOS, Homebrew Tesseract): bbox từ nằm trong dòng, ≥ 90% từ có đủ vị trí ký tự.
- VM: chọn chữ tiếng Anh và tiếng Việt (một phần từ), zoom rồi chọn, kéo nhiều hàng, bấm đúp / 3 lần,
  Shift+bấm, Ctrl+A/C, menu chuột phải,
  `Alt`+kéo, Esc, tắt tự nhận diện rồi bấm [T].

## 6. Thứ tự làm

1. `ocr-proto` (words, PROTO 2) + `core::ocr` (`prepare` trả hệ số, `TextLayout`) + config.
2. `ocr-helper` duyệt từ, test trên macOS.
3. Model (job nền, cache layout, `PreviewText`).
4. Windows: worker (bbox về ảnh gốc, `TextLayout`), App, Preview (chọn, vẽ, phím, menu), Cài đặt.
5. SPEC, README; CI; test trên VM.
