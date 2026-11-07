# Chup — SPEC v0.3

> Tài liệu đầu vào để Claude Code implement.
>
> Lộ trình chia 3 phase:
> - **Phase 1** = v1 cho Windows. Bước 1.5 chuyển giao diện sang Win32 native, bỏ WebView (mục 12).
> - **Phase 2** = v2, gồm Freeze, preview, OCR và tự động chọn vùng.
> - **Phase 3** = mở rộng sang macOS và Linux.
>
> Các câu hỏi mở đã được chốt ở mục 13.

---

## 1. Mục tiêu

- Chụp **theo vùng** nhanh bằng phím tắt hoặc tray.
- Ảnh vừa chụp hiện thành **stack tối đa 5 thumbnail** ở cạnh màn hình, **kéo thả thẳng** sang app khác (Explorer, Chrome, Zalo, Slack, Word...).
- Vùng chọn có thể **giữ lại ở chế độ mờ** (click xuyên qua) để tiếp tục dùng app khác và chụp lại vùng đó bất cứ lúc nào.
- Người dùng chọn được **folder lưu ảnh**.
- **Gọn nhẹ**: chạy nền ở tray, không chiếm CPU khi idle.
- **Windows trước**. Kiến trúc sẵn sàng cho macOS và Linux (X11).

## 2. Ngoài phạm vi Phase 1

- Chế độ **Freeze** (chụp được menu, tooltip): làm ở Phase 2, xem mục 14.
- **OCR** copy text từ ảnh: làm ở Phase 2, xem mục 15.
- Chỉnh sửa / vẽ / annotate ảnh, làm mờ thông tin nhạy cảm.
- Quay video, GIF.
- Tự động chọn vùng (nhận diện cửa sổ, phần tử UI): làm ở Phase 2, xem mục 16.
- Chụp cuộn (scroll capture).
- Vùng chọn vắt qua nhiều màn hình (giới hạn trong 1 màn hình).
- Upload cloud / share link.
- Linux **Wayland** (lý do ở mục 11).
- Lưu stack qua lần khởi động lại (khởi động app thì stack rỗng).

## 3. Nền tảng & stack kỹ thuật

| Hạng mục | Lựa chọn |
|---|---|
| OS Phase 1 | Windows 10 (64-bit) và Windows 11 |
| Ngôn ngữ | Rust stable, **không WebView, không framework UI** |
| Giao diện Windows | Win32 qua crate `windows`: layered window tự vẽ (overlay, frame, toolbar, stack) bằng thao tác pixel + Direct2D **software** + DirectWrite; Cài đặt dùng control gốc (Common Controls v6) |
| Giao diện macOS, Linux (Phase 3) | AppKit (`objc2`), X11 (`x11rb`); dùng chung crate lõi và crate vẽ `ui` (trait `Canvas`: Direct2D / CoreGraphics + CoreText / Cairo) |
| OS Phase 3.1 | macOS 26 Tahoe trở lên (chi tiết mục 11) |
| Chụp màn hình | `xcap` (Windows, Linux); ScreenCaptureKit (macOS) |
| Kéo file ra app khác | `drag` (CrabNebula) |
| Phím tắt toàn cục | `global-hotkey` |
| Tray | `tray-icon` (menu bằng `muda`) |
| Chọn folder | `IFileOpenDialog` (COM) |
| Chạy 1 instance | Named mutex + tìm cửa sổ `ChupApp` |
| Khởi động cùng OS | Khoá `HKCU\...\Run`, giá trị `Chup` |
| Clipboard ảnh | `arboard` (ảnh đã có sẵn trong memory phía Rust) |
| Xử lý ảnh, thumbnail | `image` (PNG encoder dùng compression `Fast`) |
| Manifest | `embed-manifest` (DPI Per-Monitor v2, Common Controls v6, UTF-8) |
| Installer | NSIS tự viết (`installer/chup.nsi`), cài cho user hiện tại |

**Lưu ý khi implement:** pin version mọi crate trong `Cargo.toml` và kiểm tra lại signature API trên docs.rs. `xcap` đã đổi API giữa các version.

## 4. Thuật ngữ

- **Region**: vùng chọn, hình chữ nhật, toạ độ **physical px trên toàn desktop**.
- **Overlay**: lớp phủ để kéo chọn vùng, mỗi màn hình 1 cửa sổ.
- **Frame**: viền hiển thị quanh region sau khi chọn xong.
- **Toolbar**: popup nhỏ cạnh frame, gồm các nút Chụp / Mờ / Huỷ.
- **Stack**: dải thumbnail ở cạnh màn hình.
- **Shot**: 1 ảnh đã chụp (file PNG + thumbnail + metadata).
- **Live mode**: lấy ảnh tại thời điểm bấm Chụp (Phase 1).
- **Freeze mode**: lấy ảnh ngay khi nhấn hotkey, chọn vùng trên ảnh đóng băng (Phase 2).
- **Normal / Faded**: hai trạng thái của frame trong Live mode (mục 5.2).

## 5. Luồng người dùng (Live mode)

### 5.1 Chụp vùng (luồng chính)

1. Nhấn hotkey **Chọn vùng** (`Ctrl+Alt+A`) hoặc click trái icon tray.
2. Overlay hiện trên **mọi màn hình**:
   - Phủ tối 40%, con trỏ hình crosshair.
   - Khi đang kéo, hiển thị kích thước `W × H` (physical px) cạnh con trỏ.
3. Kéo chuột để chọn vùng.
   - Vùng bị giới hạn trong màn hình nơi bắt đầu kéo.
   - Vùng nhỏ hơn 4×4 px thì coi như huỷ.
4. Thả chuột:
   - Overlay ẩn đi.
   - Frame hiện quanh region, Toolbar hiện cạnh frame.
   - Frame ở **Normal mode**.
5. Bấm **[Chụp]** trên toolbar hoặc nhấn hotkey **Chụp** (`Ctrl+Alt+X`; trước v3 là `Ctrl+Alt+S`, hay bị app khác giữ):
   - Chụp nội dung **tại thời điểm bấm**.
   - Lưu PNG vào folder đã cấu hình.
   - Copy ảnh vào clipboard nếu bật cấu hình.
   - Thêm shot vào Stack.
6. Sau khi chụp:
   - Normal mode: đóng frame và toolbar.
   - Faded mode: **giữ nguyên** frame để chụp tiếp.
7. Huỷ:
   - Đang chọn vùng: nhấn `Esc`, hoặc click phải lên overlay.
   - Sau khi đã có frame: bấm **[✕]** trên toolbar.
   - **Không đăng ký `Esc` làm hotkey toàn cục** vì sẽ phá phím Esc của mọi app khác. `Esc` chỉ xử lý local khi overlay đang có focus.

### 5.2 Chế độ mờ (Faded mode)

- Bấm **[◐ Mờ]** trên toolbar để chuyển Normal sang Faded:
  - Frame: viền giảm opacity xuống 30%, **click-through** (chuột đi xuyên qua, dùng app bên dưới bình thường).
  - Toolbar: thu gọn, opacity 40%, hiện rõ 100% khi hover. Toolbar **không** click-through.
- Hotkey Chụp vẫn hoạt động, chụp region hiện tại bao nhiêu lần cũng được.
- Bấm **[◐]** lần nữa để về Normal mode (cho phép chỉnh lại vùng).
- Hotkey Chụp khi **không có frame**: chụp lại region cuối cùng nếu vẫn còn hợp lệ (màn hình chưa bị đổi). Nếu không có region nào thì bỏ qua.

### 5.3 Chỉnh vùng (Normal mode)

- 8 handle resize nằm **ngoài** viền.
- Kéo viền để di chuyển frame.
- Phần bên trong region **hoàn toàn trong suốt** và không tương tác.

### 5.4 Stack

- **Vị trí mặc định**: cạnh phải màn hình chính, căn dưới, nằm trong **work area** (không đè taskbar).
- **Tối đa 5 shot**. Shot thứ 6 vào thì shot cũ nhất bị đẩy khỏi stack. **File trên ổ đĩa vẫn giữ nguyên.**
- **Thứ tự**: shot mới nhất nằm sát góc màn hình (dưới cùng của stack).
- **Hiển thị**: luôn hiện khi có shot, không tự ẩn. Có nút thu gọn thành 1 tab nhỏ ở cạnh màn hình.
- **Kích thước thumbnail**:
  - Rộng 160 DIP, cao theo tỉ lệ ảnh, tối đa 120 DIP.
  - Ảnh quá dài hoặc quá cao thì crop ở giữa (`object-fit: cover`).
- **Tương tác với thumbnail**:
  - **Kéo** (giữ chuột và di ≥ 4px): kéo file PNG ra ngoài (native drag).
  - **Click**: mở ảnh trong cửa sổ Preview (mục 17).
  - **Hover**: hiện các nút nhỏ:
    - Copy: copy ảnh vào clipboard.
    - Folder: mở Explorer và chọn sẵn file.
    - ✕: bỏ khỏi stack (không xoá file).
- **Header** (chỉ hiện khi hover): nút **Xoá hết**, chỉ clear stack, không xoá file.
- **Hành vi cửa sổ**:
  - **Không bao giờ cướp focus** của app đang dùng.
  - Không hiện trên taskbar, luôn nằm trên cùng (always on top).
  - Kích thước cửa sổ **co giãn đúng theo nội dung** để không chắn click ở vùng trống.
  - Stack rỗng thì ẩn cửa sổ.

### 5.5 Tray

- **Click trái**: Chọn vùng.
- **Click phải** mở menu, chia hai phần có tiêu đề mờ (chup-289), phím tắt đang gán hiện ở cột phải:
  - **Chụp**: Chọn vùng · Chụp lại vùng cuối · Chụp đóng băng, Chụp đóng băng sau 3 giây (Phase 2, mục 14.5) ·
    Mở folder ảnh
  - **Tuỳ chọn**: ☑ Phím tắt chụp đóng băng (F1) (bật/tắt hotkey Freeze, giống ô trong Cài đặt) · Cài đặt…
  - Thoát
- **Tooltip**: hiển thị "Chup". Nếu có hotkey đăng ký lỗi thì kèm cảnh báo.

### 5.6 Cài đặt

Cửa sổ thường, có trên taskbar. Chỉ tạo khi mở, huỷ khi đóng. Thay đổi được lưu ngay khi sửa, không có nút Save.

**Ngôn ngữ giao diện** (chup-81y): tiếng Việt và tiếng Anh, chuỗi ở `core::i18n`. Mặc định theo ngôn ngữ giao diện
của Windows (tiếng Việt nếu Windows là tiếng Việt, còn lại tiếng Anh); đổi trong Cài đặt có hiệu lực ngay (tray,
Cài đặt, toast mới). Ngôn ngữ OCR là lựa chọn riêng. Bộ cài cũng tự chọn tiếng Việt / tiếng Anh theo Windows.

Bố cục (chup-ck1, 2026-11-03) kiểu Cài đặt của Windows 11: bên trái là danh sách trang, bên phải là tiêu đề trang và các
thẻ nền trắng trên nền xám. Cửa sổ cỡ cố định theo trang cao nhất; mở lại trong cùng phiên thì về trang lần trước.

| Trang | Nội dung |
|---|---|
| Chung | Folder lưu ảnh, tên file; ngôn ngữ giao diện (Theo Windows / Tiếng Việt / English); khởi động cùng Windows |
| Phím tắt | 4 ô ghi phím (chọn vùng, chụp vùng hiện tại/cuối, Freeze, chụp text), báo lỗi ngay dưới ô; ô bật/tắt phím Freeze |
| Chụp ảnh | Tự động chọn vùng (SPEC 16); sau khi chụp (copy clipboard, giữ/đóng vùng chọn); ẩn toolbar/stack lúc chụp |
| Copy chữ (OCR) | Ngôn ngữ, trạng thái + nút tải/xoá gói tiếng Việt, tự nhận diện khi mở Preview |
| Stack và Preview | Cạnh và màn hình của stack; các mục Preview (SPEC 17.4) |
| Giới thiệu | Tên, phiên bản, commit (7 ký tự đầu; build tay lấy từ git, đánh dấu "bản build tay"), thời điểm build (UTC), mô tả; nút Trang dự án, Mở thư mục log; nguồn engine/dữ liệu OCR tiếng Việt, nút Giấy phép (`NOTICE-ocr.txt`) |

Các mục:
- **Folder lưu ảnh**: nút chọn folder (dialog), kèm đường dẫn hiện tại.
- **Pattern tên file.**
- **Hotkey**:
  - Ô ghi phím: bấm tổ hợp để set.
  - Báo lỗi nếu tổ hợp đã bị app khác chiếm.
- **Copy vào clipboard sau khi chụp**: bật/tắt.
- **Vị trí stack**: cạnh trái/phải; màn hình chính hoặc màn hình đang có con trỏ.
- **Hành vi sau khi chụp ở Normal mode**: đóng frame / giữ frame.
- **Khởi động cùng Windows**: bật/tắt.

## 6. Kiến trúc

### 6.1 Nguyên tắc

- **Lõi không phụ thuộc OS** (`crates/core`): hình học, config, chụp/lưu, stack, ghi phím, icon vector và state machine `Model`. `Model` nhận sự kiện, trả về danh sách `Effect`; không biết cửa sổ là gì, nên test được trên mọi máy.
- **Vỏ theo từng OS** (`crates/app/src/platform/<os>/`): tạo cửa sổ, vẽ, nhận chuột/phím, áp `Effect`.
- **Một UI thread** chạy message loop. Cửa sổ, tray, hotkey và worker không gọi thẳng vào App mà `emit(Action)`; App xử lý khi nhận `WM_APP_ACTION`. Nhờ vậy vòng lặp modal (kéo thả file, dialog) vẫn xử lý được Action, và không có khoá state.
- **I/O nặng chạy ở worker thread**: chụp, encode PNG, ghi file, copy clipboard.
- **Bộ nhớ đồ hoạ chỉ cấp khi cửa sổ đang hiện**: DIB của overlay (cỡ cả màn hình) giải phóng ngay khi ẩn.

### 6.2 Cửa sổ

| Label | Tạo lúc | Transparent | Always on top | Focus | Click-through | Taskbar |
|---|---|---|---|---|---|---|
| `overlay-{n}` | Khởi động (ẩn sẵn); tạo lại khi đổi màn hình | Có | Có | Có (cần nhận chuột/phím) | Không | Không |
| `frame` | Khởi động (ẩn) | Có | Có | Không | Chỉ ở Faded mode | Không |
| `toolbar` | Khởi động (ẩn) | Có | Có | Không | Không | Không |
| `stack` | Khởi động (ẩn) | Có | Có | **Không bao giờ** (`WS_EX_NOACTIVATE`) | Không | Không |
| `settings` | Khi mở | Không | Không | Có | Không | Có |

- **Overlay**:
  - Tạo **1 cửa sổ cho mỗi màn hình**, không tạo 1 cửa sổ phủ cả desktop. Lý do: màn hình khác DPI sẽ render sai.
  - Tạo sẵn ở trạng thái ẩn để hotkey phản hồi nhanh. Đây là trade-off: tốn thêm RAM để đổi lấy độ trễ thấp.
- **Frame và toolbar bắt buộc là 2 cửa sổ riêng**, vì click-through áp dụng cho cả cửa sổ.

### 6.3 Toạ độ và DPI (nguồn bug số 1)

- App chạy ở chế độ **Per-Monitor DPI Aware v2** (khai báo trong manifest).
- `Rect` luôn là **physical px trên toàn desktop**. Overlay của mỗi màn hình nhận toạ độ chuột physical, chỉ cần cộng gốc của màn hình; không còn bước quy đổi CSS px.
- Kích thước giao diện khai báo bằng DIP rồi nhân với scale của màn hình chứa cửa sổ (toolbar, frame theo màn hình của region; stack theo màn hình của nó).
- Frame window có rect = region **nới ra mỗi cạnh B px**, với `B = round(3 * scale)`. Viền được vẽ trong phần nới ra, nên **ảnh chụp không bao giờ dính viền**.

### 6.4 Không để UI của app lọt vào ảnh chụp

Toolbar hoặc stack có thể nằm đè lên region (ví dụ region sát cạnh phải, nơi stack đang hiện).

- **Windows**: gọi `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` cho `frame`, `toolbar`, `stack`.
  - Yêu cầu Windows 10 bản 2004 trở lên.
  - **Phải verify** cách này có tác dụng với cách `xcap` chụp trên Windows.
- **Fallback** (OS cũ, hoặc verify thất bại): ẩn các cửa sổ đó, chờ 1 frame (~50ms), chụp, rồi hiện lại.
- **macOS** (Phase 3): chụp bằng ScreenCaptureKit với `SCContentFilter(excludingApplications:)` loại mọi cửa sổ của Chup. Không dùng `sharingType = .none` (không còn tác dụng với API chụp mới). Spike phải kiểm chứng; không được thì dùng fallback.

### 6.5 Cấu trúc thư mục

```
chup/
├─ Cargo.toml              # workspace
├─ crates/
│  ├─ core/src/            # lõi, không phụ thuộc OS, có unit test
│  │  ├─ model.rs          # state machine: Model + Effect
│  │  ├─ coords.rs         # Rect, MonitorInfo, đặt toolbar/stack
│  │  ├─ capture.rs        # grab_region, persist_shot, thumbnail, tên file
│  │  ├─ config.rs         # load/save/migrate config.json, apply_patch
│  │  ├─ stack.rs          # tối đa 5 shot
│  │  ├─ hotkey_name.rs    # mã phím -> "Ctrl+Alt+A"
│  │  └─ icons.rs          # icon vector 24×24 (parse SVG path)
│  └─ app/
│     ├─ build.rs          # manifest + icon exe
│     └─ src/platform/windows/
│        ├─ mod.rs         # run(): single instance, message loop
│        ├─ app.rs         # Action -> Model -> Effect -> cửa sổ
│        ├─ win.rs         # đăng ký class, wndproc chung, hàng đợi Action
│        ├─ gfx.rs         # Surface (DIB), Direct2D, DirectWrite, theme
│        ├─ overlay.rs frame.rs toolbar.rs stack.rs settings.rs
│        └─ tray.rs hotkeys.rs util.rs logger.rs
└─ installer/chup.nsi
```

### 6.6 State

`Model` (crates/core/src/model.rs) giữ: config, danh sách màn hình, `selecting`, `region`, `last_region`, `mode` (Hidden/Normal/Faded), `frame_margin`, stack (≤5 shot), lỗi hotkey, `capture_exclusion_ok`. Shot giữ đường dẫn file, đường dẫn thumbnail (dùng làm ảnh khi kéo thả) và thumbnail đã giải mã trong RAM (để vẽ).

### 6.7 Action và Effect

- **Action** (vào App): `StartSelection`, `CancelSelection`, `RegionSelected(rect)`, `FrameMoved(outer)`, `ToggleFade`, `CloseRegion`, `Capture`, `Grabbed`, `Persisted`, `StackRemove/Clear`, `ShotOpen/Reveal/Copy/Missing`, `Menu(id)`, `Hotkey(id)`, `ConfigPatch(json)`, `Pause/ResumeHotkeys`, `DisplayChanged`...
- **Effect** (từ Model ra): `ShowOverlays`, `HideOverlays`, `ShowFrame`, `UpdateFrameMode`, `HideFrame`, `LayoutToolbar`, `HideToolbar`, `StackChanged`, `Toast`, `StartCapture`, `ApplyHotkeys`, `SyncAutostart`, `ConfigChanged`.
- Kéo thả: cửa sổ stack gọi thẳng `drag::start_drag` (file PNG + ảnh thumbnail). Chỉ kéo khi file còn; mất file thì báo App bỏ shot khỏi stack.
- Shot chỉ được đưa vào stack sau khi lưu file thành công.

### 6.8 Pipeline chụp

1. Nhận `capture_now` hoặc hotkey Chụp.
2. Lấy region. Nếu không có thì dùng `last_region`. Nếu vẫn không có thì bỏ qua.
3. Kiểm tra region còn nằm trong màn hình hiện có. Nếu không thì gửi `error` và huỷ region.
4. `grab_region()`: chụp bằng `Monitor::capture_region`, toạ độ tương đối theo màn hình, chạy ở worker thread. Trả về `RgbaImage`.
5. `persist_shot(img)`, chạy song song các việc sau:
   - Encode PNG (compression `Fast`), ghi vào `save_dir`.
   - Tạo thumbnail (rộng tối đa 320 px physical), ghi vào cache dir.
   - Copy clipboard nếu bật.
6. Đẩy shot vào stack (vượt 5 thì pop cái cũ nhất), vẽ lại và đặt lại cửa sổ stack.
7. Áp dụng hành vi sau khi chụp theo mode (mục 5.1 bước 6).

**Chuẩn bị cho Phase 2 (không tốn thêm công):**
- Giữ `grab_region()` và `persist_shot()` là 2 hàm tách riêng.
  - Freeze sẽ crop từ ảnh đóng băng rồi gọi lại `persist_shot()`.
  - OCR "Chụp text" sẽ dùng ảnh trong memory mà không ghi file.
- Overlay nhận field `mode` ngay từ Phase 1 (lúc này chỉ dùng `"live"`).

### 6.9 Tên file và folder

- **Folder mặc định**: `<Pictures>\Chup`. Lấy đường dẫn Pictures qua `SHGetKnownFolderPath`.
- **Pattern mặc định**: `chup-{yyyyMMdd-HHmmss-SSS}.png`.
- **Trùng tên**: thêm hậu tố `-1`, `-2`, …
- **Folder không tồn tại**: tự tạo.
- **Không ghi được** (mất quyền, ổ rút ra):
  - Ghi vào folder mặc định.
  - Gửi `error` báo rõ đường dẫn đã dùng.
  - **Không mất ảnh.**

### 6.10 Config

File: `%APPDATA%\app.chup.desktop\config.json` (giữ nguyên đường dẫn của bản Tauri để không mất cấu hình). Log ở `%LOCALAPPDATA%\app.chup.desktop\chup.log`. Có field `version` để migrate về sau.

```json
{
  "version": 2,
  "saveDir": "C:\\Users\\<user>\\Pictures\\Chup",
  "fileNamePattern": "chup-{yyyyMMdd-HHmmss-SSS}",
  "copyToClipboard": true,
  "hotkeys": { "selectRegion": "Ctrl+Alt+A", "capture": "Ctrl+Alt+X", "freeze": "F1", "freezeEnabled": true },
  "stack": { "edge": "right", "monitor": "primary" },
  "afterCaptureNormal": "close",
  "launchAtStartup": false
}
```

- Phase 2.2 thêm `preview` (mục 17.4); field mới có default nên vẫn là version 2.
- Phase 2.3 thêm `ocr: { "language": "en" }` (`en` = Windows OCR, `vi` = Tesseract tiếng Việt + Anh) và
  `hotkeys.ocr` ("Chụp text", mặc định rỗng); vẫn version 2. Sau đó thêm `ocr.autoPreview` (tự nhận diện chữ khi
  mở Preview, mặc định `true`, config cũ thiếu field cũng là `true`).
- Version 2 (Phase 2.1) thêm `hotkeys.freeze`. Migrate từ v1: điền `"F1"`; nếu F1 đã được dùng cho hotkey khác thì để trống (tắt).
- Version 3 (2026-11-03, chup-289): mặc định `hotkeys.capture` đổi từ `Ctrl+Alt+S` (hay bị app khác giữ) sang `Ctrl+Alt+X`.
  Migrate: còn đúng `Ctrl+Alt+S` thì đổi sang `Ctrl+Alt+X`, trừ khi phím đó đã gán cho hotkey khác. Thêm
  `hotkeys.freezeEnabled` (mặc định `true`): tắt thì không đăng ký hotkey Freeze (F1 trả lại cho app khác) mà vẫn nhớ phím.
- Version 4 (chup-81y): thêm `uiLanguage` (`system` | `vi` | `en`). Cài mới (chưa có file): `system`. Config cũ: `vi`,
  để người đang dùng không bị đổi sang tiếng Anh chỉ vì Windows của họ là tiếng Anh.
- Chuỗi rỗng = tắt hotkey đó. Hai hotkey không được trùng nhau.

Config hỏng hoặc parse lỗi: backup thành `config.bak.json`, rồi dùng mặc định.

## 7. Xử lý tình huống đặc biệt

| Tình huống | Hành vi |
|---|---|
| Hotkey bị app khác chiếm | Không crash. Tray tooltip cảnh báo, settings hiện lỗi cạnh ô hotkey |
| Mở app lần 2 | Instance cũ mở settings, instance mới thoát |
| Cắm/rút màn hình, đổi scale | Tạo lại các overlay. Region không còn hợp lệ thì đóng frame. Đặt lại vị trí stack |
| Đang chọn vùng thì nhấn hotkey Chọn vùng lần nữa | Bỏ qua |
| Nhấn hotkey Chọn vùng khi đang có frame | Đóng frame cũ, bắt đầu chọn vùng mới |
| Kéo thumbnail khi file đã bị xoá bên ngoài | Huỷ kéo, đánh dấu shot lỗi, tự bỏ khỏi stack |
| Màn hình khoá / UAC prompt | Chụp có thể ra ảnh đen. Chấp nhận |
| App fullscreen độc quyền (game) | Không đảm bảo chụp được. Chấp nhận |

## 8. Rủi ro và giới hạn đã biết

- **Live mode không chụp được menu hoặc tooltip đang mở.** Khi overlay hiện và nhận chuột, menu của app khác sẽ tự đóng. Giải quyết ở Phase 2 bằng **Freeze mode** (mục 14).
- **RAM**: bản Tauri dùng WebView2 tốn khoảng 125–150 MB (đo trên Windows 11, mục 9). Bước 1.5 bỏ WebView2 vì lý do này.
- **`WDA_EXCLUDEFROMCAPTURE` với `xcap`**: đã kiểm chứng trên Windows 11 là hoạt động. Vẫn giữ fallback (mục 6.4) cho Windows cũ.

## 9. Ngân sách hiệu năng (Phase 1)

Mục tiêu; đo thực tế ở bước 1.4 rồi chỉnh lại.

| Chỉ số | Mục tiêu | Cách đo |
|---|---|---|
| CPU khi idle | ~0% | Task Manager, 5 phút |
| RAM idle (private working set, cột Memory của Task Manager) | ≤ 100 MB | Task Manager |
| Từ nhấn hotkey đến lúc overlay hiện | ≤ 150 ms | Log timestamp |
| Từ bấm Chụp đến lúc thumbnail hiện (vùng 1920×1080) | ≤ 300 ms | Log timestamp |
| Dung lượng installer | ≤ 10 MB | File NSIS |

## 10. Tiêu chí nghiệm thu Phase 1 (test thủ công)

- [ ] Chọn vùng, chụp: file PNG đúng kích thước physical px, không dính viền, toolbar hay stack.
- [ ] Hai màn hình khác scale (100% và 150%): chọn vùng trên từng màn hình đều chụp đúng pixel.
- [ ] Faded mode: click, gõ phím, cuộn ở app bên dưới bình thường. Hotkey Chụp vẫn chụp đúng vùng.
- [ ] Chụp 7 lần liên tiếp: stack chỉ còn 5 ảnh mới nhất, đủ 7 file trên ổ đĩa.
- [ ] Kéo thumbnail thả vào được các đích sau: Explorer (copy file), ô upload của Chrome hoặc Arc, Zalo PC, Slack, Word, VS Code.
- [ ] Chụp xong thì app đang dùng **không** mất focus.
- [ ] Đổi folder lưu trong settings: ảnh tiếp theo vào folder mới, không cần restart.
- [ ] Rút màn hình phụ khi frame đang nằm trên đó: không crash, frame đóng.
- [ ] Đặt hotkey trùng với app khác: có báo lỗi, app vẫn chạy.
- [ ] Bật/tắt khởi động cùng Windows hoạt động.

## 11. Mở rộng sang các OS khác (Phase 3)

- **macOS** (thiết kế chi tiết: `docs/plans/2026-11-05-macos-design.md`):
  - Hỗ trợ **macOS 26 Tahoe trở lên**. Bản đầu ký bằng chứng chỉ tự tạo (dùng cá nhân, chưa notarize); đổi sang Developer ID sau.
  - Cần xin quyền Screen Recording: kiểm tra trước khi mở overlay, chưa có quyền thì không mở overlay và hướng dẫn mở System Settings.
  - Chụp bằng ScreenCaptureKit (`SCScreenshotManager`), loại cửa sổ của Chup khỏi ảnh (mục 6.4).
  - Tray thành icon trên menu bar (icon template). App không có icon ở Dock (`LSUIElement`), trừ lúc Preview đang mở
    (mục 17.2).
  - Hotkey mặc định dùng `Ctrl+Cmd`: `Ctrl+Cmd+A` (chọn vùng), `Ctrl+Cmd+X` (chụp), `Ctrl+Cmd+Z` (Freeze). Tránh `Cmd+Shift+3/4/5` của hệ thống; không dùng F1 (phím độ sáng).
  - Màn hình Retina có scale 2. `Rect` vẫn là physical px toàn desktop: gốc mỗi màn hình = gốc (point) × scale lớn nhất, kích thước = point × scale của chính màn hình đó (không bao giờ chồng nhau).
  - Overlay, frame, toolbar, stack là `NSPanel` không lấy focus (overlay thành key window để nhận Esc mà không activate app), hiện trên mọi Space và trên app toàn màn hình.
  - OCR dùng Apple Vision cho cả tiếng Việt và tiếng Anh: không có gói tải thêm, không có `chup-ocr.exe`.
  - Tự chọn vùng: `CGWindowListCopyWindowInfo` cho cửa sổ, `AXUIElement` cho phần tử (cần quyền Accessibility; chưa có thì chỉ khung cửa sổ). Khung từ ảnh chỉ ở Freeze (Live chưa chụp ngầm màn hình).
  - Mở cùng hệ thống bằng `SMAppService`. Phát hành `.app` trong `.dmg`.
- **Giao diện native riêng từng OS** trên cùng crate lõi: macOS dùng AppKit (`NSPanel` không lấy focus, `ignoresMouseEvents` cho Faded), Linux X11 dùng `x11rb` (override-redirect window, ARGB visual), Cài đặt dùng control gốc (AppKit / GTK).
- **Linux X11**: cài thêm dependency hệ thống theo README của `xcap`.
- **Linux Wayland: không hỗ trợ.** Lý do:
  - App không tự đặt được vị trí tuyệt đối của cửa sổ, nên stack ở cạnh màn hình và frame quanh region không đặt đúng chỗ được (trừ layer-shell, chỉ có trên một số compositor).
  - Chụp màn hình phải đi qua portal.
  - Hotkey toàn cục bị hạn chế.

## 12. Lộ trình

| Bước | Nội dung | Xong khi |
|---|---|---|
| **Phase 1 — v1 (Windows)** | | |
| 1.0 | Scaffold Tauri v2, tray, single-instance, config, cửa sổ settings với chọn folder | Đổi folder được, config được lưu |
| 1.1 | Hotkey, overlay đa màn hình, chọn vùng, chụp, lưu, clipboard (chưa có frame: thả chuột là chụp luôn) | Chụp đúng pixel trên 1 màn hình |
| 1.2 | Stack: 5 thumbnail, kéo thả ra ngoài, các nút hover, không cướp focus | Pass các mục test về stack ở mục 10 |
| 1.3 | Frame, toolbar, Normal/Faded mode, resize/move, chụp lại vùng cuối | Pass các mục test về Faded ở mục 10 |
| 1.4 | Hoàn thiện: mixed DPI, loại UI khỏi ảnh chụp, đổi màn hình, xử lý lỗi, autostart, installer NSIS, đo hiệu năng | Pass toàn bộ mục 10 và mục 9 |
| 1.5 | Bỏ Tauri/WebView2: giao diện Win32 + Direct2D, lõi tách thành crate riêng (mục 6) | Giữ nguyên hành vi Phase 1, RAM idle ≤ 100 MB |
| **Phase 2 — v2** | | |
| 2.1 | Freeze mode (mục 14) | Pass mục 14.7 |
| 2.2 | Preview khi bấm thumbnail (mục 17) | Pass mục 17.6 |
| 2.3 | Spike OCR (mục 15.6) | Chốt engine và các ngưỡng |
| 2.4 | OCR (mục 15) | Pass mục 15.8 |
| 2.5 | Spike tự động chọn vùng (mục 16.6) | Chốt có làm khung từ ảnh hay không |
| 2.6 | Tự động chọn vùng (mục 16) | Pass mục 16.7 |
| **Phase 3 — đa nền tảng** | | |
| 3.1.0 | Tách crate `ui` (trait `Canvas`), refactor vỏ Windows, không đổi hành vi | CI xanh, test hồi quy trên VM pass |
| 3.1.1 | macOS MVP: spike, menu bar, hotkey, quyền, overlay, chụp, stack, frame/toolbar, Cài đặt, Freeze, `.app` tự ký | Pass mục 10 và 14.7 trên macOS |
| 3.1.2 | macOS: Preview, OCR (Vision), Live Text | Pass mục 17.6 và 15.8 trên macOS |
| 3.1.3 | macOS: tự động chọn vùng | Pass mục 16.7 trên macOS |
| 3.1.4 | macOS: `.dmg`, CI macOS | Cài từ `.dmg` chạy được trên tài khoản macOS mới |
| 3.2 | Linux X11 | Pass mục 10 trên X11 |

## 13. Quyết định đã chốt

| # | Quyết định | Lý do |
|---|---|---|
| Q1 | Tên app: **Chup** | Người dùng chốt |
| Q2 | Hotkey: `Ctrl+Alt+A` (chọn vùng), `Ctrl+Alt+X` (chụp; trước v3 là `Ctrl+Alt+S`), `F1` (Freeze, Phase 2, bật/tắt được). OCR "Chụp text" chưa gán | Tránh `PrtScn` vì Windows 11 mặc định gán cho Snipping Tool. Freeze không dùng `Alt`, xem mục 14.4 |
| Q3 | Normal mode: chụp xong thì **đóng** frame | Muốn chụp lặp lại thì chuyển sang Faded |
| Q4 | Stack **không tự ẩn**, có nút thu gọn | Tự ẩn dễ làm mất ảnh đúng lúc đang định kéo |
| Q5 | Click thumbnail: mở cửa sổ Preview của Chup (mục 17). Trước 2.2: mở bằng app mặc định của OS | Người dùng chốt (2026-11-01): cần nhìn ảnh vừa chụp cạnh chỗ đang làm, chỉnh được chỗ, cỡ, opacity, và có OCR trên đó |
| Q6 | Shot mới nhất nằm sát góc dưới | Chuột di ra góc ngắn nhất cho ảnh vừa chụp |
| Q7 | Copy clipboard sau khi chụp: **bật** mặc định | Giống thói quen của Snipping Tool |
| Q8 | Freeze mode làm ở **Phase 2**, trước OCR | Người dùng chốt. Làm trước OCR vì OCR "Chụp text" dùng lại overlay của Freeze |
| Q9 | Có resize/move frame ngay Phase 1 | Cần cho use case chụp lặp lại cùng vùng |
| Q-OCR1 | OCR chỉ tiếng Việt và tiếng Anh | Ngôn ngữ khác cần model riêng, làm sau nếu cần |
| Q-OCR2 | Hotkey "Chụp text" mặc định chưa gán | Tránh xung đột, người dùng tự gán trong settings |
| Q-OCR3 | "Chụp text" không lưu ảnh, không thêm vào stack | Mục đích là lấy chữ, không làm rác folder |
| Q-OCR4 | Giữ xuống dòng như trong ảnh | Dễ đoán. Gộp đoạn sai còn khó sửa hơn |
| Q-MAC1 | macOS: chia mốc, MVP (Phase 1 + Freeze) trước | Người dùng chốt (2026-11-05). Có bản dùng sớm, kiểm chứng kiến trúc trước khi port phần nặng |
| Q-MAC2 | macOS 26 trở lên | Người dùng chốt. ScreenCaptureKit + Vision có tiếng Việt: bỏ được Tesseract trên Mac |
| Q-MAC3 | Ký bằng chứng chỉ tự tạo, chưa notarize | Người dùng chốt. Miễn phí; chữ ký ổn định nên không mất quyền Screen Recording mỗi lần build |
| Q-MAC4 | Code vẽ dùng chung qua trait `Canvas`, mỗi OS cài bằng API vẽ native; refactor Windows trước (3.1.0) | Người dùng chốt. Vẽ một lần cho mọi OS, Linux sau chỉ cần thêm một bản cài trait |

---

## 14. Phase 2 — Freeze mode

### 14.1 Mục tiêu

Chụp được những thứ biến mất khi chuột hoặc focus thay đổi:
- Menu chuột phải, dropdown.
- Tooltip, hover state.
- Video đang chạy (lấy đúng khung hình tại lúc nhấn).

### 14.2 So sánh với Live mode

| | Live (Phase 1) | Freeze |
|---|---|---|
| Thời điểm lấy ảnh | Khi bấm Chụp | Ngay khi nhấn hotkey, trước khi hiện bất cứ cửa sổ nào |
| Overlay | Phủ tối trong suốt, thấy màn hình thật | Hiện ảnh đóng băng; phần ngoài vùng chọn bị tối |
| Sau khi chọn vùng | Frame + toolbar riêng, có Faded mode | Toolbar vẽ ngay trong overlay, **không có** Faded mode |
| Chụp lặp lại cùng vùng | Có | Không |

### 14.3 Luồng

1. **Kích hoạt**: nhấn hotkey **Freeze** (`F1`), hoặc chọn từ tray: "Chụp đóng băng" / "Chụp đóng băng sau 3 giây".
2. **Chụp ngay**: Rust chụp **mọi màn hình song song** và giữ ảnh trong memory (`RgbaImage`).
   - Lúc này chưa hiện cửa sổ nào, nên app không cướp focus và menu/tooltip vẫn còn trên màn hình.
3. **Hiện overlay**: overlay mỗi màn hình hiện ảnh đóng băng tương ứng, **tỉ lệ 1:1 physical px**.
   - Phần ngoài vùng chọn phủ tối 40%; vùng chọn sáng bình thường.
4. **Chọn vùng**: kéo chuột để chọn. Thả chuột thì:
   - Vùng chọn có 8 handle để chỉnh.
   - Toolbar hiện cạnh vùng, vẽ ngay trong overlay (không phải cửa sổ riêng): **[✓ Lưu] [Copy] [✕]**. Thêm **[T]** khi đã có OCR.
5. **Hành động**:
   - `Enter` hoặc double-click trong vùng = **Lưu**: crop, gọi `persist_shot()` (lưu file, clipboard theo config, vào stack).
   - **Copy**: chỉ copy vào clipboard, không lưu file, không vào stack.
   - `Esc` hoặc click phải = huỷ.
6. **Kết thúc**: overlay đóng, giải phóng ảnh đóng băng.

### 14.4 Kỹ thuật

- **Truyền ảnh đóng băng xuống overlay**: **không** dùng data URL base64, vì ảnh 4K lên tới vài chục MB và rất chậm.
  - Overlay native giữ ảnh đóng băng trong DIB của chính nó, tô tối phần ngoài vùng chọn bằng thao tác pixel.
- **Crop luôn từ `RgbaImage` gốc**, không lấy lại từ màn hình, để đảm bảo đúng từng pixel.
- **Hiển thị 1:1**: overlay có đúng kích thước physical của màn hình, ảnh chép thẳng vào DIB, không scale.
- **Hotkey Freeze không dùng `Alt`**: trên Windows, phím Alt thường làm đóng menu đang mở trước khi app kịp chụp. Mặc định `F1` (không modifier). Cần test thực tế (mục 14.7). Phương án luôn chắc chắn là **hẹn giờ 3 giây**.
- **Trong lúc đếm ngược hẹn giờ**: không hiện gì lên màn hình (tránh lọt vào ảnh), chỉ đổi icon tray (icon mờ đi, tooltip "sắp chụp đóng băng…").
- **Dùng lại overlay window của Phase 1** ở chế độ Freeze. Ảnh không đi qua `Model`: `Effect::FreezeGrab` chụp trên worker, `Effect::ShowFreezeOverlays` chia ảnh cho overlay từng màn hình; Lưu/Copy là `Effect::FreezePersist(rect)` / `FreezeCopy(rect)`, hẹn giờ là `StartFreezeTimer` / `CancelFreezeTimer`.
- **Mục tray "Chụp đóng băng"** chờ ~350 ms cho menu tray tắt hẳn rồi mới chụp (hotkey F1 thì chụp ngay).
- **RAM**: ảnh 4K RGBA khoảng 33 MB mỗi màn hình. Giải phóng ngay khi overlay đóng.
- **Action bổ sung**:
  - Action: `FreezeConfirm { rect, action: Save | Copy | Ocr }` (physical px, mục 6.3). Bản 2.1 có `Save | Copy`; `Ocr` thêm khi làm OCR.
  - Config: thêm `hotkeys.freeze: "F1"`, tăng `version` lên 2 và migrate.

### 14.5 Tray bổ sung

Thêm vào menu tray:
- Chụp đóng băng
- Chụp đóng băng sau 3 giây
- (phần Tuỳ chọn) Phím tắt chụp đóng băng: bật/tắt hotkey Freeze. F1 giữ ở mọi app nên ai cần F1 cho app khác thì tắt.

### 14.6 Ngân sách

| Chỉ số | Mục tiêu |
|---|---|
| Từ nhấn hotkey đến lúc ảnh đóng băng hiện (2 màn hình, trong đó 1 màn 4K) | ≤ 250 ms |
| RAM sau khi đóng overlay | Về mức idle của Phase 1 |

### 14.7 Nghiệm thu

- [ ] Mở menu chuột phải trong Explorer và Chrome, nhấn `F1`: ảnh có menu.
- [ ] Tooltip đang hiện: ảnh có tooltip (thử cả hotkey và hẹn giờ 3 giây).
- [ ] Video YouTube đang chạy: ảnh đúng khung hình tại lúc nhấn.
- [ ] Màn hình 150%: crop đúng pixel, ảnh đóng băng hiển thị không bị mờ.
- [ ] Lưu: tạo file và vào stack. Copy: không tạo file.
- [ ] RAM trở về mức idle sau khi đóng overlay.

---

## 15. Phase 2 — OCR: copy text từ ảnh

Thiết kế chi tiết: `docs/plans/2026-11-01-ocr-design.md`.

### 15.1 Mục tiêu

- Lấy text (tiếng Việt có dấu và tiếng Anh) từ một shot, hoặc trực tiếp từ vùng chọn, rồi copy vào clipboard.
- **Offline hoàn toàn**: không gửi ảnh lên cloud.
- **Hai bộ cài** (chup-0oa, 2026-11-03): **Full** có sẵn OCR tiếng Việt (engine + dữ liệu, không cần mạng);
  **Lite** chỉ có `chup.exe`, OCR tiếng Việt tải **khi cài** (ô "OCR tiếng Việt", mặc định bỏ trống) hoặc sau trong app.

### 15.2 UX

- **Preview** (mục 17): toolbar có thêm nút **[T]**.
  - Bấm vào: OCR ảnh đang xem và copy text.
  - Hiện toast "Đã copy N dòng" kèm 1 dòng preview.
  - **Chọn chữ trên ảnh** (kiểu Live Text, mục 17.2): chữ nhận diện được chọn được bằng chuột theo từng ký tự rồi copy.
    Cài đặt "Tự nhận diện chữ khi mở Preview" (mặc định bật) nhận diện ở nền ngay khi mở; tắt thì chỉ sau khi bấm [T].
- **Toolbar Live mode**: thêm nút **[T]** (chỉ icon, trước ✕). Chụp region, OCR, copy text, **không lưu file và không thêm vào stack**.
- **Toolbar Freeze mode**: thêm nút **[T Chữ]** (`freeze_confirm` với `action: "ocr"`).
- **Hotkey "Chụp text"**: mặc định chưa gán.
  - Khi người dùng gán, hotkey mở overlay ở Freeze mode.
  - Chọn vùng xong là OCR luôn, không hiện toolbar.
- **Không tìm thấy chữ**: hiện toast "Không tìm thấy chữ", clipboard giữ nguyên.
- **Đang OCR**: hiện spinner trên Preview hoặc toolbar, có thể huỷ.
- **Bộ cài Lite**: trang chọn thành phần có ô **"OCR tiếng Việt"** (mô tả ghi rõ tải thêm ~11 MB), mặc định bỏ trống. Tích thì
  bộ cài chạy `chup.exe --install-ocr` (cửa sổ tải có tiến trình). Tải lỗi vẫn cài xong app. Bộ cài Full không có ô này.
- **Lần đầu dùng tiếng Việt, chưa có gói** (hoặc nút "Tải gói" trong Cài đặt):
  - Hỏi người dùng có tải gói OCR không, ghi rõ dung lượng. Tải xong thì OCR luôn ảnh vừa yêu cầu.
  - Tải có thanh tiến trình, verify checksum.
  - Mất mạng hoặc file lỗi thì xoá file dở dang, cho thử lại.

### 15.3 Chọn engine

| Engine | Ưu | Nhược |
|---|---|---|
| Windows.Media.Ocr (WinRT, có sẵn trong Windows) | Không tốn thêm dung lượng, nhanh, offline | Chỉ nhận ngôn ngữ đã cài OCR language pack. **Chưa xác minh có pack tiếng Việt** |
| Apple Vision (macOS) | Có sẵn, chất lượng tốt. Theo danh sách công bố cho macOS Tahoe 26, có tiếng Việt | Chỉ dùng được trên macOS |
| PaddleOCR PP-OCRv5 qua ONNX Runtime (`ort`) | Một engine cho mọi OS. Tiếng Việt dùng model nhận diện nhóm `latin` | Phải tải model và ONNX Runtime. Độ chính xác dấu tiếng Việt **chưa đo** |
| Tesseract (data `vie`) | Lâu đời, có sẵn data tiếng Việt | Phụ thuộc native lib, khó đóng gói trên Windows. Thường kém với chữ nhỏ trên màn hình |

**Kết quả spike (2026-11-01, chi tiết `docs/plans/2026-11-01-ocr-spike.md`):**
- Windows **không có** gói OCR `vi-VN` (chỉ en-US và các ngôn ngữ khác). Windows OCR đọc tốt tiếng Anh, 24–47 ms.
- PaddleOCR (PP-OCRv5 latin, v6): sai dấu tiếng Việt, CER 9–18%. **Loại.**
- Tesseract 5 `vie_fast` (+ `eng_fast`): CER 0,4–1,3% (chữ nhỏ phải phóng ×2), 0,2–0,5 s, 0,5 MB (+3,9 MB).
- **Chốt (người dùng chọn 2 lựa chọn):** "Tiếng Anh" dùng Windows OCR (mặc định, không tải gì);
  "Tiếng Việt + Anh" dùng Tesseract `vie_fast+eng_fast`.

**Kiểm tra Windows OCR có pack tiếng Việt không** (chạy PowerShell quyền Admin):

```powershell
Get-WindowsCapability -Online | Where-Object { $_.Name -Like 'Language.OCR*' }
```

### 15.4 Kiến trúc

```rust
pub struct OcrWord { pub text: String, pub bbox: [i32; 4], pub conf: f32, pub chars: Vec<[i32; 2]> } // [x, w] từng ký tự
pub struct OcrLine { pub text: String, pub bbox: [i32; 4], pub conf: f32, pub words: Vec<OcrWord> }

pub trait OcrEngine {
    fn recognize(&mut self, img: &image::GrayImage) -> Result<Vec<OcrLine>, String>;
}
```

- **"Tiếng Anh"**: `Windows.Media.Ocr` (WinRT qua crate `windows`) chạy trong `chup.exe`.
- **"Tiếng Việt + Anh"**: exe phụ **`chup-ocr.exe`** (crate `crates/ocr-helper`, ngoài workspace) link tĩnh Tesseract 5,
  do CI của Chup build. Chup chạy nó như process con, gửi ảnh xám qua pipe
  (khung tin ở crate `crates/ocr-proto`), nhận lại các dòng. Process gắn Job Object `KILL_ON_JOB_CLOSE`.
- **Hai gói tải về**, ghim trong `crates/core/src/ocr/manifest.json` (tên, kích thước, SHA-256 từng file; CI đọc cùng file),
  thiết kế `docs/plans/2026-11-03-ocr-full-lite-design.md`:
  - **Dữ liệu** `vie` + `eng.traineddata` (~4,4 MB) từ `tesseract-ocr/tessdata_fast` (ghim commit), vào `ocr\vi-<N>\`.
  - **Engine** `chup-ocr.exe` + `NOTICE-ocr.txt` (~6,6 MB) từ Release `ocr-engine-v<N>` của repo Chup (public), vào
    `ocr\engine-v<N>\`. Chỉ tải khi không có `chup-ocr.exe` cạnh `chup.exe` (bộ cài Lite, chạy exe rời).
  - Thư mục gốc `%LOCALAPPDATA%\app.chup.desktop\ocr\`. Cài qua thư mục tạm rồi rename; lỗi thì xoá phần dở dang.
    Tải bằng WinHTTP (theo redirect của GitHub Release). Logic cài ở `core::ocr::pack` (test được).
- **Chọn engine khi chạy**: `chup-ocr.exe` cạnh `chup.exe` (bộ cài Full) trước; không có, hoặc khác PROTO, thì bản đã tải.
- **`chup.exe --install-ocr`**: tải những gì còn thiếu, cửa sổ tải (tiến trình, Huỷ), exit code 0 xong / 1 lỗi / 2 huỷ.
  Bộ cài Lite và app dùng chung.
- **Lazy-load**: exe phụ chỉ chạy khi cần, giữ sống giữa các lần OCR, **tắt sau 2 phút không dùng** để trả RAM (trước 2026-11-03 là 5 phút; đo lúc dùng thật các lần OCR cách nhau < 1 phút).
  Huỷ job đang chạy = kill process. Crash hoặc quá 15 s = kill, báo lỗi.
- **Luồng chạy**: 1 worker thread giữ cả 2 engine. Hàng đợi chỉ giữ 1 job; job mới tới thì thay job đang chờ.
- **Cache**: nhớ kết quả cuối theo (file, ngôn ngữ). Copy lần 2 không OCR lại.
- **Job nền cho Preview** (thiết kế `docs/plans/2026-11-01-live-text-design.md`): mở Preview là nhận diện ở nền
  để chọn chữ (không quay "bận", không toast, tiếng Việt thiếu gói thì bỏ qua không hỏi). Bấm [T] lúc đó thì đợi
  job nền xong rồi copy. Job khác (Freeze, Chụp text) thay job nền; xong thì job nền chạy lại nếu Preview còn mở.
  Đóng Preview huỷ job nền.
- **Đang OCR ảnh của Preview** (job người dùng yêu cầu, chup-27e): nút [T] hiện cả khi chuột ở ngoài cửa sổ, quay vòng
  chấm, kèm nhãn "Đang nhận diện chữ…" ở chỗ nhãn zoom; sau 4 s nhãn báo thêm "Lần đầu có thể mất khoảng 1 phút".
  Preview vẫn phóng, kéo, đóng được (OCR chạy ở worker). Bấm [T] lần nữa thì huỷ.
- **macOS: làm nóng Vision.** Lần đầu sau khi cài hay cập nhật Chup, Vision biên dịch model gần 1 phút (cache theo bản
  build). 3 s sau khi khởi động, worker OCR nhận diện một ảnh chữ nhỏ ở nền để lần dùng đầu không phải chờ; job thật tới
  trước khi nó bắt đầu thì thay nó.
- Bbox (dòng, từ, ký tự) được đổi về toạ độ ảnh gốc (bỏ hệ số phóng ×2 hoặc thu nhỏ của Windows OCR).
  Tesseract trả vị trí từng ký tự; Windows OCR chỉ có bbox từ nên vị trí ký tự được ước lượng theo độ rộng chữ.

### 15.5 Pipeline

1. **Lấy ảnh**:
   - "Chụp text" và Freeze: dùng `RgbaImage` trong memory.
   - Preview: ảnh Preview đã giải mã.
2. **Tiền xử lý** (`core::ocr::prepare`): chuyển xám. **Tesseract và scale màn hình lúc chụp < 1,5 thì phóng ×2**
   (spike: chữ ~12 px kém hẳn nếu không phóng). Windows OCR không phóng. Không đảo màu (Tesseract 5 tự thử ảnh đảo).
3. **Nhận diện**: trả về các dòng kèm bbox và confidence (Windows OCR không có confidence: 1,0).
4. **Hậu xử lý** (`core::ocr::postprocess`):
   - Bỏ dòng rác: không có chữ/số, hoặc ≤ 2 ký tự và confidence < 50.
   - Sắp xếp dòng từ trên xuống dưới, trái sang phải. Hai bbox chồng nhau theo chiều dọc > 50% thì coi là cùng hàng, nối bằng dấu cách.
   - **Chuẩn hoá Unicode về NFC** (crate `unicode-normalization`). Tiếng Việt ở dạng tổ hợp (NFD) khi dán vào một số app sẽ lỗi hiển thị dấu hoặc không tìm kiếm được.
5. **Kết thúc**: ghi text vào clipboard, hiện toast "Đã copy N dòng: “dòng đầu…”".

**Giới hạn đã biết:** layout nhiều cột (báo, bảng) sẽ bị trộn thứ tự dòng. Chấp nhận ở Phase 2.

### 15.6 Spike trước khi implement (1–2 ngày)

- **Bộ test** khoảng 30 ảnh chụp thật:
  - UI web/app tiếng Việt, chat Zalo, code trong VS Code, PDF.
  - Theme sáng và tối.
  - Scale 100% và 150%.
- **Đo cho từng engine**:
  - CER (tỉ lệ lỗi ký tự), tách riêng lỗi dấu tiếng Việt.
  - Thời gian xử lý.
  - RAM khi model đã load.
  - Dung lượng gói tải về.
- **Kết quả cần chốt**: engine mặc định cho mỗi OS, ngưỡng upscale, ngưỡng confidence.

### 15.7 Ngân sách (mục tiêu, chỉnh sau spike)

| Chỉ số | Mục tiêu |
|---|---|
| Vùng nhỏ (khoảng 5 dòng) | ≤ 500 ms |
| Vùng 1920×1080 dày chữ | ≤ 2 s |
| RAM thêm khi model đang load | ≤ 150 MB, về mức Phase 1 sau khi giải phóng |
| Dung lượng installer | Đo ở CI (2897593): Lite 0,69 MB, Full 4,9 MB (engine + dữ liệu). `chup.exe` 1,4 MB |

### 15.8 Nghiệm thu

- [ ] CER ≤ 3% trên bộ test của spike (tiếng Việt có dấu).
- [ ] Dán text vào Word, Zalo, VS Code: dấu hiển thị đúng, tìm kiếm được (đã NFC).
- [ ] Ảnh không có chữ: báo "không tìm thấy chữ", clipboard không đổi.
- [ ] Mất mạng lúc tải gói OCR: báo lỗi, thử lại được, không để lại file hỏng.
- [ ] Sau 2 phút không dùng: RAM giảm về mức Phase 1.
- [ ] "Chụp text" không tạo file, không thêm vào stack.
- [ ] Bộ cài: không tích "OCR tiếng Việt" thì không tải gì; tích thì tải xong và OCR tiếng Việt dùng được ngay;
  mất mạng lúc cài vẫn cài xong app. Gỡ cài đặt xoá gói.

---

## 16. Phase 2 — Tự động chọn vùng

### 16.1 Mục tiêu

- Khi overlay đang hiện, **rê chuột là thấy ngay khung chữ nhật** của thứ nằm dưới con trỏ: cửa sổ, panel, nút, ô nhập, ảnh, khung chat...
- **Click một lần** (không kéo) là chọn đúng khung đó, không phải kéo tay cho khớp mép.
- Kéo chuột vẫn chọn vùng tự do như Phase 1. Tự động chọn chỉ là lối tắt, không thay thế.
- Dùng được trong cả **Live mode** và **Freeze mode** (mục 14).

### 16.2 Luồng

1. Nhấn hotkey Chọn vùng (hoặc Freeze). Overlay hiện như bình thường.
2. **Rê chuột**: khung nằm dưới con trỏ được **tô viền nét đứt** (màu accent), bên trong khung không phủ tối (thấy rõ ảnh sẽ chụp), kèm nhãn nhỏ `W × H · Loại` (ví dụ "Cửa sổ", "Nút", "Vùng ảnh").
   - Khung luôn bị cắt trong màn hình đang có con trỏ (giữ quy tắc 1 màn hình của Phase 1).
3. **Đổi cấp khung** (ví dụ đang tô nút, muốn lấy cả toolbar chứa nút đó):
   - Cuộn chuột lên: lên khung cha. Cuộn xuống: về khung con.
   - Phím tương đương: `↑` lên cha, `↓` xuống con.
   - Các cấp gồm khung của app (UI Automation) và **mọi lớp khung đoán từ ảnh** lồng nhau quanh con trỏ (tối đa 6 lớp
     trong phần tử sâu nhất, ví dụ ảnh → thẻ → cột), để `↑` `↓` tới được các khung thấy khi rê chuột (chup-4q9).
   - Khung đã chọn bằng cuộn / `↑` `↓` **được giữ nguyên khi con trỏ còn trong nó** (rê qua chữ, ảnh bên trong thẻ
     vẫn tô thẻ). Ra khỏi khung đó: cấp đã lên **được giữ** khi di chuột trong cùng cửa sổ (đang tô thanh công cụ thì
     rê sang nút bên cạnh vẫn tô thanh công cụ); sang cửa sổ khác thì về khung trong cùng.
4. **Chọn**:
   - Click trái mà không di chuột quá 4px: lấy khung đang tô làm region. Từ đây giống hệt bước 4 của mục 5.1 (Live mode: hiện frame và toolbar; Freeze: hiện handle và toolbar trong overlay; "Chụp text": OCR luôn).
   - Freeze khi đã có vùng chọn: rê ra **ngoài** vùng vẫn tô khung, click là chọn khung mới. Trong vùng, trên handle, trên toolbar thì không tô.
   - Nhấn giữ và kéo ≥ 4px: chọn vùng tự do như Phase 1, bỏ qua khung đang tô.
5. **Tạm tắt**: giữ `Shift` trong lúc rê thì không tô khung nào (để kéo tay chính xác ở vùng có nhiều khung lồng nhau).
6. Huỷ: `Esc` hoặc click phải, như cũ.

### 16.3 Nguồn khung (Windows)

Lấy theo thứ tự ưu tiên. Khung được tô là **khung nhỏ nhất chứa con trỏ**, ở cấp mà người dùng đang chọn.

| # | Nguồn | Cách lấy | Ghi chú |
|---|---|---|---|
| 1 | Cửa sổ top-level | `EnumWindows` theo z-order; bỏ cửa sổ ẩn, cửa sổ bị cloak (`DWMWA_CLOAKED`) và cửa sổ của Chup | Dùng `DWMWA_EXTENDED_FRAME_BOUNDS` thay cho `GetWindowRect`, để không dính viền resize vô hình và bóng đổ của Windows 10/11 |
| 2 | Phần tử UI trong cửa sổ | UI Automation: từ element của cửa sổ dưới con trỏ (`ElementFromHandle`), đi xuống phần tử sâu nhất có `BoundingRectangle` chứa con trỏ, theo Control View | Không dùng `ElementFromPoint`, vì điểm đó luôn trúng overlay của Chup |
| 3 | Khung nhận diện từ ảnh | Tìm cạnh thẳng ngang/dọc trên ảnh chụp màn hình rồi ghép thành hình chữ nhật | Cho những app không khai báo UI Automation: canvas, game, ảnh trong trang web, app Electron cũ. **Phạm vi hẹp**: chỉ dùng khi phần tử UIA sâu nhất không có con chứa con trỏ (hoặc cửa sổ không có UIA), khung nằm trong phần tử đó |

- **Cây khung** = cửa sổ → phần tử UIA lồng nhau → (khung từ ảnh nằm trong phần tử sâu nhất). "Lên cha / xuống con" đi theo cây này.
- Bỏ các khung **nhỏ hơn 8×8 px** hoặc **trùng khít khung cha** (chênh không quá 2px mỗi cạnh), để cuộn chuột không phải đi qua nhiều cấp giống hệt nhau.
- Đi xuống cây UIA: chọn con nhỏ nhất chứa con trỏ; nhánh đó không có con chứa con trỏ mà có anh em trùng khít khung thì thử anh em đó (Chromium có nhiều Pane trùng khít, có cái rỗng).

### 16.4 Kỹ thuật

- **Chụp nhanh danh sách cửa sổ trước khi hiện overlay** (Freeze: trước bước chụp ảnh và trước khi overlay lấy focus). Spike: 0,1–1 ms.
  - Live mode: danh sách cửa sổ có thể cũ đi nếu người dùng chờ lâu trong overlay. Chấp nhận, vì overlay đang phủ màn hình nên hầu như không có cửa sổ nào đổi chỗ.
- **UI Automation chạy lazy, trên thread riêng**:
  - Một thread COM (MTA) cố định, nhận yêu cầu "điểm (x, y) trong cửa sổ H" qua channel. Chỉ xử lý yêu cầu mới nhất, không chờ cố định (truy vấn đã cache gần như tức thì). Mọi yêu cầu và kết quả mang số thế hệ của phiên chọn vùng; kết quả của phiên cũ bị bỏ.
  - Dùng `CacheRequest` lấy `BoundingRectangle`, `ControlType` trong một lượt, tránh mỗi thuộc tính một lần gọi cross-process.
  - Cache cây theo cửa sổ trong suốt một lần chọn vùng (con lấy lười, giữ lại); xoá cache khi overlay đóng. Mỗi truy vấn có hạn 80 ms: hết hạn thì trả chuỗi sâu nhất đã có, lần sau đi tiếp từ cache.
  - Đặt timeout (`IUIAutomation2::ConnectionTimeout`, `TransactionTimeout`) **1 s** (spike: Word lần đầu mất 276 ms). Timeout, app treo hoặc chạy quyền admin (Chup không đọc được UIA của nó) thì cửa sổ đó chỉ dùng khung cửa sổ trong phiên.
  - Khi overlay vừa mở, hỏi ngay tại vị trí con trỏ: Chromium / Electron chỉ bật cây accessibility sau khi có client hỏi.
- **Toạ độ**: Chup là app Per-Monitor DPI Aware v2, nên DWM và UIA trả về **physical px**. Dùng thẳng làm `Rect` (mục 6.3), không quy đổi.
- **Freeze mode**: lấy danh sách cửa sổ ngay lúc chụp đóng băng, nên menu, tooltip vẫn có trong danh sách và chọn được khung của cả menu. UIA vẫn lười: menu đóng khi overlay lấy focus, nên không chọn được từng mục menu.
- **Khung từ ảnh**: Freeze dùng ảnh đóng băng; Live chụp ngầm màn hình sau khi overlay hiện, nên overlay được loại khỏi ảnh chụp (`WDA_EXCLUDEFROMCAPTURE`, như frame / toolbar / stack; hệ quả: app chia sẻ màn hình không thấy overlay). Máy không hỗ trợ thì Live không có khung từ ảnh. Bản đồ cạnh dựng lười theo màn hình (4K: ~63 ms, 33 MB), giải phóng khi overlay đóng.
- **Không để UIA làm chậm việc vẽ**: overlay tô ngay khung cửa sổ (đã có sẵn), rồi thay bằng khung UIA khi kết quả về.
- **Action bổ sung**:
  - Overlay gửi điểm dưới con trỏ qua App cho thread UIA (`Action::AutoHover`), nhận lại chuỗi khung `{ rect, kind }` từ ngoài vào trong (physical px, `Action::AutoFrames`).
  - Cuộn chuột hoặc phím mũi tên đổi `depth` (lên khung cha / xuống khung con).
  - Click chọn khung dùng lại `Action::RegionSelected(rect)` của Phase 1.
- **Config**: thêm `autoDetect: { enabled: true, imageEdges: true }` (serde default, không tăng `version`). Cài đặt có 2 ô tương ứng.
- **Phase 3**:
  - macOS: `CGWindowListCopyWindowInfo` cho cửa sổ, Accessibility API (`AXUIElement`) cho phần tử. Cần quyền Accessibility.
  - Linux X11: `XQueryTree` cho cửa sổ; phần tử qua AT-SPI nếu có, không thì chỉ cửa sổ.

### 16.5 Ngân sách

| Chỉ số | Mục tiêu |
|---|---|
| Thêm vào thời gian từ hotkey đến lúc overlay hiện | ≤ 10 ms |
| Từ lúc di chuột đến lúc khung cửa sổ được tô | ≤ 16 ms (1 frame) |
| Từ lúc di chuột đến lúc khung UIA được tô (lần đầu trong cửa sổ đó) | ≤ 100 ms |
| CPU khi idle (overlay đã đóng) | ~0%, thread UIA ngủ |

### 16.6 Spike (khoảng 1 ngày)

> Đã làm 2026-11-03: [docs/plans/2026-11-03-autoselect-spike.md](docs/plans/2026-11-03-autoselect-spike.md). Chốt: làm nguồn 3, bật mặc định, phạm vi hẹp. Thiết kế: [docs/plans/2026-11-03-autoselect-design.md](docs/plans/2026-11-03-autoselect-design.md).

- Đo UIA trên các app hay dùng: Chrome, Edge, Explorer, VS Code, Zalo PC, Slack, Word, Excel, Teams. Ghi lại:
  - Thời gian lần đầu lấy cây và các lần sau (đã cache).
  - App nào trả về cây rỗng hoặc chỉ có một khung lớn (thường gặp ở app Electron, game).
- Thử nhận diện khung từ ảnh trên khoảng 20 ảnh chụp các app ở trên: tỉ lệ khung đúng, khung thừa (nhiễu), thời gian xử lý màn hình 4K.
- **Kết quả cần chốt**: có làm nguồn 3 (khung từ ảnh) ở Phase 2 không; nếu có thì bật mặc định hay để tuỳ chọn.

### 16.7 Nghiệm thu

- [ ] Rê chuột qua cửa sổ Explorer, Chrome: khung khớp mép cửa sổ, không dính bóng đổ hay viền vô hình.
- [ ] Trong Chrome: tô được thanh địa chỉ, một ảnh trong trang, một khung bài viết. Cuộn chuột lên cha và xuống con được.
- [ ] Click chọn khung rồi chụp: ảnh đúng kích thước khung, đúng pixel ở scale 100% và 150%.
- [ ] Giữ `Shift`: không tô khung; kéo tay vẫn chọn vùng như cũ.
- [ ] Freeze mode: mở menu chuột phải, nhấn `F1`, click chọn được đúng khung của menu.
- [ ] App đang treo hoặc chạy quyền admin: overlay không bị đứng, vẫn tô được khung cửa sổ.
- [ ] Cửa sổ vắt qua 2 màn hình: khung bị cắt trong màn hình đang có con trỏ.

## 17. Phase 2 — Preview khi bấm thumbnail

Thiết kế chi tiết: `docs/plans/2026-11-01-preview-design.md`.

### 17.1 Mục tiêu

- Bấm thumbnail là thấy ảnh vừa chụp đủ to để đọc, đặt cạnh chỗ đang làm để dựa vào đó làm việc khác
  (gõ lại số liệu, so giao diện, làm theo hướng dẫn).
- Chỗ và cỡ mở có mặc định hợp lý và chỉnh được; opacity chỉnh được.
- Là chỗ đặt OCR (mục 15.2): copy text từ ảnh đang xem.

### 17.2 Hành vi

- **Một cửa sổ Preview**: bấm thumbnail khác thì thay ảnh.
- **Chỗ và cỡ mở** (Cài đặt "Chỗ mở"):
  - **Chỗ lần trước** (mặc định): kéo di chuyển là Chup nhớ chỗ. Lần sau ảnh bám cạnh khung cũ gần cạnh màn hình
    hơn (khung ở góc dưới-phải thì ảnh mới cũng nằm sát góc đó). Khung cũ không quyết định cỡ. Chưa có khung, hoặc
    màn hình chứa khung đã bị rút: mở như "Cạnh stack".
  - **Cạnh stack**: sát mép trong của stack, căn đáy work area, cách 12 DIP.
  - **Giữa màn hình**.
  - **Cỡ khi mở tự tính theo từng ảnh**, không phụ thuộc ảnh trước: lấy mức nhỏ nhất của (1) 1:1, (2) diện tích tối
    đa f² work area, f là mức trong Cài đặt (mặc định 50%, tức 25% diện tích), (3) mỗi cạnh tối đa 90% work area.
    Ảnh cùng tỉ lệ màn hình cỡ như f mỗi chiều; ảnh dải ngang, trang dọc được dài hơn để đọc được.
    Ảnh nhỏ hơn 160×48 DIP thì cửa sổ giữ cỡ đó, ảnh nằm giữa trên nền tối.
  - **Kéo mép** (khi "Chỗ lần trước"): nhớ tỉ lệ so với cỡ tự tính (50–200%), áp vào giới hạn (2) của ảnh sau. Chỉ học
    từ ảnh công thức đã thu nhỏ: kéo mép ảnh đang 1:1 (ảnh nhỏ, ảnh dải) chỉ chỉnh riêng ảnh đó. Đổi mức trong Cài đặt
    thì bỏ tỉ lệ đã học.
  - Mở trên màn hình đang có stack.
- **Chỉnh trực tiếp**:
  - Kéo bất kỳ đâu (ngoài nút) để di chuyển.
  - Kéo mép hoặc góc (dải 6 DIP trong khung, cộng cả lề bóng đổ ngoài khung; góc 12 DIP) để đổi cỡ, luôn giữ
    tỉ lệ ảnh, mép đối diện đứng yên. Đổi cỡ khi đang phóng to thì vẫn thấy đúng phần ảnh đó.
  - Lăn chuột: phóng to/thu nhỏ ảnh **trong khung** quanh con trỏ (×1,25 mỗi nấc), cửa sổ giữ nguyên cỡ và chỗ.
    Nhỏ nhất là vừa khung (thấy cả ảnh), lớn nhất 800% ảnh gốc.
  - macOS, như app Preview của máy: chụm 2 ngón trên trackpad là thu nhỏ, xoè ra là phóng to quanh con trỏ, đổi mượt
    không theo nấc. Vuốt 2 ngón (lên, xuống, trái, phải) dời ảnh đang phóng to theo chiều cuộn của máy (bật Natural
    Scrolling, mặc định, thì ảnh đi theo ngón tay); vừa khung thì vuốt không làm gì. Chuột có nấc vẫn phóng to như
    Windows, tính theo chiều tay thật (đẩy lên là phóng to). Mở Preview hoặc bấm vào Preview thì Chup thành app đang
    dùng, và có icon ở Dock tới khi đóng Preview: macOS chỉ gửi cử chỉ trackpad cho app thường (không phải app chỉ có
    icon menu bar) đang dùng.
  - Đang phóng to: kéo (ngoài nút, mép, chữ) để kéo ảnh, con trỏ hình mũi tên 4 chiều; ảnh không kéo ra ngoài khung.
    `Alt` + kéo vẫn di chuyển cửa sổ.
  - `+`/`-` phóng to/thu nhỏ quanh tâm khung, `0` về vừa khung.
  - Cửa sổ luôn nằm trong work area khi đổi cỡ.
  - Bóng đổ nhẹ quanh khung (màu bóng của theme, loang 8 DIP, lệch xuống 2 DIP).
- **Opacity**: mặc định theo Cài đặt (100%, chọn 100–20%). `Ctrl` + lăn chuột chỉnh ±10% cho cửa sổ đang mở
  (không lưu). Không thấp hơn 20%.
- **Luôn nằm trên** cửa sổ khác: Cài đặt (mặc định bật). Nút ghim trên toolbar bật/tắt cho cửa sổ đang mở.
- **Toolbar** góc trên-phải, chỉ hiện khi hover: [T] [Ghim] [Copy] [Folder] [✕] ([T] = OCR, mục 15.2).
- **Chọn chữ** (khi đã nhận diện, mục 15.2):
  - Con trỏ I-beam trên chữ. Kéo bắt đầu trên chữ là chọn theo từng ký tự (qua nhiều dòng được); kéo bắt đầu ở
    chỗ khác là di chuyển cửa sổ (đang phóng to thì kéo ảnh). `Alt` + kéo luôn di chuyển cửa sổ. Bấm đúp chọn một từ, bấm 3 lần chọn cả dòng.
    `Shift` + bấm nới vùng chọn. Bấm (không kéo) bỏ vùng chọn.
  - Vùng chọn tô màu nhấn trong suốt, theo đúng zoom và cỡ cửa sổ.
  - `Ctrl+A` chọn tất cả chữ. `Ctrl+C` copy chữ đã chọn (chưa chọn thì copy ảnh). Toast như [T].
  - Chuột phải: menu "Copy chữ", "Chọn tất cả chữ", "Copy ảnh".
  - `Esc` bỏ vùng chọn trước, lần sau mới đóng.
- Đổi zoom hoặc opacity: nhãn "75%" (tỉ lệ so với ảnh gốc) / "Opacity 60%" ở đáy trong 1,2 giây.
- `Esc` hoặc ✕: đóng. `Ctrl+C`: copy ảnh (khi chưa chọn chữ).
- Preview lấy focus khi mở; có trên taskbar và Alt+Tab (tiêu đề là tên file).
- Preview **không** bị loại khỏi ảnh chụp (khác stack, toolbar): người dùng có thể muốn chụp lại hoặc
  chia sẻ màn hình có nó.
- File đã bị xoá bên ngoài: như kéo thả (mục 7).

### 17.3 Kỹ thuật

- Layered window như các cửa sổ phụ khác; opacity là alpha chung của `UpdateLayeredWindow`.
- Giải mã PNG ở worker; bấm liên tiếp thì chỉ hiện ảnh của lần bấm cuối.
- Ảnh, DIB và bitmap Direct2D chỉ tồn tại khi đang mở (ảnh 4K ~100 MB), đóng là giải phóng.
- Hình học (chỗ mở, zoom, kéo mép) ở `core::preview`, unit test được.

### 17.4 Config

```json
"preview": { "alwaysOnTop": true, "opacity": 100, "position": "remember", "size": 50, "last": null, "sizeScale": 100 }
```

- `position`: `remember` | `stack` | `center`. `last`: khung lần trước `{x, y, w, h}` (physical px toàn desktop),
  chỉ ghi khi `position = remember`, chỉ dùng để chọn chỗ mở. `sizeScale`: tỉ lệ % đã học từ kéo mép.
- `opacity` kẹp 20–100, `size` kẹp 20–100, `sizeScale` kẹp 50–200.
- Tự nhận diện chữ nằm ở `"ocr": { "language": "en", "autoPreview": true }` (mục 15).

### 17.5 Ngân sách

- Từ click thumbnail tới lúc Preview hiện (ảnh 1920×1080): ≤ 200 ms.
- Hover toolbar, kéo mép: không giật ở màn 4K.

### 17.6 Nghiệm thu

- [ ] Click thumbnail: Preview mở sát stack, ảnh nét (1:1 khi vừa), lấy focus.
- [ ] Kéo di chuyển, kéo 4 mép và 4 góc đổi cỡ giữ tỉ lệ.
- [ ] Lăn chuột phóng to quanh con trỏ, cửa sổ không đổi cỡ; đang phóng to thì kéo ảnh được, `Alt` + kéo di chuyển
      cửa sổ; `0` về vừa khung. Chọn chữ vẫn đúng chỗ khi phóng to.
- [ ] macOS: chụm 2 ngón thu nhỏ, xoè ra phóng to; vuốt 2 ngón 4 hướng dời ảnh theo ngón tay. Bấm sang app khác rồi
      quay lại bấm vào Preview: chụm vẫn được.
- [ ] Đóng rồi mở ảnh khác: nằm đúng chỗ lần trước, cỡ tự tính theo ảnh mới. Đổi "Chỗ mở" sang Cạnh stack / Giữa màn hình: đúng chỗ.
- [ ] Kéo di chuyển Preview của ảnh dải rồi mở ảnh lớn: ảnh lớn không bị nhỏ theo. Kéo mép ảnh lớn rồi mở ảnh lớn khác: to, nhỏ theo tỉ lệ đó.
- [ ] Ctrl + lăn chuột đổi opacity; Cài đặt đổi opacity và "luôn nằm trên" thì áp ngay cho Preview đang mở.
- [ ] Ghim: tắt thì click app khác che được Preview; bật thì luôn nằm trên.
- [ ] Copy, Folder, ✕, `Esc`, `Ctrl+C`, Alt+F4 (ẩn, mở lại được).
- [ ] Bấm thumbnail khác khi đang mở: thay ảnh. File bị xoá: toast, bỏ khỏi stack.
- [ ] RAM về mức trước khi mở sau khi đóng.
- [ ] Chọn chữ: kéo trên chữ tiếng Anh và tiếng Việt (một phần từ, nhiều dòng, sau khi zoom), bấm đúp, bấm 3 lần,
  `Shift` + bấm, `Ctrl+A`, `Ctrl+C`,
  menu chuột phải, `Alt` + kéo, `Esc`. Tắt "Tự nhận diện" thì chỉ chọn được sau khi bấm [T].
