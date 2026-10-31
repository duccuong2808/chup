# Chup

Chụp màn hình theo vùng cho Windows. Ảnh vừa chụp nằm thành stack ở cạnh màn hình
và kéo thả thẳng được sang app khác. Đặc tả đầy đủ ở [SPEC.md](SPEC.md).

- **Trạng thái**: Phase 1 (v1, Windows) xong, cộng bước 1.5: bỏ Tauri/WebView2, giao diện
  vẽ native bằng Win32 + Direct2D/DirectWrite.
- **Rust thuần, không WebView.** Installer ~480 KB, `chup.exe` ~1 MB, RAM khi rảnh dưới 1 MB
  (xem [Số đo](#số-đo)).

## Yêu cầu

- Chạy: Windows 10 bản 2004 trở lên hoặc Windows 11, 64-bit. Không cần runtime nào thêm
  (MSVC runtime đã link tĩnh, không dùng WebView2).
- Build: [Rust](https://rustup.rs) stable (≥ 1.82), toolchain `x86_64-pc-windows-msvc`,
  Visual Studio Build Tools (workload "Desktop development with C++"), và
  [NSIS](https://nsis.sourceforge.io) nếu cần đóng gói installer.

## Build và chạy

```powershell
cargo build --release -p chup        # target\release\chup.exe
makensis /DVERSION=0.2.0 /DEXE=target\release\chup.exe installer\chup.nsi
```

- Installer cài cho user hiện tại vào `%LOCALAPPDATA%\Chup`, không cần quyền admin.
  Chạy với `/S` để cài im lặng.
- Chup chạy nền ở khay hệ thống, **không mở cửa sổ nào khi khởi động**.
  - Nhấn `Ctrl+Alt+A` hoặc click trái icon tray để chọn vùng.
  - Click phải icon tray để mở menu (Cài đặt, Thoát…).
  - Mở `chup.exe` lần nữa khi đang chạy: bản đang chạy mở Cài đặt.

Test và lint (lõi chạy được trên mọi OS; code Windows check được từ macOS/Linux, chỉ không link được):

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p chup --target x86_64-pc-windows-msvc -- -D warnings
```

### CI/CD (GitHub Actions)

Workflow `.github/workflows/build.yml` chạy trên `windows-latest`:

- Mỗi lần push / PR: job `check` (`cargo test`, `cargo clippy -D warnings`) và job `build`
  (`cargo build --release` + NSIS) chạy song song. Kết quả là artifact `chup-windows-<sha>`
  gồm installer và `chup.exe` (tab Actions → chọn run → Artifacts).
- Push tag `v*` (vd: `git tag v0.2.0 && git push origin v0.2.0`): tạo thêm GitHub Release dạng
  draft kèm installer. Nhớ đổi `version` trong `Cargo.toml` (workspace) cho khớp tag trước.

## Dùng nhanh

| Thao tác | Cách làm |
|---|---|
| Chọn vùng | `Ctrl+Alt+A`, hoặc click trái icon tray |
| Huỷ chọn vùng | `Esc`, hoặc click phải |
| Chụp | Nút **Chụp** trên toolbar, hoặc `Ctrl+Alt+S` |
| Chụp lại vùng cuối | `Ctrl+Alt+S` khi không có vùng chọn, hoặc menu tray |
| Chế độ mờ | Nút **Mờ**: viền nhạt, chuột đi xuyên qua, vẫn chụp được bằng phím tắt |
| Di chuyển / đổi kích thước vùng | Kéo phần lề quanh viền / kéo 8 ô vuông nhỏ |
| Dùng ảnh trong stack | Kéo thumbnail thả vào app khác; click để mở ảnh; hover để copy / mở folder / bỏ khỏi stack |
| Thu gọn stack | Hover stack → nút `›`; click tab nhỏ ở mép màn hình để mở lại |

## Cấu trúc code

```
crates/core/src/            chup-core: không phụ thuộc OS, unit test được
├─ model.rs        state machine: nhận sự kiện, trả Vec<Effect> (mọi logic hành vi ở đây)
├─ capture.rs      grab_region / persist_shot, tên file, thumbnail
├─ clipboard.rs    đưa ảnh vào clipboard (Windows: Win32 trực tiếp, xem dưới)
├─ coords.rs       hình học: Rect, màn hình, vị trí toolbar/stack, lề frame
├─ config.rs       load / save / migrate / patch config.json
├─ stack.rs        stack tối đa 5 shot
├─ hotkey_name.rs  ghi phím tắt: mã VK -> "Ctrl+Alt+A"
└─ icons.rs        icon dạng SVG path
crates/app/src/platform/windows/   bin `chup`: vỏ Win32
├─ mod.rs          message loop, single instance, timer (đổi màn hình, làm nóng, trim)
├─ app.rs          Action -> Model -> áp Effect lên cửa sổ
├─ win.rs          đăng ký class, wndproc chung, hàng đợi Action (emit)
├─ gfx.rs          Surface (DIB BGRA premultiplied), Direct2D software + DirectWrite
├─ overlay.rs      chọn vùng, mỗi màn hình 1 layered window
├─ frame.rs        viền + 8 handle, kéo/resize, chế độ Mờ (click-through)
├─ toolbar.rs      nút Chụp / Mờ / Đóng
├─ stack.rs        thumbnail, nút hover, kéo thả file, toast
├─ settings.rs     cửa sổ Cài đặt (control Win32 gốc)
├─ tray.rs, hotkeys.rs, logger.rs, util.rs
installer/chup.nsi  installer NSIS
```

- **Cấu hình**: `%APPDATA%\app.chup.desktop\config.json`.
- **Log và cache thumbnail**: `%LOCALAPPDATA%\app.chup.desktop\` (`chup.log`, `thumbs\`).

## Số đo

Đo trên VM Windows 11 24H2, màn hình 3600×2024, DPI 192 (200%), bản release từ CI.
RAM là private working set.

| | Bản Tauri (v0.1) | Bản native (v0.2) |
|---|---|---|
| Installer | 1,4 MB | ~480 KB |
| RAM vừa khởi động | 104–126 MB (cả các process WebView2) | 5,1–5,4 MB |
| RAM khi rảnh (sau 15 s) | ~153 MB sau khi dùng | 0,3–1,9 MB |
| CPU khi rảnh | | 0% (mọi timer đều chạy 1 lần, không có vòng lặp poll) |

Bản native:
- App sẵn sàng 42–80 ms sau khi chạy.
- Overlay hiện sau ~35 ms khi bấm hotkey (lần đầu sau khi trim ~113 ms).
- Kéo chọn vùng 4K (44 lần di chuột, cả hiện frame/toolbar) tốn tổng ~0,17 s CPU.
- Chụp 42–114 ms; lưu (PNG + thumbnail + clipboard) 120–460 ms. Số này đo trước khi bỏ arboard.
  Hiện PNG chỉ mã hoá 1 lần cho cả file và clipboard, nên lưu nhanh hơn.
- Đỉnh RAM khi chụp vùng 2812×1350: ~54 MB, rảnh 15 s thì trả lại hệ điều hành.

Cách giữ RAM thấp:
- Bộ nhớ đồ hoạ chỉ cấp khi cửa sổ đang hiện: DIB overlay (cỡ màn hình, ~29 MB ở 4K) và DIB
  frame giải phóng khi ẩn / khi thôi kéo.
- Rảnh 15 s: giải phóng DIB overlay còn giữ, `HeapCompact`, `EmptyWorkingSet`.
- Clipboard không qua arboard: dùng lại byte PNG có sẵn, dựng CF_DIBV5 trong 1 lượt ghi.

## Đã kiểm chứng và chưa kiểm chứng

**Đã test trên Windows 11 thật (VM), bản native:**
- Overlay chọn vùng; frame 8 handle; resize và kéo di chuyển frame, toolbar chạy theo.
- Nút Chụp: ảnh sạch (không dính viền, toolbar, stack), đúng kích thước.
- `WDA_EXCLUDEFROMCAPTURE` có tác dụng với cách chụp của `xcap`.
- Stack: hover hiện nút và nhãn W × H; kéo thả thumbnail ra Desktop; click thẻ mở ảnh;
  thu gọn và mở lại; toast lỗi khi file bị xoá bên ngoài (thẻ tự bỏ khỏi stack).
- Chế độ Mờ: click xuyên qua, `Ctrl+Alt+S` chụp nhiều lần liên tiếp.
- Không cướp focus: sau khi chụp, app đang dùng vẫn giữ focus.
- Single instance: chạy lần 2 thì mở Cài đặt.
- Cài đặt: ghi phím tắt, báo lỗi tổ hợp đã bị app khác chiếm, `Esc` huỷ ghi phím.

- Clipboard (khi chụp và nút Copy, ảnh 2193×1350): có đủ `PNG` (đúng byte của file), CF_DIBV5,
  và `Bitmap`/`DIB` do Windows tự suy ra. Chuẩn bị ~16–20 ms, giữ clipboard ~9–10 ms.
- Nút mở folder (Explorer chọn sẵn file), nút bỏ khỏi stack, Xoá hết, `Ctrl+Alt+S` chụp lại vùng cuối.
- Menu tray (Mở folder ảnh, Cài đặt…); bật/tắt chạy cùng Windows (khoá `HKCU\…\Run`);
  đổi folder lưu qua hộp chọn folder, ảnh sau đó lưu đúng folder mới.

Lưu ý khi test trên VM VMPal: agent của VMPal đồng bộ clipboard sang máy Mac. Gặp ảnh lớn
trong clipboard, nó mất kết nối vài giây rồi ghi text cũ đè lên. Lỗi này có cả khi PowerShell
tự đặt ảnh, nên không do Chup. Muốn kiểm tra clipboard thì phải đọc ngay trong ~2 giây đầu.

**Chưa test:**
1. Nhiều màn hình, nhất là màn hình khác DPI (100% + 150%). Phần quy đổi toạ độ có unit test.
2. Tooltip/menu có hiệu ứng trong suốt có lọt vào ảnh không (`xcap` chụp bằng GDI `BitBlt`
   không có cờ `CAPTUREBLT`).

## Khác với SPEC

- **Frame không bị ẩn khi dùng chế độ "ẩn UI lúc chụp".**
  - Viền nằm hoàn toàn ngoài vùng chụp, phần trong vùng trong suốt, nên frame không bao giờ dính vào ảnh.
  - Không ẩn frame thì không phải hiện lại nó, tránh làm mất focus app đang dùng.
- **Chưa có tooltip** cho nút trên toolbar và stack.
- **Trim khi rảnh**: sau 15 s không thao tác, app giải phóng bộ đệm và trả working set
  cho hệ điều hành. Lần chọn vùng đầu tiên sau đó chậm hơn (đo được ~113 ms so với ~35 ms).
- **Cài đặt bố cục 2 cột**.
