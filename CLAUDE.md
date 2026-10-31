# Chup

App chụp màn hình theo vùng cho Windows (macOS/Linux X11 ở Phase 3).
Rust thuần, **không WebView**: giao diện Win32 + Direct2D/DirectWrite. Lõi tách riêng, không phụ thuộc OS.

- **Đặc tả là nguồn sự thật: [SPEC.md](SPEC.md).** Mọi thay đổi hành vi phải khớp SPEC; nếu cần lệch thì cập nhật SPEC và ghi vào mục "Khác với SPEC" trong README.
- Trạng thái: Phase 1 xong và đã test trên Windows 11 thật; bước 1.5 (bỏ Tauri, UI native) xong. Kết quả test ở README, mục "Đã kiểm chứng và chưa kiểm chứng".
- Phase tiếp theo: 2.1 Freeze (SPEC mục 14), rồi OCR (SPEC mục 15), rồi tự động chọn vùng (SPEC mục 16).

## Lệnh

```bash
cargo test --workspace                                   # test lõi (chạy được trên macOS)
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p chup --target x86_64-pc-windows-msvc -- -D warnings   # check code Windows từ macOS (không link)
cargo build --release -p chup                            # trên Windows
makensis /DVERSION=0.2.0 /DEXE=target\release\chup.exe installer\chup.nsi   # installer
```

Máy dev là macOS: check/clippy cho target Windows chạy được (cần `rustup target add x86_64-pc-windows-msvc`), nhưng không link được. Bản chạy thật lấy từ CI (artifact `chup-windows-<sha>` có cả `chup.exe` và installer).

## Kiến trúc (chi tiết: SPEC mục 6)

- `crates/core`: `Model` (state machine) nhận sự kiện, trả về `Vec<Effect>`. Không biết cửa sổ là gì, nên mọi logic đều unit test được. Thêm hành vi mới thì thêm vào đây trước, kèm test.
- `crates/app/src/platform/windows/`: vỏ Win32. `app.rs` đổi `Action` -> gọi `Model` -> áp `Effect` lên cửa sổ.
- **Một UI thread.** Cửa sổ, tray, hotkey, worker **không gọi thẳng vào App**: chúng `win::emit(Action)`; App xử lý khi nhận `WM_APP_ACTION`.
- State của mỗi cửa sổ nằm trong `RefCell`; **không giữ borrow khi gọi API Win32** (API có thể gửi message ngược lại vào wndproc).
- Mọi `Rect` là **physical px trên toàn desktop**. Kích thước giao diện khai báo bằng DIP rồi nhân scale của màn hình.
- `capture.rs`: `grab_region` và `persist_shot` cố ý tách riêng để Phase 2 (Freeze, OCR) dùng lại.
- Cửa sổ phụ (overlay, frame, toolbar, stack) là **layered window** vẽ vào `gfx::Surface` (DIB premultiplied BGRA) rồi `UpdateLayeredWindowIndirect`.
  - Overlay và frame vẽ bằng thao tác pixel trực tiếp (rẻ ở 4K); toolbar, stack, chữ dùng Direct2D **software** + DirectWrite.
  - Surface chỉ tồn tại khi cửa sổ đang hiện; ẩn là giải phóng (overlay 4K ~33 MB).
  - Pixel alpha 0 của layered window không nhận chuột: vùng cần nhận chuột mà "trong suốt" thì dùng alpha 1/255.
- Toolbar, stack, frame có `WS_EX_NOACTIVATE` và trả `MA_NOACTIVATE`: không bao giờ lấy focus. Overlay thì lấy focus để nhận Esc.
- Viền frame nằm hoàn toàn ngoài region (lề `frame_margin`), nên không bao giờ dính vào ảnh.

## Quy ước

- Thông báo cho người dùng viết bằng tiếng Việt; comment code tiếng Việt. Lỗi kỹ thuật của crate chỉ ghi log (`%LOCALAPPDATA%\app.chup.desktop\chup.log`), không hiện cho người dùng.
- Version crate được pin bằng `=`; kiểm tra signature trên docs.rs trước khi nâng (đặc biệt `xcap`, `windows`).
- Hotkey dùng định dạng của crate `global-hotkey` (`Ctrl+Alt+A`, `F1`...).
