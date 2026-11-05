# Thiết kế Phase 3.1: bản macOS

Ngày: 2026-11-05. Beads: epic `chup-mac`; Mốc 0 → 4 là `chup-mac.1` → `chup-mac.5`. Đặc tả gốc: [SPEC mục 11](../../SPEC.md#11-mở-rộng-sang-các-os-khác-phase-3).
Tài liệu này chốt cách làm và lộ trình; chỗ lệch SPEC ghi ở mục 9 và đã cập nhật vào SPEC cùng nhánh.

## Người dùng chốt (2026-11-05)

- **Chia mốc, MVP trước.** Mốc đầu của Mac = luồng Phase 1 + Freeze (đúng SPEC 3.1). Preview, OCR / Live Text, tự chọn
  vùng làm ở các mốc sau.
- **Tự ký, dùng cá nhân trước.** Chứng chỉ code signing tự tạo trên máy dev, chữ ký ổn định nên quyền Screen Recording
  không bị hỏi lại sau mỗi lần build. Chưa notarize. Thiết kế sẵn để đổi sang Developer ID sau (chỉ đổi tên identity).
- **macOS 26 Tahoe trở lên.** Được dùng ScreenCaptureKit (`SCScreenshotManager`) và Apple Vision có tiếng Việt, nên
  **không cần Tesseract / `chup-ocr.exe` trên Mac**.
- **Native, dùng chung code vẽ qua trait `Canvas`** (Windows = Direct2D / DirectWrite như hiện tại, macOS =
  CoreGraphics / CoreText). Làm **Mốc 0 refactor bản Windows trước** (PR riêng, test VM) rồi mới viết vỏ macOS.

## Hướng đã chọn

- **Ba tầng:** `core` (state machine, không biết vẽ) → `ui` (mới: vẽ + layout + hit-test, không biết OS, gọi qua trait)
  → vỏ OS (`platform/windows`, `platform/macos`: cửa sổ, sự kiện, cài đặt trait, API hệ thống).
- **Vỏ macOS giữ nguyên các quy tắc của vỏ Windows** (CLAUDE.md): một UI thread, mọi nguồn khác `emit(Action)`, state
  cửa sổ trong `RefCell` và không giữ borrow khi gọi AppKit, `Rect` là physical px toàn desktop, surface chỉ sống khi
  cửa sổ hiện, cửa sổ phụ không lấy focus.
- **Cài đặt (Settings) không dùng chung**: Windows dùng control Win32, Mac dựng bằng control AppKit (đúng SPEC 11).
- Hành vi khác nhau giữa OS (hotkey mặc định, OCR không cần tải gói) **đi vào `core` trước, kèm test**.

## Lộ trình

| Mốc | Nội dung | Xong khi |
|---|---|---|
| **0. Refactor Windows** | Crate `crates/ui`; vỏ Windows chỉ còn cửa sổ + sự kiện + cài đặt trait. Không đổi hành vi | CI xanh; test hồi quy trên VM pass; UI giống bản trước |
| **1. MVP macOS** | Spike, khung app, menu bar, hotkey, quyền, overlay, chụp, lưu, clipboard, stack, frame/toolbar, Cài đặt, mở cùng hệ thống, Freeze, đóng gói `.app` tự ký | Pass SPEC mục 10 và 14.7 (bản Mac, mục 8 dưới đây) trên máy dev |
| **2. Preview + OCR** | Preview dùng chung code vẽ; OCR bằng Vision; Live Text | Pass 17.6 và 15.8 trên Mac |
| **3. Tự chọn vùng** | `CGWindowListCopyWindowInfo` + `AXUIElement` (quyền Accessibility) + khung từ ảnh (core sẵn có) | Pass 16.7 trên Mac |
| **4. Phát hành** | `.dmg`, job CI macOS, README / SPEC hoàn thiện | Cài từ `.dmg` chạy được trên một tài khoản macOS mới |

Mỗi mốc là một nhánh + một PR. Mốc 1–4 chạy thử thẳng trên máy dev (macOS 27.2); chỉ Mốc 0 cần VM Windows.

## 1. Mốc 0: crate `ui` và trait `Canvas`

### Ranh giới

Chuyển sang `crates/ui` (phụ thuộc `chup-core`, **không** phụ thuộc `windows` hay `objc2`):

| Từ vỏ Windows | Sang `ui` |
|---|---|
| `gfx.rs`: `Color`, `Theme`, thao tác pixel của `Surface` (`fill`, `outline_outside`, `outline_dashed_outside`, `blend_*`, `copy_rect_from`, `over`), `PillPainter` | `ui::color`, `ui::pixels`, `ui::pill` |
| `overlay.rs`: `paint`, `place_label`, sprite nhãn kích thước | `ui::overlay` |
| `frame.rs`: `paint`, `hit`, `handle_origins`, `Part` | `ui::frame` |
| `toolbar.rs`: `Btn`, `BarButton`, `button_width`, `bar_width`, `paint_bar`, `hit` | `ui::toolbar` |
| `stack.rs`: phần tính layout trong `relayout`, `thumb_h`, vẽ thumbnail / nút / toast, `Hit` | `ui::stack` |
| `preview.rs`: `compose`, `edge`, `hit`, `min_size`, `Btn`, `Grip` | `ui::preview` |

Ở lại vỏ Windows: tạo / hiện / ẩn cửa sổ, `wndproc`, `present` (`UpdateLayeredWindowIndirect`), DPI, focus, kéo thả,
tray, hotkey, worker, `settings.rs`, và **cài đặt trait** bằng Direct2D / DirectWrite (`gfx.rs` còn `D2dCanvas`,
`DwFont`).

### Trait

```rust
// ui::pixels: view lên bộ nhớ của surface (BGRA premultiplied, top-down), không sở hữu.
pub struct Pixels<'a> { pub data: &'a mut [u32], pub w: i32, pub h: i32 }
// Mỗi vỏ có Surface riêng (Windows: DIB section; macOS: Vec<u32>) và trả về Pixels: không copy ở 4K.

pub trait Canvas {
    type Font: FontMetrics;
    type Image;                                   // ảnh đã upload (ID2D1Bitmap / CGImage)
    fn font(&self, px: f32, semibold: bool, align: Align, wrap: bool) -> Option<Self::Font>;
    fn image(&self, img: &RgbaImage) -> Option<Self::Image>;
    fn begin(&mut self, clear: Color);
    fn begin_over(&mut self);
    fn end(&mut self);
    fn push_clip(&mut self, r: RectF);
    fn pop_clip(&mut self);
    fn fill_round(&mut self, r: RectF, radius: f32, c: Color);
    fn stroke_round(&mut self, r: RectF, radius: f32, width: f32, c: Color);
    fn shadow(&mut self, r: RectF, radius: f32, spread: f32, dy: f32, c: Color);
    fn text(&mut self, text: &str, font: &Self::Font, r: RectF, c: Color);
    fn icon(&mut self, icon: Icon, x: f32, y: f32, size: f32, c: Color);   // path từ core::icons
    fn image_cover(&mut self, img: &Self::Image, r: RectF, radius: f32);
    fn image_at(&mut self, img: &Self::Image, r: RectF, radius: f32, origin: (f32, f32), scale: f32);
}
pub trait FontMetrics { fn measure(&self, text: &str, max_w: f32) -> (f32, f32); }
```

Danh sách hàm lấy đúng từ `gfx::Canvas` hiện tại (đã đủ cho toolbar, stack, Preview). `RectF` là kiểu riêng của `ui`
(không lộ `D2D_RECT_F`). Hàm vẽ trong `ui` generic theo `C: Canvas`, nên không có chi phí dyn.

### Test

- Unit test layout / hit-test trong `ui` (chạy được trên macOS): vị trí nút toolbar, layout stack theo số shot / cạnh /
  scale, handle frame, vùng resize Preview.
- `RecordingCanvas` (trong `#[cfg(test)]`) ghi lại danh sách lệnh vẽ. Test so sánh với danh sách mong đợi cho vài
  trạng thái tiêu biểu, để sau này đổi code vẽ biết ngay cái gì đổi.
- Hồi quy trên VM Windows: chụp màn hình overlay / frame (Normal, Faded) / toolbar / stack có toast / Preview phóng to
  + chọn chữ / Cài đặt, ở 100% và 150%, sáng và tối; so với bản `main` trước refactor. Đo RAM idle (≤ 100 MB, SPEC 9).

## 2. Mốc 1: vỏ macOS (MVP)

### 2.1 Spike trước (khoảng 1 ngày, `tools/mac-spike/` ngoài workspace)

Mỗi câu hỏi có cách kiểm và phương án dự phòng; kết quả ghi vào `docs/plans/<ngày>-macos-spike.md`.

| Câu hỏi | Dự phòng nếu không được |
|---|---|
| `SCScreenshotManager` chụp một màn hình ở độ phân giải gốc, loại cửa sổ của chính app bằng `SCContentFilter(excludingApplications:)`. Thời gian chụp? | Ẩn cửa sổ của app, chờ 1 frame, chụp, hiện lại (fallback SPEC 6.4) |
| `NSPanel` kiểu `.nonactivatingPanel` làm key window để nhận Esc **mà app đang dùng không mất focus** | Bắt Esc bằng hotkey toàn cục tạm thời khi overlay hiện |
| Hotkey của `global-hotkey` có chạy khi menu của app khác đang mở (event tracking)? Freeze cần điều này | Đổi sang `CGEventTap` (cần quyền Accessibility) cho riêng hotkey Freeze |
| Quyền Screen Recording với app tự ký: build lại có giữ quyền không; macOS 26+ có hỏi lại định kỳ không | Ghi rõ trong README; app phát hiện mất quyền và hướng dẫn cấp lại |
| Panel trong suốt: click ở vùng không có nút có lọt xuống app bên dưới khi `hitTest:` trả nil | Thu nhỏ panel sát nội dung, đổi kích thước theo layout |
| `drag` crate kéo PNG từ `NSPanel` không activate sang Finder, Slack, trình duyệt | Tự gọi `NSDraggingSession` qua `objc2` |
| Hai màn hình khác scale (Retina + màn ngoài 1x): ánh xạ toạ độ (mục 2.4) có đúng không | Xem lại công thức trong mục 2.4 |

### 2.2 Cấu trúc

```
crates/app/src/platform/macos/
├─ mod.rs        # run(): NSApplication (activation policy .accessory), single instance, vòng lặp
├─ app.rs        # App: Action -> Model -> Effect (giống windows/app.rs)
├─ main_thread.rs# emit(Action) từ mọi thread -> hàng đợi + dispatch_async lên main queue
├─ panel.rs      # tạo NSPanel không viền, trong suốt, layer-backed; present(Surface)
├─ gfx.rs        # Surface (Vec<u32>), CgCanvas: Canvas trên CGBitmapContext, CtFont: FontMetrics
├─ overlay.rs  frame.rs  toolbar.rs  stack.rs        # cửa sổ + sự kiện, vẽ bằng ui::*
├─ capture.rs    # ScreenCaptureKit, quyền Screen Recording
├─ tray.rs  hotkeys.rs  settings.rs  util.rs  logger.rs
```

Crate mới (target macOS, pin `=`, kiểm signature trên docs.rs trước khi chốt version): `objc2`, `objc2-foundation`,
`objc2-app-kit`, `objc2-core-graphics`, `objc2-core-text`, `objc2-screen-capture-kit`, `objc2-service-management`,
`block2`, `dispatch2`. `global-hotkey`, `tray-icon`, `drag` đổi từ `cfg(windows)` sang dùng chung.

### 2.3 Cửa sổ

| Cửa sổ | Loại | Level | Nhận chuột | Focus |
|---|---|---|---|---|
| overlay (mỗi `NSScreen` một cái) | `NSPanel` `.nonactivatingPanel`, không viền | `.screenSaver` (trên menu bar, Dock) | có | key window (nhận Esc), không activate app |
| frame | `NSPanel` | `.statusBar` | Normal: có; Faded: `ignoresMouseEvents = true` | không |
| toolbar, stack | `NSPanel` | `.statusBar` | có | không |
| Cài đặt | `NSWindow` thường | normal | có | có (activate app khi mở, trả về `.accessory` khi đóng) |

Mọi panel: `collectionBehavior = canJoinAllSpaces | fullScreenAuxiliary | stationary | ignoresCycle` (hiện trên mọi
Space và trên app toàn màn hình), `hasShadow = false`, `isOpaque = false`, nền trong suốt.

**Vẽ và hiện:** `Surface` là `Vec<u32>` BGRA premultiplied, đúng layout của `CGBitmapContext` với
`kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little`, nên pixel op của `ui` và `CgCanvas` dùng chung một
bộ nhớ. `CgCanvas` lật trục y một lần để toạ độ top-down như Windows. Hiện = tạo `CGImage` từ buffer rồi gán
`layer.contents` (với `contentsScale` = scale của màn hình). Dirty rect như Windows: chỉ vẽ lại phần đổi, nhưng
`layer.contents` là cả ảnh (đo ở spike; overlay 5K có thể cần chia tile).

Nhận chuột ở pixel trong suốt: macOS tự quyết theo alpha của cửa sổ (không giống luật alpha 0 / 1 của Windows) và
khác nhau giữa view thường và layer-backed. Không dựa vào đó: mỗi panel tự định nghĩa vùng nhận chuột bằng `hitTest:`
(trả nil ngoài vùng có nút / thumbnail), overlay luôn nhận cả màn hình. Spike kiểm lại.

### 2.4 Toạ độ và scale

macOS dùng point, gốc ở góc dưới-trái màn hình chính, trục y hướng lên; mỗi màn hình có `backingScaleFactor` riêng.
`Rect` của core vẫn là **physical px toàn desktop**, quy đổi duy nhất ở `platform/macos/coords.rs`:

- Lật trục: `y_top = maxY(màn hình chính) - y`.
- `S = max(backingScaleFactor)` của mọi màn hình đang cắm.
- Gốc màn hình (px) = gốc (point, đã lật) × `S`; kích thước màn hình (px) = kích thước (point) × scale của chính nó.

Cách này không bao giờ làm hai màn hình chồng lên nhau trong toạ độ px. Màn hình có scale < `S` để lại khoảng trống
giữa các màn hình, nơi không có pixel nào; vô hại vì vùng chọn luôn nằm trong một màn hình (SPEC mục 2). Đổi cấu
hình màn hình (`NSApplicationDidChangeScreenParametersNotification`) thì tính lại `S`, đóng frame như SPEC 7 đang làm
với Windows. Công thức là hàm thuần, đặt trong `core::coords` để test (`macos_layout(&[ScreenPt]) -> Vec<MonitorInfo>`).

### 2.5 Chụp, quyền, loại UI khỏi ảnh

- Chụp bằng `SCScreenshotManager.captureImage(contentFilter:configuration:)`: một màn hình, độ phân giải gốc, **loại
  mọi cửa sổ của Chup** qua `SCContentFilter(display:excludingApplications:[chup])`. Đây là cách thay
  `sharingType = .none` (SPEC 6.4 đã nghi không còn tác dụng với API mới). Crop vùng rồi gọi `core::capture::persist_shot`
  như Windows; `core::capture::grab_*` (xcap) chỉ dùng trên Windows / Linux.
- Freeze: chụp mọi màn hình song song, xong thì `freeze_grabbed` như Windows.
- API trả kết quả qua completion block ở thread khác: block chỉ `emit(Action)`, không đụng App.
- **Quyền Screen Recording:** `CGPreflightScreenCaptureAccess()` trước mỗi lần mở overlay. Chưa có quyền thì
  `CGRequestScreenCaptureAccess()` (lần đầu) hoặc hiện hộp thoại hướng dẫn kèm nút mở
  `x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture`. Không mở overlay khi chưa có quyền
  (tránh chụp ra ảnh chỉ có hình nền).

### 2.6 Phần còn lại của MVP

- **Event loop:** `NSApplication.run()`. `emit(Action)` đẩy vào hàng đợi chung rồi `dispatch_async` lên main queue một
  lần (gộp nhiều action), giống `WM_APP_ACTION` của Windows.
- **Single instance:** lock file (`flock`) trong thư mục dữ liệu; instance thứ hai báo instance đầu (qua
  `NSDistributedNotificationCenter`) rồi thoát.
- **Đường dẫn:** config, thumbnail: `~/Library/Application Support/app.chup.desktop/`; log:
  `~/Library/Logs/Chup/chup.log`; thư mục lưu mặc định: `~/Pictures/Chup`.
- **Menu bar:** `tray-icon` (NSStatusItem), icon dạng template (tự đổi theo sáng / tối). Cùng các mục menu như tray
  Windows, chuỗi qua `core::i18n`.
- **Hotkey:** `global-hotkey` trên main thread. Mặc định theo OS (đặt trong `core::config`, có test):
  Chọn vùng `Ctrl+Super+A`, Chụp lại `Ctrl+Super+X`, Freeze `Ctrl+Super+Z`, OCR chưa gán. (`Super` = phím Cmd theo
  định dạng của `global-hotkey`. Tránh `Cmd+Shift+3/4/5` của hệ thống; F1 trên bàn phím Mac là phím độ sáng nên không
  dùng làm mặc định.) Chốt lại sau spike nếu trùng với phím quen dùng.
- **Clipboard:** `arboard` (đã có trong core cho non-Windows) cho ảnh và chữ. `copy_file_image` ghi thêm file URL lên
  `NSPasteboard` để dán vào Finder ra file: làm ở Mốc 2 nếu cần.
- **Stack kéo thả:** `drag` crate với `NSView` của panel stack; chuột phải / nút hover như Windows.
- **Mở cùng hệ thống:** `SMAppService.mainApp.register()` / `unregister()`; trạng thái đọc từ `SMAppService.status`.
- **Cài đặt:** `NSWindow` + `NSTabViewController` kiểu toolbar (chuẩn của Settings trên macOS), cùng các trang và cùng
  `apply_patch` của core như bản Windows. Ô ghi hotkey bắt phím bằng `NSEvent.addLocalMonitorForEvents`, đổi ra định dạng
  của `core::hotkey_name`.
- **Sáng / tối:** đọc `NSApp.effectiveAppearance`, nghe đổi qua KVO; `ui::Theme` dùng chung.
- **Ngôn ngữ:** `NSLocale.preferredLanguages` thay cho `system_is_vietnamese()` của Windows.

## 3. Mốc 2: Preview, OCR, Live Text

- **Preview:** `NSPanel` có thể nhận focus (Preview là cửa sổ người dùng làm việc với), vẽ bằng `ui::preview`, logic
  phóng to / kéo / chọn chữ đã ở `core::preview` và `core::ocr::layout`.
- **OCR:** `VNRecognizeTextRequest`, `recognitionLevel = .accurate`, `recognitionLanguages = ["vi-VN", "en-US"]` cho
  "Tiếng Việt + Anh", `["en-US"]` cho "Tiếng Anh". Chạy trên worker thread trong process (không có helper exe, không
  có PROTO). Kết quả đổi sang `core::ocr::layout::Line` như hai engine hiện có.
- **Live Text theo ký tự:** `VNRecognizedText.boundingBox(for: range)` cho bbox từng ký tự thật, tốt hơn Windows OCR
  (phải ước lượng theo độ rộng ký tự). Toạ độ Vision chuẩn hoá, gốc dưới-trái: đổi trong vỏ.
- **Core:** vỏ Mac đặt `ocr_vi = Ready` khi khởi động nên `AskOcrDownload` không bao giờ phát; trang OCR của Cài đặt
  trên Mac ẩn phần "gói tiếng Việt". Không đổi state machine.

## 4. Mốc 3: Tự chọn vùng

- Cửa sổ: `CGWindowListCopyWindowInfo(.optionOnScreenOnly | .excludeDesktopElements)` lấy bounds + layer, chụp đồng bộ
  trên UI thread ngay trước khi hiện overlay (như Windows). Bỏ cửa sổ của Chup và layer menu bar / Dock.
- Phần tử: `AXUIElementCopyElementAtPosition` rồi đi lên / xuống cây như worker UIA, trên một worker thread riêng.
  Cần quyền Accessibility: chưa có thì chỉ khung cửa sổ + khung từ ảnh, Cài đặt có nút xin quyền.
- `core::autoselect` dùng lại nguyên; thêm `FrameKind::from_ax(role)` cạnh `from_uia`.

## 5. Mốc 4: Đóng gói, ký, CI

- `scripts/macos-bundle.sh`: build `--release` cho arm64 (máy Intel chạy macOS 26 ít, thêm x86_64 + `lipo` khi có
  người cần), dựng `Chup.app` (Info.plist: `CFBundleIdentifier = app.chup.desktop`, `LSUIElement = true`,
  `LSMinimumSystemVersion = 26.0`), ký `codesign --options runtime --sign "$CHUP_SIGN_ID"`, tạo `.dmg` bằng `hdiutil`.
- `scripts/macos-make-cert.sh`: tạo chứng chỉ code signing tự ký vào keychain đăng nhập, một lần trên máy dev. Đổi sang
  Developer ID chỉ cần đổi `CHUP_SIGN_ID` và thêm bước `notarytool`.
- CI: job `macos-latest` chạy `cargo test --workspace`, clippy cho target macOS, build `.app` **ký ad-hoc** làm artifact
  (CI không có chứng chỉ của máy dev; bản dùng thật vẫn build và ký trên máy dev).

## 6. Thay đổi ở core

- `config`: hotkey mặc định theo `cfg(target_os)`; migrate không đụng tới config cũ. Test cho cả hai bộ mặc định.
- `coords`: hàm thuần `macos_layout` (mục 2.4) + test (một màn, hai màn cùng scale, Retina + 1x đặt trái / phải /
  trên, màn hình chính không ở gốc).
- `autoselect`: `FrameKind::from_ax`.
- `i18n`: chuỗi mới (quyền Screen Recording, quyền Accessibility, tên menu Mac) đủ vi + en.
- `capture`: `grab_region` / `grab_monitors` (xcap) chỉ biên dịch trên OS không phải macOS; `persist_shot`,
  `encode_png` dùng chung.
- `main.rs` / `platform/mod.rs`: thêm nhánh `cfg(target_os = "macos")`; Linux vẫn in thông báo chưa hỗ trợ.

## 7. Rủi ro

| Rủi ro | Ảnh hưởng | Xử lý |
|---|---|---|
| Refactor Mốc 0 làm lệch UI Windows (thứ tự vẽ, làm tròn toạ độ) | Hồi quy khó thấy | `RecordingCanvas` + so ảnh chụp trước / sau trên VM, PR riêng |
| Hiệu năng `layer.contents` với overlay 5K / 6K (Pro Display XDR: ~117 MB mỗi buffer) | Overlay giật khi kéo | Đo ở spike; chia tile hoặc chỉ cập nhật sublayer của vùng chọn |
| Hotkey không chạy khi menu app khác đang mở | Freeze không chụp được menu (mục đích chính của Freeze) | Spike; dự phòng `CGEventTap` |
| Quyền Screen Recording bị hỏi lại định kỳ trên macOS mới | Phiền người dùng | Không tránh được ở mức app; phát hiện và hướng dẫn |
| Chứng chỉ tự ký hết hạn / máy mới | Mất quyền đã cấp | Script tạo lại; README ghi cách cấp lại quyền |
| `objc2-*` đổi API giữa các bản | Build lỗi khi nâng | Pin `=`, nâng có chủ đích |

## 8. Nghiệm thu Mốc 1 trên macOS (SPEC 10 và 14.7, bản Mac)

Như SPEC 10, đổi các chỗ riêng Windows:
- Chụp xong, app đang dùng **không** mất focus (kiểm với TextEdit đang gõ dở, Safari).
- Overlay hiện trên app toàn màn hình và trên mọi Space; không lọt frame / toolbar / stack vào ảnh.
- Màn Retina + màn ngoài 1x: chụp đúng pixel ở cả hai, vùng chọn không lệch.
- Chưa có quyền Screen Recording: không mở overlay, có hướng dẫn; cấp quyền xong chụp được ngay (hoặc báo cần mở lại app
  nếu macOS yêu cầu).
- Hotkey trùng phím của app khác: có báo lỗi, app vẫn chạy.
- Bật / tắt mở cùng hệ thống; build lại bằng cùng chứng chỉ không mất quyền.
- Freeze (14.7): chụp được menu chuột phải của Finder, menu của Safari đang mở, video đang chạy.

## 9. Cập nhật SPEC (cùng nhánh)

- Mục 3: macOS 26+, Vision cho OCR trên Mac.
- Mục 6.4: macOS dùng `SCContentFilter` loại app khỏi ảnh, bỏ `sharingType`.
- Mục 11: viết lại theo tài liệu này (cửa sổ, toạ độ, quyền, hotkey mặc định, đóng gói).
- Mục 12: thay bước 3.1 bằng các mốc 3.1.0 – 3.1.4.
- Mục 13: thêm các quyết định ngày 2026-11-05.

## 10. Kết quả thực hiện (2026-11-05)

Code của cả 5 mốc đã xong trên các nhánh `macos-ui-crate` (Mốc 0, PR #16) và `macos-mvp` (Mốc 1–4). Khác với kế hoạch:

- **Không có spike riêng** (`tools/mac-spike/`): các câu hỏi ở mục 2.1 được trả lời ngay trong vỏ thật, kiểm bằng chế độ
  tự kiểm tra `CHUP_SELFTEST` (bơm sự kiện vào handler của cửa sổ, xuất ảnh từng cửa sổ ra PNG). Câu hỏi cần quyền Ghi
  màn hình / phím tắt thật / kéo thả thật vẫn mở (máy dev chạy từ terminal không có quyền Ghi màn hình).
- **Toạ độ:** thêm `MacLayout::cg_to_px` / `px_to_cg` cho toạ độ CoreGraphics (gốc trên-trái, `kCGWindowBounds`,
  Accessibility).
- **Vẽ:** mỗi lần cập nhật cửa sổ chép cả surface thành `CGImage` rồi gán `layer.contents` (không theo vùng bẩn).
  Overlay 3600×2338: hiện 76 ms, kéo chọn vẫn mượt trên M-series; chưa cần chia tile.
- **Pixel trong suốt nhận chuột:** giữ mẹo alpha 1/255 như Windows cho lề frame; overlay luôn nhận chuột.
- **Tự chọn vùng Live chưa có khung từ ảnh** (cần chụp ngầm từ worker; Freeze có).
- **`logger`, `hotkeys`, `tray`** chuyển sang `platform/common/` dùng chung hai vỏ.
- **Đo:** RAM rảnh ~70 MB, overlay mở ~235 MB (surface + ảnh của layer), Freeze 48 ms từ phím tắt tới lúc ảnh hiện,
  OCR Vision ~0,2 s cho ảnh 1400×120 (lần đầu tiếng Việt ~50 s để macOS chuẩn bị model).

