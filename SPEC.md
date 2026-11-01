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
| Giao diện macOS, Linux (Phase 3) | AppKit (`objc2`), X11 (`x11rb`); dùng chung crate lõi |
| Chụp màn hình | `xcap` |
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
5. Bấm **[Chụp]** trên toolbar hoặc nhấn hotkey **Chụp** (`Ctrl+Alt+S`):
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
  - **Click**: mở ảnh bằng app mặc định của OS.
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
- **Click phải** mở menu:
  - Chọn vùng
  - Chụp lại vùng cuối
  - Chụp đóng băng, Chụp đóng băng sau 3 giây (Phase 2, mục 14.5)
  - Mở folder ảnh
  - Cài đặt…
  - Thoát
- **Tooltip**: hiển thị "Chup". Nếu có hotkey đăng ký lỗi thì kèm cảnh báo.

### 5.6 Cài đặt

Cửa sổ thường, có trên taskbar. Chỉ tạo khi mở, huỷ khi đóng. Thay đổi được lưu ngay khi sửa, không có nút Save.

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
- **macOS** (Phase 3): thử `NSWindow.sharingType = .none`. **Chưa chắc** cách này còn hiệu lực với API chụp mới; cần kiểm chứng, không được thì dùng fallback.

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
  "hotkeys": { "selectRegion": "Ctrl+Alt+A", "capture": "Ctrl+Alt+S", "freeze": "F1" },
  "stack": { "edge": "right", "monitor": "primary" },
  "afterCaptureNormal": "close",
  "launchAtStartup": false
}
```

- Version 2 (Phase 2.1) thêm `hotkeys.freeze`. Migrate từ v1: điền `"F1"`; nếu F1 đã được dùng cho hotkey khác thì để trống (tắt).
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

- **macOS**:
  - Cần xin quyền Screen Recording: kiểm tra trước khi chụp, hướng dẫn mở System Settings nếu chưa có quyền.
  - Tray thành icon trên menu bar. Hotkey mặc định dùng `Cmd` thay cho `Ctrl+Alt`.
  - Màn hình Retina có scale 2.
  - Kiểm chứng lại cách loại cửa sổ khỏi ảnh chụp (mục 6.4).
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
| 2.2 | Preview to khi bấm thumbnail: tuỳ chọn luôn nằm trên, chỉnh opacity. OCR sẽ đặt trên preview này (thiết kế sau, đổi Q5 và mục 15.2) | Chưa chốt |
| 2.3 | Spike OCR (mục 15.6) | Chốt engine và các ngưỡng |
| 2.4 | OCR (mục 15) | Pass mục 15.8 |
| 2.5 | Spike tự động chọn vùng (mục 16.6) | Chốt có làm khung từ ảnh hay không |
| 2.6 | Tự động chọn vùng (mục 16) | Pass mục 16.7 |
| **Phase 3 — đa nền tảng** | | |
| 3.1 | macOS | Pass mục 10 và 14.7 trên macOS |
| 3.2 | Linux X11 | Pass mục 10 trên X11 |

## 13. Quyết định đã chốt

| # | Quyết định | Lý do |
|---|---|---|
| Q1 | Tên app: **Chup** | Người dùng chốt |
| Q2 | Hotkey: `Ctrl+Alt+A` (chọn vùng), `Ctrl+Alt+S` (chụp), `F1` (Freeze, Phase 2). OCR "Chụp text" chưa gán | Tránh `PrtScn` vì Windows 11 mặc định gán cho Snipping Tool. Freeze không dùng `Alt`, xem mục 14.4 |
| Q3 | Normal mode: chụp xong thì **đóng** frame | Muốn chụp lặp lại thì chuyển sang Faded |
| Q4 | Stack **không tự ẩn**, có nút thu gọn | Tự ẩn dễ làm mất ảnh đúng lúc đang định kéo |
| Q5 | Click thumbnail: mở bằng app mặc định của OS | Không phải tự làm trình xem ảnh |
| Q6 | Shot mới nhất nằm sát góc dưới | Chuột di ra góc ngắn nhất cho ảnh vừa chụp |
| Q7 | Copy clipboard sau khi chụp: **bật** mặc định | Giống thói quen của Snipping Tool |
| Q8 | Freeze mode làm ở **Phase 2**, trước OCR | Người dùng chốt. Làm trước OCR vì OCR "Chụp text" dùng lại overlay của Freeze |
| Q9 | Có resize/move frame ngay Phase 1 | Cần cho use case chụp lặp lại cùng vùng |
| Q-OCR1 | OCR chỉ tiếng Việt và tiếng Anh | Ngôn ngữ khác cần model riêng, làm sau nếu cần |
| Q-OCR2 | Hotkey "Chụp text" mặc định chưa gán | Tránh xung đột, người dùng tự gán trong settings |
| Q-OCR3 | "Chụp text" không lưu ảnh, không thêm vào stack | Mục đích là lấy chữ, không làm rác folder |
| Q-OCR4 | Giữ xuống dòng như trong ảnh | Dễ đoán. Gộp đoạn sai còn khó sửa hơn |

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
- **Dùng lại overlay window của Phase 1**: `Effect::ShowOverlays` mang thêm chế độ freeze và ảnh của từng màn hình.
- **RAM**: ảnh 4K RGBA khoảng 33 MB mỗi màn hình. Giải phóng ngay khi overlay đóng.
- **Action bổ sung**:
  - Action: `FreezeConfirm { rect, action: Save | Copy | Ocr }` (physical px, mục 6.3). Bản 2.1 có `Save | Copy`; `Ocr` thêm khi làm OCR.
  - Config: thêm `hotkeys.freeze: "F1"`, tăng `version` lên 2 và migrate.

### 14.5 Tray bổ sung

Thêm 2 mục vào menu tray:
- Chụp đóng băng
- Chụp đóng băng sau 3 giây

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

### 15.1 Mục tiêu

- Lấy text (tiếng Việt có dấu và tiếng Anh) từ một shot, hoặc trực tiếp từ vùng chọn, rồi copy vào clipboard.
- **Offline hoàn toàn**: không gửi ảnh lên cloud.
- **Installer không phình**: model OCR chỉ tải về khi dùng lần đầu.

### 15.2 UX

- **Stack**: khi hover thumbnail có thêm nút **[T]**.
  - Bấm vào: OCR shot đó và copy text.
  - Hiện toast "Đã copy N dòng" kèm 1 dòng preview.
- **Toolbar Live mode**: thêm nút **[T Chụp text]**. Chụp region, OCR, copy text, **không lưu file và không thêm vào stack**.
- **Toolbar Freeze mode**: thêm nút **[T]** (`freeze_confirm` với `action: "ocr"`).
- **Hotkey "Chụp text"**: mặc định chưa gán.
  - Khi người dùng gán, hotkey mở overlay ở Freeze mode.
  - Chọn vùng xong là OCR luôn, không hiện toolbar.
- **Không tìm thấy chữ**: hiện toast "Không tìm thấy chữ", clipboard giữ nguyên.
- **Đang OCR**: hiện spinner trên thumbnail hoặc toolbar, có thể huỷ.
- **Lần đầu dùng, chưa có model**:
  - Hỏi người dùng có tải gói OCR không, ghi rõ dung lượng.
  - Tải có thanh tiến trình, verify checksum.
  - Mất mạng hoặc file lỗi thì xoá file dở dang, cho thử lại.

### 15.3 Chọn engine

| Engine | Ưu | Nhược |
|---|---|---|
| Windows.Media.Ocr (WinRT, có sẵn trong Windows) | Không tốn thêm dung lượng, nhanh, offline | Chỉ nhận ngôn ngữ đã cài OCR language pack. **Chưa xác minh có pack tiếng Việt** |
| Apple Vision (macOS) | Có sẵn, chất lượng tốt. Theo danh sách công bố cho macOS Tahoe 26, có tiếng Việt | Chỉ dùng được trên macOS |
| PaddleOCR PP-OCRv5 qua ONNX Runtime (`ort`) | Một engine cho mọi OS. Tiếng Việt dùng model nhận diện nhóm `latin` | Phải tải model và ONNX Runtime. Độ chính xác dấu tiếng Việt **chưa đo** |
| Tesseract (data `vie`) | Lâu đời, có sẵn data tiếng Việt | Phụ thuộc native lib, khó đóng gói trên Windows. Thường kém với chữ nhỏ trên màn hình |

**Đề xuất:**
- Dùng `trait OcrEngine`. Engine mặc định cho mỗi OS sẽ chốt sau spike (mục 15.6).
- Ứng viên số 1 cho Windows là **PaddleOCR qua ONNX**, vì đây là đường duy nhất vừa chạy mọi OS vừa có khả năng đọc tiếng Việt trên Windows.
- Trên OS nào engine có sẵn hỗ trợ tiếng Việt (macOS), ưu tiên engine có sẵn vì không tốn dung lượng tải.

**Kiểm tra Windows OCR có pack tiếng Việt không** (chạy PowerShell quyền Admin):

```powershell
Get-WindowsCapability -Online | Where-Object { $_.Name -Like 'Language.OCR*' }
```

Nếu kết quả không có dòng `vi-VN` thì loại Windows OCR khỏi đường xử lý tiếng Việt.

### 15.4 Kiến trúc

```rust
pub struct OcrLine { pub text: String, pub bbox: [i32; 4], pub confidence: f32 }
pub struct OcrResult { pub lines: Vec<OcrLine> }

pub trait OcrEngine: Send + Sync {
    fn id(&self) -> &'static str;
    fn languages(&self) -> Vec<String>;
    fn recognize(&self, img: &image::RgbaImage, langs: &[&str]) -> Result<OcrResult, OcrError>;
}
```

```
src-tauri/src/ocr/
├─ mod.rs            # trait, pipeline, chọn engine theo OS + config
├─ paddle.rs         # ONNX (ort)
├─ win.rs            # Windows.Media.Ocr qua crate `windows`
├─ mac.rs            # Vision qua `objc2-vision`
└─ model_manager.rs  # tải, verify checksum, version, xoá model
```

- **Vị trí model và ONNX Runtime**: `%LOCALAPPDATA%\<identifier>\ocr\`, tải theo nhu cầu.
- **DLL của ONNX Runtime** load động lúc chạy (tính năng load-dynamic của `ort`; kiểm tra tên feature theo version đang dùng), nên không nằm trong installer.
- **Lazy-load**: chỉ load model khi dùng lần đầu. **Giải phóng sau 5 phút không dùng** để trả RAM.
- **Luồng chạy**: OCR chạy trên thread riêng. Hàng đợi chỉ giữ 1 job; job mới tới thì huỷ job đang chờ.
- **Cache**: lưu kết quả theo shot (`Shot.ocr_text: Option<String>`). Copy lần 2 không OCR lại.

### 15.5 Pipeline

1. **Lấy ảnh**:
   - "Chụp text" và Freeze: dùng `RgbaImage` trong memory.
   - Shot trong stack: đọc lại file.
2. **Tiền xử lý**:
   - Upscale 2× (Lanczos) khi chữ nhỏ. Chữ chụp ở scale 100% thường nhỏ hơn nhiều so với kích thước dòng chữ mà model kỳ vọng. Ngưỡng cụ thể chốt sau spike.
   - Với theme tối (chữ sáng trên nền tối): thử đảo màu nếu confidence thấp. Spike sẽ quyết định có cần không.
3. **Nhận diện**: trả về các dòng kèm bbox và confidence.
4. **Hậu xử lý**:
   - Sắp xếp dòng từ trên xuống dưới, trái sang phải. Hai bbox chồng nhau theo chiều dọc > 50% thì coi là cùng hàng, nối bằng dấu cách.
   - **Chuẩn hoá Unicode về NFC** (crate `unicode-normalization`). Tiếng Việt ở dạng tổ hợp (NFD) khi dán vào một số app sẽ lỗi hiển thị dấu hoặc không tìm kiếm được.
   - Bỏ dòng có confidence dưới ngưỡng (spike chốt).
5. **Kết thúc**: ghi text vào clipboard, gửi event để hiện toast.

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
| Dung lượng installer | Không tăng so với Phase 1 |

### 15.8 Nghiệm thu

- [ ] CER ≤ 3% trên bộ test của spike (tiếng Việt có dấu).
- [ ] Dán text vào Word, Zalo, VS Code: dấu hiển thị đúng, tìm kiếm được (đã NFC).
- [ ] Ảnh không có chữ: báo "không tìm thấy chữ", clipboard không đổi.
- [ ] Mất mạng lúc tải gói OCR: báo lỗi, thử lại được, không để lại file hỏng.
- [ ] Sau 5 phút không dùng: RAM giảm về mức Phase 1.
- [ ] "Chụp text" không tạo file, không thêm vào stack.

---

## 16. Phase 2 — Tự động chọn vùng

### 16.1 Mục tiêu

- Khi overlay đang hiện, **rê chuột là thấy ngay khung chữ nhật** của thứ nằm dưới con trỏ: cửa sổ, panel, nút, ô nhập, ảnh, khung chat...
- **Click một lần** (không kéo) là chọn đúng khung đó, không phải kéo tay cho khớp mép.
- Kéo chuột vẫn chọn vùng tự do như Phase 1. Tự động chọn chỉ là lối tắt, không thay thế.
- Dùng được trong cả **Live mode** và **Freeze mode** (mục 14).

### 16.2 Luồng

1. Nhấn hotkey Chọn vùng (hoặc Freeze). Overlay hiện như bình thường.
2. **Rê chuột**: khung nằm dưới con trỏ được **tô viền nét đứt** (màu accent), kèm nhãn nhỏ `W × H` và loại khung (ví dụ "Cửa sổ", "Nút", "Vùng ảnh").
   - Khung luôn bị cắt trong màn hình đang có con trỏ (giữ quy tắc 1 màn hình của Phase 1).
3. **Đổi cấp khung** (ví dụ đang tô nút, muốn lấy cả toolbar chứa nút đó):
   - Cuộn chuột lên: lên khung cha. Cuộn xuống: về khung con.
   - Phím tương đương: `↑` lên cha, `↓` xuống con.
4. **Chọn**:
   - Click trái mà không di chuột quá 4px: lấy khung đang tô làm region. Từ đây giống hệt bước 4 của mục 5.1 (Live mode: hiện frame và toolbar; Freeze: hiện handle và toolbar trong overlay).
   - Nhấn giữ và kéo ≥ 4px: chọn vùng tự do như Phase 1, bỏ qua khung đang tô.
5. **Tạm tắt**: giữ `Shift` trong lúc rê thì không tô khung nào (để kéo tay chính xác ở vùng có nhiều khung lồng nhau).
6. Huỷ: `Esc` hoặc click phải, như cũ.

### 16.3 Nguồn khung (Windows)

Lấy theo thứ tự ưu tiên. Khung được tô là **khung nhỏ nhất chứa con trỏ**, ở cấp mà người dùng đang chọn.

| # | Nguồn | Cách lấy | Ghi chú |
|---|---|---|---|
| 1 | Cửa sổ top-level | `EnumWindows` theo z-order; bỏ cửa sổ ẩn, cửa sổ bị cloak (`DWMWA_CLOAKED`) và cửa sổ của Chup | Dùng `DWMWA_EXTENDED_FRAME_BOUNDS` thay cho `GetWindowRect`, để không dính viền resize vô hình và bóng đổ của Windows 10/11 |
| 2 | Phần tử UI trong cửa sổ | UI Automation: từ element của cửa sổ dưới con trỏ (`ElementFromHandle`), đi xuống phần tử sâu nhất có `BoundingRectangle` chứa con trỏ, theo Control View | Không dùng `ElementFromPoint`, vì điểm đó luôn trúng overlay của Chup |
| 3 | Khung nhận diện từ ảnh | Tìm cạnh thẳng ngang/dọc trên ảnh chụp màn hình rồi ghép thành hình chữ nhật | Cho những app không khai báo UI Automation: canvas, game, ảnh trong trang web, app Electron cũ. **Cần spike** (mục 16.6) |

- **Cây khung** = cửa sổ → phần tử UIA lồng nhau → (khung từ ảnh nằm trong phần tử sâu nhất). "Lên cha / xuống con" đi theo cây này.
- Bỏ các khung **nhỏ hơn 8×8 px** hoặc **trùng khít khung cha** (chênh dưới 2px mỗi cạnh), để cuộn chuột không phải đi qua nhiều cấp giống hệt nhau.

### 16.4 Kỹ thuật

- **Chụp nhanh danh sách cửa sổ trước khi hiện overlay**: cùng lúc với bước chụp ảnh của Freeze. Mất vài ms, không làm chậm hotkey.
  - Live mode: danh sách cửa sổ có thể cũ đi nếu người dùng chờ lâu trong overlay. Chấp nhận, vì overlay đang phủ màn hình nên hầu như không có cửa sổ nào đổi chỗ.
- **UI Automation chạy lazy, trên thread riêng**:
  - Một thread COM (MTA) cố định, nhận yêu cầu "điểm (x, y) trong cửa sổ H" qua channel. Chỉ xử lý yêu cầu mới nhất (debounce khoảng 30 ms khi chuột đang di).
  - Dùng `CacheRequest` lấy `BoundingRectangle`, `ControlType`, `Name` trong một lượt, tránh mỗi thuộc tính một lần gọi cross-process.
  - Cache kết quả theo cửa sổ trong suốt một lần chọn vùng; xoá cache khi overlay đóng.
  - Đặt timeout (`IUIAutomation2::ConnectionTimeout`, `TransactionTimeout` khoảng 200 ms). App bị treo hoặc chạy quyền admin (Chup không đọc được UIA của nó) thì chỉ dùng khung cửa sổ.
- **Toạ độ**: Chup là app Per-Monitor DPI Aware v2, nên DWM và UIA trả về **physical px**. Dùng thẳng làm `Rect` (mục 6.3), không quy đổi.
- **Freeze mode**: lấy danh sách cửa sổ và UIA ngay lúc chụp đóng băng, nên menu, tooltip vẫn còn trong cây UI và chọn được đúng khung của chúng.
- **Không để UIA làm chậm việc vẽ**: overlay tô ngay khung cửa sổ (đã có sẵn), rồi thay bằng khung UIA khi kết quả về.
- **Action bổ sung**:
  - Overlay gửi điểm dưới con trỏ cho thread UIA, nhận lại `{ rect, kind, depth }` (physical px).
  - Cuộn chuột hoặc phím mũi tên đổi `depth` (lên khung cha / xuống khung con).
  - Click chọn khung dùng lại `Action::RegionSelected(rect)` của Phase 1.
- **Config**: thêm `autoDetect: { enabled: true, imageEdges: false }`, gộp vào lần tăng `version` lên 2 của Freeze (mục 14.4).
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
