# Thiết kế Phase 2.1: Freeze mode

Ngày: 2026-11-01. Đặc tả gốc: [SPEC mục 14](../../SPEC.md#14-phase-2--freeze-mode). Tài liệu này chốt cách làm; hành vi vẫn theo SPEC.

## Hướng đã chọn

Mở rộng overlay của Phase 1 thêm chế độ Freeze (SPEC 14.4: "dùng lại overlay window của Phase 1").

- Hình học chọn vùng/chỉnh vùng nằm trong module thuần `core::selection`, test được trên macOS.
- `Model` chỉ biết có phiên freeze hay không; ảnh và vùng đang chỉnh là state của overlay.
- Không tạo cửa sổ mới: toolbar vẽ ngay trong DIB của overlay (SPEC 14.3).

Trong overlay: kéo trong vùng là di chuyển, kéo ở ngoài là chọn vùng mới.

## 1. Core

### `core::selection` (module mới, thuần)

```rust
pub enum Grip { Move, N, NE, E, SE, S, SW, W, NW }

/// Điểm nằm trên handle nào, trong vùng (Move) hay ngoài (None).
pub fn hit(rect: Rect, p: (i32, i32), handle_px: i32) -> Option<Grip>;

/// Vùng mới khi kéo `grip` từ `from` tới `to`.
pub fn apply(rect: Rect, grip: Grip, from: (i32, i32), to: (i32, i32), bounds: Rect) -> Rect;
```

- Move: giữ nguyên kích thước, kẹp trong `bounds`.
- Resize: kéo qua cạnh đối diện thì vùng lật chiều; tối thiểu 1×1; không vượt `bounds`.
- Toạ độ là physical px theo overlay của màn hình đó (giống overlay hiện tại).

### Hàm pixel dùng chung (trong core để test được)

- `dim_px(rgba) -> bgra`: đổi RGBA sang BGRA premultiplied, alpha 255, nhân 0,6 (tối 40%).
- `crop(img, rect_desktop, mon) -> RgbaImage`: đổi toạ độ desktop sang toạ độ ảnh của màn hình, cắt theo màn hình.

### State trong `Model`

```
freeze: Option<FreezeSession>   // chỉ đánh dấu đang có phiên
freeze_timer: bool              // đang đếm ngược 3 giây
```

| Sự kiện | Effect |
|---|---|
| `start_freeze()` | Đóng frame Live nếu đang mở, rồi `FreezeGrab` |
| `start_freeze_delayed()` | `FreezeTimer(3s)` (tray đổi icon); gọi lại thì đếm lại từ đầu |
| F1 lúc đang đếm ngược | Huỷ hẹn giờ, `FreezeGrab` ngay |
| `freeze_grabbed(Ok)` | `ShowOverlays` ở chế độ Freeze |
| `freeze_grabbed(Err)` (mọi màn hình hỏng) | Toast "Không chụp được màn hình", kết thúc phiên |
| `freeze_confirm(rect, Save)` | `HideOverlays`, `FreezePersist(rect)` |
| `freeze_confirm(rect, Copy)` | `HideOverlays`, `FreezeCopy(rect)` |
| `freeze_cancel()` | `HideOverlays` |

- Đang freeze thì bỏ qua hotkey Chọn vùng, Chụp và Freeze.
- Freeze **không** cập nhật `last_region` (SPEC 14.2: không có "chụp lặp lại cùng vùng").
- `FreezeConfirm` lúc này chỉ có `Save | Copy`; `Ocr` thêm khi làm OCR.

### Config v2

- `CONFIG_VERSION = 2`, thêm `hotkeys.freeze` (serde default `"F1"`).
- Migrate v1 → v2 chỉ điền default; file v2 giữ nguyên giá trị người dùng.
- Đăng ký F1 lỗi (app khác giữ): báo qua `hotkey_errors` như hotkey khác.

## 2. Overlay (`overlay.rs`)

### Hiện

- `show(focus, mode)` với `mode = Live | Freeze(RgbaImage)`: overlay giữ ảnh của màn hình mình.
- Màn hình nào chụp hỏng (các màn khác được): overlay đó hiện lớp tối kèm dòng "Không chụp được màn hình này".

### Vẽ (thao tác pixel trên phần dirty)

- Mỗi pixel lấy từ ảnh gốc qua `dim_px` nếu nằm ngoài vùng chọn, giữ nguyên nếu ở trong. Alpha luôn 255 nên mọi chỗ nhận chuột.
- Viền trắng 1 px ngoài vùng; 8 handle ô vuông 8 DIP, ruột trắng viền tối.
- Nhãn `W × H` khi kéo và dòng gợi ý lúc đầu: dùng lại `PillPainter` và sprite có sẵn.
- Dirty khi kéo/di chuyển/resize = vùng cũ ∪ vùng mới (nới cỡ handle) ∪ nhãn ∪ toolbar cũ/mới.
- Log thời gian từng bước như `show()` hiện tại. Dự kiến tô cả màn 4K 15–25 ms ở release.

### Toolbar `[✓ Lưu] [Copy] [✕]`

- Vẽ bằng Direct2D vào một sprite nhỏ rồi blend vào DIB; chỗ đặt tính bằng `place_toolbar` của core (dưới vùng → trên vùng → bên trong).
- Phần vẽ nút trong `toolbar.rs` tách thành hàm dùng chung, nhận danh sách nút (sau này thêm [T] không phải sửa layout).
- Hover đổi màu nút: chỉ vẽ lại sprite toolbar.

### Chuột và phím

- Con trỏ: chữ thập ngoài vùng, 4 chiều trong vùng, resize trên handle, mũi tên thường trên toolbar.
- Chuột trái xuống: trên nút → bấm nút; trên handle/trong vùng → chỉnh vùng (`core::selection`); ngoài → chọn vùng mới. Chọn trên màn hình khác thì vùng chuyển sang màn đó.
- Double-click trong vùng hoặc `Enter` → Lưu. `Esc` hoặc chuột phải → huỷ.
- Overlay emit `Action::FreezeConfirm { rect, action }` (physical px toàn desktop) hoặc `Action::FreezeCancel`.

### Giải phóng

Hết phiên freeze là bỏ DIB và ảnh ngay (không đợi lúc rảnh như Live): ở 4K khoảng 66 MB mỗi màn hình.

## 3. Luồng trong `App`

```
F1 / tray ──> Model.start_freeze ──> FreezeGrab
  thread: chụp mọi màn hình song song (chưa hiện cửa sổ nào)
  ──> Action::FreezeGrabbed(Vec<(monitor key, Result<RgbaImage>)>)
  ──> Model.freeze_grabbed ──> ShowOverlays(Freeze) ──> overlay.show(.., Freeze(img))
chọn/chỉnh vùng trong overlay ──> Action::FreezeConfirm { rect, Save | Copy }
  Save: lấy ảnh khỏi overlay ──> thread: crop + persist_shot ──> Action::Persisted (đường có sẵn: stack, toast)
  Copy: lấy ảnh khỏi overlay ──> thread: crop + clipboard::copy_image(&img, None)
        ──> Action::FreezeCopied(Result) ──> toast "Đã copy ảnh" hoặc toast lỗi
```

- Copy chỉ ghi DIBV5, không mã hoá PNG.
- `rect` sau khi cắt theo màn hình bị rỗng: coi như huỷ.
- Cửa sổ của app (stack, toolbar) không lọt vào ảnh nhờ capture exclusion của Phase 1.

### Tray và hẹn giờ

- Menu tray thêm "Chụp đóng băng" và "Chụp đóng băng sau 3 giây".
- Hẹn giờ: `SetTimer` 3 s trên cửa sổ chính; trong lúc chờ tray đổi sang icon đồng hồ cát, màn hình không hiện gì (SPEC 14.4). Hết giờ emit `Action::FreezeNow`.
- Settings thêm dòng hotkey "Chụp đóng băng".

## 4. Test

### Unit test trong core (TDD, chạy trên macOS)

- `selection::hit`: từng handle trong 8 handle, trong vùng, ngoài vùng, handle chồng nhau khi vùng rất nhỏ.
- `selection::apply`: move bị kẹp ở 4 cạnh; resize từng handle; lật chiều; tối thiểu 1×1; không vượt màn hình.
- `Model`: `start_freeze` đóng frame Live; bỏ qua hotkey khi đang freeze; grabbed Ok/Err; confirm Save/Copy; cancel; không đổi `last_region`; hẹn giờ và F1 lúc đếm ngược.
- Config: v1 migrate lên v2 có `freeze = "F1"`; v2 giữ giá trị người dùng.
- `crop`: đúng pixel khi màn hình ở toạ độ âm và scale 150%.
- `dim_px`: giá trị pixel trong và ngoài vùng.

### Check từ macOS

`cargo clippy -p chup --target x86_64-pc-windows-msvc -- -D warnings`.

### Test trên VM (SPEC 14.7; VM 1 màn hình, 200%)

- Menu chuột phải trong Explorer, nhấn F1: ảnh có menu.
- Tooltip: với F1 và với hẹn giờ 3 giây.
- Lưu: có file, vào stack. Copy: không có file, dán vào Paint thấy ảnh.
- Di chuyển, resize đủ 8 handle, double-click, Enter, Esc, chuột phải.
- Ảnh hiện nét ở 200% (1:1); so pixel ảnh crop với ảnh chụp gốc.
- Đo thời gian từ nhấn hotkey đến lúc ảnh hiện; RAM trước, trong, sau phiên.

### Chưa test được trên VM

2 màn hình, màn 4K, 150%, Chrome/YouTube: ghi vào README mục "Chưa test".
