# Chup

App chụp màn hình theo vùng cho Windows (macOS/Linux X11 ở Phase 3).
Tauri v2 + Rust + TypeScript thuần + SCSS + Vite multi-page.

- **Đặc tả là nguồn sự thật: [SPEC.md](SPEC.md).** Mọi thay đổi hành vi phải khớp SPEC; nếu cần lệch thì cập nhật SPEC và ghi vào mục "Khác với SPEC" trong README.
- Trạng thái: Phase 1 (bước 1.0–1.4) đã code xong, **chưa test trên Windows thật**. Danh sách rủi ro cần test nằm trong README, mục "Đã kiểm chứng và chưa kiểm chứng".
- Phase tiếp theo: 2.1 Freeze (SPEC mục 14), rồi OCR (SPEC mục 15).

## Lệnh

```bash
npm install
npm run tauri dev        # chạy dev (Windows)
npm run build            # tsc --noEmit + vite build
npm run tauri build      # installer NSIS
cd src-tauri && cargo test && cargo clippy --all-targets
```

Máy dev là macOS: `cargo test` và `cargo check` chạy được. Code riêng Windows (`platform/windows.rs`) chỉ compile khi build cho target Windows.

## Kiến trúc (chi tiết: SPEC mục 6)

- **Rust giữ toàn bộ state** (`state.rs`); webview chỉ hiển thị và gọi command. Không giữ `AppState::lock()` qua `.await` hoặc khi gọi API cửa sổ.
- Luồng chính ở `actions.rs`; `commands.rs` chỉ là lớp IPC mỏng. Mọi command đều `async` (command sync tạo/điều khiển cửa sổ trên Windows dễ deadlock).
- `coords.rs` là hình học thuần, có unit test. Mọi `Rect` là **physical px trên toàn desktop**. Overlay gửi CSS px, Rust quy đổi bằng `css_to_physical`.
- `capture.rs`: `grab_region` và `persist_shot` cố ý tách riêng để Phase 2 (Freeze, OCR) dùng lại.
- Cửa sổ: `overlay-{n}` (mỗi màn hình 1 cái), `frame`, `toolbar`, `stack`, `settings`. Tạo trong `wm.rs`.
- **Toolbar và stack là cửa sổ không lấy focus**: hiện/ẩn bằng `PlatformExt::show_inactive` / `hide_inactive`, **không** dùng `show()` / `hide()` của Tauri.
- Viền frame nằm hoàn toàn ngoài region (lề `frame_margin`), nên không bao giờ dính vào ảnh.

## Thêm command mới

1. Viết hàm trong `commands.rs` và đăng ký trong `generate_handler!` ở `lib.rs`.
2. Thêm tên vào `COMMANDS` trong `build.rs`.
3. Cấp `allow-<tên-gạch-ngang>` trong `capabilities/<window>.json` cho đúng cửa sổ cần.
4. Thêm wrapper có kiểu vào `src/shared/ipc.ts`, DTO vào `src/shared/types.ts`.

## Quy ước

- Thông báo cho người dùng viết bằng tiếng Việt; comment code tiếng Việt.
- Version crate được pin bằng `=`; kiểm tra signature trên docs.rs trước khi nâng (đặc biệt `xcap`).
- Hotkey dùng định dạng của crate `global-hotkey` (`Ctrl+Alt+A`, `F1`...).
