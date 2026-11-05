# Chup

Chụp màn hình theo vùng cho Windows và macOS. Ảnh vừa chụp nằm thành stack ở cạnh màn hình và kéo thả thẳng được sang app
khác. Có chụp đóng băng (menu, tooltip), Preview, OCR / chọn chữ trên ảnh, tự động chọn vùng; giao diện tiếng Việt và
tiếng Anh. Đặc tả đầy đủ ở [SPEC.md](SPEC.md).

- **Trạng thái**: Phase 1 và Phase 2 (Freeze, Preview, OCR, Live Text, tự chọn vùng) xong, đã dùng trên Windows 11 x64
  thật. **macOS 26+** (Phase 3.1): đủ tính năng, đã chạy tự kiểm tra trên máy dev; chưa test tay đầy đủ (mục
  [macOS](#macos)). Linux X11 ở Phase 3.2.
- **Rust thuần, không WebView**: Win32 + Direct2D/DirectWrite trên Windows, AppKit + CoreGraphics/CoreText trên macOS,
  chung lõi (`chup-core`) và chung code vẽ (`chup-ui`). `chup.exe` ~1 MB, rảnh thì RAM dưới 1 MB, CPU 0%.

## Cài và dùng

Cần Windows 10 bản 2004 trở lên hoặc Windows 11, 64-bit; không cần runtime nào thêm. Bộ cài cài cho user hiện tại vào
`%LOCALAPPDATA%\Chup`, không cần quyền admin (`/S` để cài im lặng).

- **Full** (`Chup_<ver>_x64-setup.exe`, ~5 MB): có sẵn OCR tiếng Việt, không cần mạng.
- **Lite** (`Chup_<ver>_x64-setup-lite.exe`, ~0,7 MB): chỉ `chup.exe`. Tích ô "OCR tiếng Việt" để tải engine và dữ liệu
  (~11 MB) lúc cài, hoặc tải sau trong Cài đặt. Chạy `chup.exe` rời cũng giống bản Lite.

Chup chạy nền ở khay hệ thống, không mở cửa sổ nào khi khởi động. Mở `chup.exe` lần nữa khi đang chạy thì mở Cài đặt.

| Thao tác | Cách làm |
|---|---|
| Chọn vùng | `Ctrl+Alt+A`, hoặc click trái icon tray. Click phải icon tray mở menu |
| Tự động chọn vùng | Lúc chọn vùng, rê chuột là tô sẵn khung cửa sổ / nút / ảnh dưới con trỏ, click để chọn. Lăn chuột hoặc `↑` `↓` đổi khung lớn hơn / nhỏ hơn; giữ `Shift` để tạm tắt; kéo vẫn chọn tự do |
| Huỷ chọn vùng | `Esc`, hoặc click phải |
| Chụp | Nút **Chụp** trên toolbar, hoặc `Ctrl+Alt+X` (không có vùng chọn thì chụp lại vùng cuối) |
| Chế độ mờ | Nút **Mờ**: viền nhạt, chuột đi xuyên qua, vẫn chụp được bằng phím tắt |
| Chỉnh vùng | Kéo phần lề quanh viền để di chuyển, kéo 8 ô vuông để đổi cỡ |
| Chụp đóng băng | `F1`, hoặc menu tray (có hẹn giờ 3 giây). Lưu: `Enter`, double-click hoặc nút **Lưu**; nút **Copy** chỉ vào clipboard. Cần F1 cho app khác thì tắt trong Cài đặt > Phím tắt |
| Dùng ảnh trong stack | Kéo thumbnail thả vào app khác; click để mở Preview; hover để copy / mở folder / bỏ khỏi stack. Nút `›` thu gọn stack |
| Preview | Kéo để di chuyển, kéo mép để đổi cỡ, lăn chuột phóng to ảnh trong khung (kéo để xem chỗ khác, `Alt` + kéo di chuyển cửa sổ), `Ctrl` + lăn chuột đổi opacity, nút ghim để luôn nằm trên. `Esc` đóng |
| Copy chữ trong ảnh (OCR) | Nút **[T]** trên Preview / toolbar, nút **Chữ** khi Freeze, hoặc hotkey "Chụp text" (gán trong Cài đặt) |
| Chọn chữ trên ảnh | Trong Preview kéo trên chữ để bôi đen, bấm đúp chọn từ, 3 lần chọn dòng, `Ctrl+C` hoặc chuột phải để copy. `Alt`+kéo luôn di chuyển cửa sổ |

Ngôn ngữ giao diện mặc định theo Windows, đổi ở Cài đặt > Chung.

## macOS

Cần **macOS 26** trở lên. Chup chạy nền với icon trên menu bar (không có icon ở Dock); bấm icon để mở menu.

- **Cài**: mở `Chup-<ver>.dmg`, kéo `Chup.app` vào Applications. Bản build tự ký / ký ad-hoc chưa notarize: lần đầu mở
  bằng chuột phải > Mở (hoặc Cài đặt hệ thống > Quyền riêng tư & Bảo mật > Vẫn mở).
- **Quyền Ghi màn hình** (bắt buộc): lần đầu Chup hỏi và dẫn tới Cài đặt hệ thống > Quyền riêng tư & Bảo mật > Ghi màn
  hình & âm thanh hệ thống. Bật Chup, thoát rồi mở lại nếu macOS yêu cầu. Chưa có quyền thì Chup không mở overlay.
- **Quyền Trợ năng** (tuỳ chọn): để tự chọn vùng nhận được nút, ô nhập, đoạn chữ bên trong cửa sổ. Cài đặt > Chụp >
  "Cấp quyền Trợ năng…". Không có thì chỉ nhận khung cửa sổ.
- **Phím tắt mặc định**: `⌃⌘A` chọn vùng, `⌃⌘X` chụp (không có vùng chọn thì chụp lại vùng cuối), `⌃⌘Z` chụp đóng
  băng. Thao tác khác như bảng trên, `Ctrl` đổi thành `⌘` (`⌘C`, `⌘A`), `Alt` thành `⌥`.
- **OCR** dùng Apple Vision có sẵn (tiếng Việt + Anh), không tải gì. Lần đầu dùng tiếng Việt macOS chuẩn bị model
  (có thể mất gần 1 phút, một lần).

Build và đóng gói (trên Mac):

```bash
cargo run -p chup                       # chạy thẳng (quyền Ghi màn hình gắn với app terminal)
scripts/macos-make-cert.sh              # một lần: chứng chỉ "Chup Self-Signed" để quyền không mất khi build lại
scripts/macos-bundle.sh --dmg           # target/macos/Chup.app + Chup-<ver>.dmg, ký bằng chứng chỉ trên
CHUP_SELFTEST=/tmp/chup-st target/release/chup   # tự kiểm tra: chạy luồng chính, xuất ảnh từng cửa sổ ra PNG
target/release/chup --ocr-file anh.png  # OCR một file bằng Vision
```

File của app trên macOS: cấu hình `~/Library/Application Support/app.chup.desktop/config.json`, thumbnail trong
`~/Library/Caches/app.chup.desktop/`, log `~/Library/Logs/Chup/chup.log`, ảnh mặc định `~/Pictures/Chup`.

**Đã kiểm chứng** (máy dev, MacBook Pro Retina, macOS 27.2, 2026-11-05) bằng `CHUP_SELFTEST`: overlay chọn vùng, nhãn
kích thước, frame + toolbar (Normal, Mờ), lưu ảnh, stack (hover, toast tiếng Việt), Freeze (vùng chọn, handle, toolbar,
Enter lưu), tự chọn vùng (khung cửa sổ), Cài đặt (đủ trang), Preview (toolbar, chọn từ theo ký tự, phóng to), OCR câu
tiếng Việt có dấu khớp 100% (~0,2 s). RAM rảnh ~70 MB. Overlay 3600×2338 hiện trong 76 ms, Freeze 48 ms.
**Chưa kiểm chứng** (cần người dùng thật, máy đã cấp quyền): chụp thật bằng ScreenCaptureKit (terminal của máy dev
không có quyền Ghi màn hình), phím tắt toàn cục, kéo thumbnail sang app khác, overlay khi app toàn màn hình / Space khác,
nhiều màn hình khác scale, Accessibility, mở cùng hệ thống, bản Windows sau khi tách `chup-ui` (CI xanh, chưa test VM).

## Build (Windows)

Cần [Rust](https://rustup.rs) stable (≥ 1.82) với toolchain `x86_64-pc-windows-msvc`, Visual Studio Build Tools
("Desktop development with C++"), và [NSIS](https://nsis.sourceforge.io) nếu đóng gói bộ cài.

```powershell
cargo build --release -p chup        # target\release\chup.exe
# Bộ cài Lite
makensis /DLITE /DVERSION=0.2.0 /DEXE=target\release\chup.exe installer\chup.nsi
# Bộ cài Full: lấy đúng engine + dữ liệu ghim trong crates\core\src\ocr\manifest.json (kiểm SHA-256)
installer\fetch-ocr.ps1 -Pack engine -Out ocr-dist
installer\fetch-ocr.ps1 -Pack data -Out ocr-data          # in ra tên thư mục gói, vd. vi-1
makensis /DVERSION=0.2.0 /DEXE=target\release\chup.exe /DOCR_DIR=ocr-dist /DDATA_DIR=ocr-data /DDATA_PACK=vi-1 installer\chup.nsi
```

Test và lint (lõi chạy trên mọi OS; code Windows check được từ macOS/Linux, chỉ không link được):

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p chup --target x86_64-pc-windows-msvc -- -D warnings
# chup-ocr.exe nằm ngoài workspace, cần Tesseract (Windows: vcpkg tesseract:x64-windows-static + VCPKG_ROOT;
# macOS: brew install tesseract pkg-config)
CHUP_TESSDATA=<thư mục có vie/eng.traineddata> cargo test --release --manifest-path crates/ocr-helper/Cargo.toml
```

**CI** (`.github/workflows/build.yml`, `windows-latest` + `macos-latest`): job macOS chạy test + clippy và đóng gói
`Chup.app` (ký ad-hoc) thành artifact `chup-macos-<sha>`. Mỗi PR và mỗi push lên `main` có đổi code chạy test + clippy, build và test
`chup-ocr.exe`, rồi build `chup.exe` và 2 bộ cài, ra artifact `chup-windows-<sha>`. Push tag `v*` (nhớ đổi `version`
trong `Cargo.toml` trước) tạo thêm GitHub Release dạng draft. Commit chỉ đổi `*.md`, `docs/`, `.beads/`, `.claude/`, `tools/` thì không chạy CI.

### Phát hành engine OCR

Engine `chup-ocr.exe` không build lại mỗi lần phát hành Chup: app và bộ cài Full dùng đúng file ghim trong
`crates/core/src/ocr/manifest.json`. Chỉ khi sửa `crates/ocr-helper` hoặc `crates/ocr-proto`:

1. Push, lấy artifact `chup-ocr` của job `ocr-helper` (`chup-ocr.exe` + `NOTICE-ocr.txt`).
2. Tạo Release `ocr-engine-v<N+1>` (không đánh dấu latest; tag không bắt đầu bằng `v` nên không kích hoạt workflow)
   và đính kèm 2 file đó.
3. Sửa mục `engine` trong manifest: `dir` = `engine-v<N+1>`, `baseUrl`, `size` + `sha256` từng file.
   Đổi PROTO thì bắt buộc làm, vì `chup.exe` mới không nói chuyện được với engine cũ.

## Cấu trúc code

```
crates/core/        chup-core: không phụ thuộc OS, unit test được. model.rs là state machine
                    (nhận sự kiện, trả Vec<Effect>), chứa mọi logic hành vi; i18n.rs chứa chuỗi giao diện vi/en
crates/ui/          chup-ui: vẽ + bố cục + hit-test (toolbar, frame, overlay, stack, Preview) qua trait Canvas,
                    không phụ thuộc OS
crates/app/src/platform/windows/   bin `chup`: vỏ Win32. app.rs đổi Action -> Model -> áp Effect lên cửa sổ;
                    mỗi cửa sổ một file (overlay, frame, toolbar, stack, preview, settings…)
crates/app/src/platform/macos/     vỏ AppKit cùng cấu trúc (NSPanel, CoreGraphics/CoreText, ScreenCaptureKit, Vision)
crates/app/src/platform/common/    logger, hotkey, tray dùng chung hai vỏ
scripts/            đóng gói macOS (Chup.app, .dmg, chứng chỉ tự ký)
crates/ocr-proto/   khung tin giữa chup.exe và chup-ocr.exe
crates/ocr-helper/  chup-ocr.exe: Tesseract link tĩnh (ngoài workspace)
installer/          bộ cài NSIS, fetch-ocr.ps1
docs/plans/         thiết kế và spike từng tính năng
```

File của app: cấu hình `%APPDATA%\app.chup.desktop\config.json`; log, cache thumbnail và gói OCR trong
`%LOCALAPPDATA%\app.chup.desktop\` (`chup.log`, `thumbs\`, `ocr\vi-1\`).

## Hiệu năng và kiểm chứng

Đo trên máy x64 thật (Intel Core Ultra 7 255H, Windows 11, 125%), 2026-11-03:

- Rảnh: RAM (private working set) 0,1–0,2 MB sau 15 s, CPU 0%, không có timer định kỳ.
- Chụp 19–48 ms, lưu 27–97 ms; F1 tới lúc ảnh Freeze hiện 47 ms.
- Đang dùng: đỉnh ~75 MB khi Preview + OCR ảnh cỡ màn hình; 5 phút dùng liên tục tốn 4 s CPU.

Mọi tính năng đã test tay trên Windows 11 (VM và máy thật). **Chưa test** (chính): nhiều màn hình / màn hình khác DPI, màn 4K;
kéo thả vào Zalo PC, Slack, Word; cài Lite có tích OCR và Full trên máy không mạng; giao diện tiếng Anh ngoài Cài đặt.

Chi tiết từng lần test, số đo đầy đủ và cách đo: [docs/testing.md](docs/testing.md).

## Khác với SPEC

- **Frame không bị ẩn khi dùng chế độ "ẩn UI lúc chụp".**
  - Viền nằm hoàn toàn ngoài vùng chụp, phần trong vùng trong suốt, nên frame không bao giờ dính vào ảnh.
  - Không ẩn frame thì không phải hiện lại nó, tránh làm mất focus app đang dùng.
- **Chưa có tooltip** cho nút trên toolbar và stack.
- **Trim khi rảnh**: sau 15 s không thao tác, app giải phóng bộ đệm và trả working set
  cho hệ điều hành. Lần chọn vùng đầu tiên sau đó chậm hơn (đo được ~113 ms so với ~35 ms).
- **Cài đặt chia trang** (danh sách trang bên trái, thẻ bên phải, kiểu Windows 11), thay cho một trang dài.
  SPEC 5.6 đã ghi.
- **Freeze, mục tray "Chụp đóng băng"** chờ ~350 ms cho menu tray tắt hẳn rồi mới chụp (F1 chụp ngay).
- **Freeze, lúc đếm ngược hẹn giờ**: icon tray mờ đi kèm tooltip, thay vì một icon riêng.
- **OCR, nút [T] lúc đang chạy**: hiện vòng "bận" đứng yên (không quay) trên nút; bấm lần nữa là huỷ.
- **Overlay chọn vùng bị loại khỏi ảnh chụp** (Phase 2.4, để Live chụp ngầm màn hình nhận khung từ ảnh):
  app chia sẻ màn hình (Teams, Zoom) không thấy overlay. SPEC 16.4 đã ghi.
- **Không dịch**: log, và câu báo file cấu hình hỏng (sinh ra trước khi biết ngôn ngữ). Máy Windows tiếng Anh đã dùng
  Chup từ trước thì app giữ tiếng Việt (config cũ), còn bộ cài hiện tiếng Anh (bộ cài theo Windows, không đọc config).

### macOS

- **Chụp bằng ScreenCaptureKit**, không dùng `xcap` (API Apple đã khai tử); cửa sổ của Chup được loại khỏi ảnh bằng
  bộ lọc của ScreenCaptureKit thay cho `sharingType`. SPEC 6.4 đã ghi.
- **Tự chọn vùng, khung từ hình ảnh chỉ ở Freeze**: Live trên macOS chưa chụp ngầm màn hình để nhận khung (Windows có).
- **Cài đặt dùng tab ở trên** (kiểu Cài đặt của macOS) thay cho danh sách trang bên trái.
- **Preview trên Mac**: kéo nền để di chuyển (không có thanh tiêu đề); `⌘W` cũng đóng; trackpad cuộn 24 pt = 1 nấc
  phóng to. Không có hẹn giờ vẽ lại khi đổi DPI (AppKit tự xử lý scale theo màn hình).
- **Bộ đếm RAM**: rảnh ~70 MB (AppKit + font hệ thống), không xuống dưới 1 MB như Windows (macOS không có trim working
  set kiểu `EmptyWorkingSet`); vẫn trong ngân sách 100 MB của SPEC 9.

