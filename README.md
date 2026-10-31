# Chup

Chụp màn hình theo vùng cho Windows. Ảnh vừa chụp nằm thành stack ở cạnh màn hình
và kéo thả thẳng được sang app khác. Đặc tả đầy đủ ở [SPEC.md](SPEC.md).

Đây là code **Phase 1 (v1, Windows)**, gồm các bước 1.0–1.4 trong SPEC.

## Yêu cầu

- Windows 10 bản 2004 trở lên hoặc Windows 11, 64-bit.
- [Rust](https://rustup.rs) stable, toolchain `x86_64-pc-windows-msvc`.
- Visual Studio Build Tools, workload "Desktop development with C++".
- Node.js 20 trở lên.
- WebView2 Runtime (Windows 11 có sẵn).

## Chạy

```powershell
npm install
npm run tauri dev      # chạy dev, hot reload phần frontend
npm run tauri build    # build installer NSIS
```

- Installer nằm ở `src-tauri/target/release/bundle/nsis/Chup_0.1.0_x64-setup.exe`.
- Chup chạy nền ở khay hệ thống, **không mở cửa sổ nào khi khởi động**.
  - Nhấn `Ctrl+Alt+A` hoặc click trái icon tray để chọn vùng.
  - Click phải icon tray để mở menu (Cài đặt, Thoát…).

Unit test chạy được trên mọi OS:

```powershell
cd src-tauri
cargo test
```

## Dùng nhanh

| Thao tác | Cách làm |
|---|---|
| Chọn vùng | `Ctrl+Alt+A`, hoặc click trái icon tray |
| Huỷ chọn vùng | `Esc`, hoặc click phải |
| Chụp | Nút **Chụp** trên toolbar, `Ctrl+Alt+S`, hoặc `Enter` khi viền đang được focus |
| Chụp lại vùng cuối | `Ctrl+Alt+S` khi không có vùng chọn, hoặc menu tray |
| Chế độ mờ | Nút **Mờ**: viền nhạt, chuột đi xuyên qua, vẫn chụp được bằng phím tắt |
| Di chuyển / đổi kích thước vùng | Kéo phần lề quanh viền / kéo 8 ô vuông nhỏ |
| Dùng ảnh trong stack | Kéo thumbnail thả vào app khác; click để mở ảnh; hover để copy / mở folder / bỏ khỏi stack |

## Cấu trúc code

```
src-tauri/src/
├─ lib.rs        khởi tạo: plugin, state, tray, cửa sổ, hotkey, autostart
├─ actions.rs    các luồng chính: chọn vùng, frame, chụp, stack, đổi màn hình
├─ commands.rs   IPC cho frontend (SPEC 6.7)
├─ capture.rs    grab_region / persist_shot, tên file, thumbnail, clipboard
├─ coords.rs     hình học thuần: CSS px -> physical px, vị trí toolbar/stack
├─ config.rs     load / save / migrate / patch config.json
├─ stack.rs      stack tối đa 5 shot
├─ hotkeys.rs    phím tắt toàn cục, báo lỗi xung đột
├─ tray.rs       icon khay + menu
├─ wm.rs         tạo và đặt vị trí cửa sổ
└─ platform/     code riêng OS (Windows: no-activate, exclude-from-capture)
src/
├─ shared/       ipc.ts (wrapper có kiểu), types.ts, icon, design token
└─ windows/      overlay, frame, toolbar, stack, settings (mỗi cửa sổ 1 trang)
```

- **Cấu hình**: `%APPDATA%\app.chup.desktop\config.json`.
- **Đổi identifier**: sửa `identifier` trong `src-tauri/tauri.conf.json`. Lưu ý: đổi identifier thì Chup đọc config ở thư mục mới.

## Đã kiểm chứng và chưa kiểm chứng

**Đã kiểm chứng** (trong môi trường build, chưa chạy giao diện):
- `cargo check` qua cho cả Linux và target `x86_64-pc-windows-msvc`, gồm cả code riêng Windows trong `platform/windows.rs`.
- Unit test qua: quy đổi toạ độ DPI, đặt toolbar/stack, config (patch, file hỏng, thiếu field), tên file, ghi không đè, fallback folder, thumbnail, stack.
- Tauri build script đã validate các file capability.
- Frontend qua `tsc --strict` và `vite build`.

**Chưa chạy trên Windows thật.** Những điểm dưới đây cần anh test tay theo mục 10 của SPEC, xếp theo rủi ro từ cao xuống thấp:

1. **Stack/toolbar có cướp focus không.**
   - Cửa sổ đã được tạo với `focusable(false)` (Tauri gắn `WS_EX_NOACTIVATE`) và luôn hiện bằng `SW_SHOWNOACTIVATE`.
   - Rủi ro còn lại: WebView2 có thể tự lấy focus khi click vào nội dung.
   - Nếu bị: cần subclass cửa sổ con của WebView2 để trả về `MA_NOACTIVATE` cho `WM_MOUSEACTIVATE`.
2. **`WDA_EXCLUDEFROMCAPTURE` với cách chụp GDI của `xcap`.**
   - Nếu toolbar hoặc stack vẫn lọt vào ảnh: bật "Ẩn toolbar và stack trong lúc chụp" trong Cài đặt.
3. **Kéo viền / kéo handle để chỉnh vùng.**
   - Dùng thao tác native (`startDragging` / `startResizeDragging`).
   - Mép ngoài ~5px của lề là vùng resize native do Tauri gắn sẵn; phần lề bên trong là vùng di chuyển.
4. **Màn hình khác DPI** (100% + 150%).
   - Phần quy đổi toạ độ đã có test.
   - Phần đặt cửa sổ qua màn hình khác DPI thì chưa test được.
5. **Tooltip/menu có hiệu ứng trong suốt có lọt vào ảnh không.**
   - `xcap` chụp bằng GDI `BitBlt` không có cờ `CAPTUREBLT`.
   - Nếu thiếu: thử bật feature `wgc` của `xcap`.

## Khác với SPEC

- **Frame không bị ẩn khi dùng chế độ "ẩn UI lúc chụp".**
  - Viền nằm hoàn toàn ngoài vùng chụp, phần trong vùng trong suốt, nên frame không bao giờ dính vào ảnh.
  - Không ẩn frame thì không phải hiện lại nó, tránh làm mất focus app đang dùng.
- **Tự phát hiện thay đổi màn hình** bằng cách kiểm tra mỗi 2 giây, thay vì nghe `WM_DISPLAYCHANGE`. Chi phí CPU không đáng kể.
- **Kích thước toolbar cố định** theo mode, đặt ở `wm.rs`. Đổi nhãn nút thì cần chỉnh lại kích thước.
