# Thiết kế Phase 2.2: Preview khi bấm thumbnail

Ngày: 2026-11-01. Beads: chup-b0m. Đặc tả: SPEC mục 17 (mới); đổi Q5, 5.4, 15.2, 6.10.

## Mục đích (người dùng chốt)

Bấm thumbnail thì hiện Preview có opacity. Preview giúp người chụp nhìn ảnh vừa chụp để dựa vào đó làm việc khác,
nên **chỗ và cỡ mở** là quan trọng nhất: có mặc định và chỉnh được. OCR (copy text) sẽ đặt trên Preview.
Các chi tiết còn lại do Claude chốt (người dùng dặn làm luôn, không hỏi).

## Chỗ và cỡ

- Cài đặt "Chỗ mở":
  - **Chỗ và cỡ lần trước** (mặc định). Kéo, đổi cỡ, zoom là nhớ khung (`preview.last`). Lần sau ảnh vừa khung
    (tối đa 1:1), bám cạnh khung gần cạnh màn hình hơn. Chưa có khung, hoặc màn hình của khung đã bị rút:
    mở như "Cạnh stack".
  - **Cạnh stack**: sát mép trong của stack, căn đáy work area. Stack ẩn thì sát cạnh màn hình.
  - **Giữa màn hình**.
- Cài đặt "Cỡ khi mở": ảnh tối đa 30/50/70/90% work area (mặc định 50%), không phóng quá 1:1.
- Lý do chọn mặc định: mở sát stack (chỗ vừa bấm) và chỉ chiếm nửa màn hình thì không che chỗ đang làm.
  Người dùng chỉnh một lần (kéo ra góc, đổi cỡ), các lần sau tự đúng chỗ.
- Chỉnh trực tiếp: kéo để di chuyển; kéo mép/góc để đổi cỡ (giữ tỉ lệ, mép đối diện đứng yên); lăn chuột để
  zoom quanh con trỏ; `+`/`-`/`0`.

## Còn lại

- Opacity: Cài đặt 100–20% (mặc định 100%); `Ctrl` + lăn chuột ±10% cho cửa sổ đang mở.
- Luôn nằm trên: Cài đặt (mặc định bật, vì mục đích là nhìn ảnh trong lúc làm app khác); nút ghim cho cửa sổ đang mở.
- Toolbar khi hover: [Ghim] [Copy] [Folder] [✕]; OCR thêm [T].
- 1 cửa sổ dùng chung; lấy focus khi mở; có trên taskbar/Alt+Tab; không loại khỏi ảnh chụp.

## Core

- `config.preview { alwaysOnTop, opacity, position, size, last }`: field mới có default, vẫn version 2.
- `core::preview`: `open_center`, `open_beside_stack`, `open_in_box`, `zoom_at`, `resize_to`, `edge_hit`,
  `drag_edge`, opacity. Unit test.
- `Model::shot_open(id)` -> `Effect::OpenPreview(path)`; `Model::set_preview_box(rect)` lưu khung khi `remember`.

## Platform (`preview.rs`)

- Layered window, opacity = `SourceConstantAlpha`. 2 surface: `base` (ảnh đã scale, vẽ lại khi đổi cỡ) và
  `out` (`base` + toolbar + nhãn). 1 `Canvas` dùng chung, `BindDC` sang surface cần vẽ để bitmap ảnh tạo 1 lần.
- Cờ "đang mở" là `Cell` ngoài `RefCell`, và `out` được lấy ra khỏi state trước `UpdateLayeredWindow`:
  Windows gửi ngược WM_WINDOWPOSCHANGED/WM_DPICHANGED vào wndproc trong lúc đẩy ảnh (bài học từ overlay).
- Di chuyển bằng `WM_NCLBUTTONDOWN(HTCAPTION)`; vẽ lại không do đổi cỡ thì đọc lại vị trí thật trước.
- Đổi cỡ bằng SetCapture tự xử lý (không dùng vòng lặp sizing của Windows) để giữ tỉ lệ và kẹp work area.
- Lưu khung: sau khi kéo/đổi cỡ xong, và 700 ms sau lần zoom cuối (hoặc lúc đóng).
- Đóng là bỏ ảnh, surface, bitmap; cửa sổ ẩn chứ không huỷ (kể cả WM_CLOSE).

## Test

- Core (macOS): chỗ mở 3 kiểu, bám góc khung, ảnh tí hon, zoom giữ điểm neo và kẹp, kéo mép giữ tỉ lệ,
  edge_hit, opacity, config, `shot_open`, `set_preview_box`.
- VM: SPEC 17.6.
