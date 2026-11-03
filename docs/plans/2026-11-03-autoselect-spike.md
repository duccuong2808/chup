# Spike tự động chọn vùng (SPEC 16.6): kết quả

Ngày: 2026-11-03. Beads: chup-pwn.1 (epic chup-pwn). Công cụ đo: `tools/autoselect-spike` (Rust, `windows` 0.62.2
như app; chỉ đọc, không in tên phần tử hay tiêu đề cửa sổ).

**Trạng thái: xong.** Thiếu Edge, Excel, app treo / quyền admin / menu khi Freeze: đưa vào test khi implement
(SPEC 16.7).

## Người dùng chốt (2026-11-03)

- **Làm nguồn 3 ở Phase 2, bật mặc định, phạm vi hẹp**: chỉ dùng khi UIA không có phần tử con chứa con trỏ
  (hoặc cửa sổ không có UIA), khung cắt trong phần tử UIA sâu nhất. Tắt được trong Cài đặt (`imageEdges`).
  Lý do: khung từ ảnh phần lớn hợp lý và rẻ; giới hạn phạm vi để app có UIA tốt (Chrome, Office) không bị khung
  rác theo màu nền.
- Chuyển sang thiết kế, không đo bù.

## Môi trường

Windows 11 x64 thật (build 26300), 1 màn hình 1920×1200, 100%. Công cụ chạy như Chup: Per-Monitor DPI v2,
COM MTA, `CUIAutomation8`, CacheRequest lấy `BoundingRectangle`, `ControlType`, `Name`, lọc theo Control View.
Mỗi cửa sổ đo lưới 4×4 điểm.

## 1. Liệt kê cửa sổ (nguồn 1)

`EnumWindows` + bỏ ẩn / thu nhỏ / `DWMWA_CLOAKED` + `DWMWA_EXTENDED_FRAME_BOUNDS`, 563 HWND, 13 cửa sổ hiển thị:
lần đầu 1,1 ms, sau đó 0,1 ms. Ngân sách +10 ms (SPEC 16.5) dư nhiều.

`GetWindowRect` rộng hơn khung DWM 8–9 px mỗi cạnh ở cửa sổ có viền resize (Chrome, Explorer, Word, VS Code...),
đúng như SPEC 16.3 dự đoán; dùng khung DWM là đúng.

## 2. UI Automation (nguồn 2)

Cách đo: `ElementFromHandleBuildCache` lấy gốc, rồi đi xuống: mỗi nút lấy con bằng `FindAllBuildCache(Children)`
(1 lượt cross-process), giữ lại trong cây (cache theo cửa sổ). Lượt 1 = lần đầu Chup hỏi app; lượt 2 = chạy lại.

| App | Gốc | Điểm đầu (số lượt gọi) | Điểm sau: chậm nhất | Độ sâu | Cả cây một lượt | Ghi chú |
|---|---|---|---|---|---|---|
| Chrome, trang tin | 3 ms | 23–29 ms (15) | 36–44 ms | 7–11 | 97 ms, 839 phần tử | |
| Explorer (`CabinetWClass`) | 3 ms | 22–43 ms (4) | 38–47 ms | 2–6 | 144 ms, 156 | ô địa chỉ, danh sách file |
| Desktop (`Progman`) | 3 ms | 24 ms (3) | 0,5 ms | 1–2 | 24 ms, 25 | icon desktop |
| VS Code (Electron) | 31 ms | 36 ms (10) | 6 ms | lượt 1: 7, chỉ 1 Pane; lượt 2: 11–15 | 32 ms, 290 | accessibility bật lười |
| Teams (WebView2) | 3–13 ms | 79–119 ms (15) | 71–76 ms | 12–18 | 162 ms, 1027 | |
| Zen (Firefox) | 3–7 ms | 42–55 ms (9) | **178–217 ms** | 4–25 | 529 ms, 19774 | cây rất sâu |
| Word | 6–12 ms | lượt 1: **276 ms, timeout**; lượt 2: 31 ms | 0 ms | 3 | 260 ms, 335 | chỉ ra `Document` |
| Warp (GPU tự vẽ) | 4–9 ms | 5 ms (1) | | 0 | 10 ms, 7 | không có phần tử con |

Kết luận tạm:

1. **Đi xuống theo điểm + cache** (SPEC 16.4) là đúng hướng. Lấy cả cây một lượt quá chậm ở app lớn (Zen 529 ms,
   Word 260 ms, Teams 162 ms).
2. **Chromium/Electron bật cây accessibility lười**: lần đầu VS Code chỉ trả 1 Pane. Lượt hỏi đầu tiên nên gửi ngay
   khi overlay mở (cửa sổ dưới con trỏ), không đợi chuột di.
   Chưa kiểm chứng: Chromium có thể tự tắt accessibility sau một thời gian không có client.
3. **Timeout 200 ms (SPEC 16.4) làm Word lỗi `0x80131505` (UIA_E_TIMEOUT) ở lần hỏi đầu.** Cần timeout dài hơn
   (thread riêng nên không chặn UI), trong lúc chờ vẫn tô khung cửa sổ.
4. **Chromium có nhiều Pane trùng khít nhau, có cái rỗng**: chọn "con nhỏ nhất chứa điểm" rồi dừng sẽ lạc vào nhánh
   rỗng (bản đầu của công cụ dừng ở độ sâu 1). Thử mọi con chứa điểm thì đúng, nhưng chậm ở cây sâu (Zen 217 ms).
   Cần: ưu tiên con nhỏ nhất, chỉ thử anh em trùng khít khi nhánh đó rỗng; đặt hạn thời gian, hết hạn thì trả
   kết quả sâu nhất đã có.
5. Có phần tử nằm ngoài màn hình (Chrome có sidebar ẩn ở x âm): cắt khung theo cửa sổ và màn hình, bỏ phần tử
   không giao màn hình.
6. App tự vẽ bằng GPU (Warp) không có phần tử con: chỉ có khung cửa sổ. Đây là chỗ nguồn 3 có ích.

Chưa đo: Edge, Excel (không thấy cửa sổ lúc đo), Zalo, Slack (máy không có); app treo, app chạy quyền admin;
menu chuột phải khi Freeze; Word: vì sao hàng điểm trên cùng (vùng ribbon) cũng ra `Document`.

## 3. Khung từ ảnh (nguồn 3)

Công cụ: `tools/edge-rects` (Rust thuần, không ML). Ảnh: 9 cửa sổ chụp bằng `PrintWindow(PW_RENDERFULLCONTENT)`
(cửa sổ bị che vẫn chụp được), cắt theo khung DWM; khung UIA của cùng cửa sổ làm đáp án tham khảo. Ảnh chỉ lưu
tạm trên máy đo, không đưa vào repo. Ít hơn ~20 ảnh SPEC đề ra: 2 ảnh bị loại vì nội dung riêng tư.

Thuật toán, theo điểm con trỏ:
1. Bản đồ cạnh: cạnh ngang ở (x, y) khi kênh màu lệch nhiều nhất giữa (x, y) và (x, y-1) ≥ ngưỡng; cạnh dọc tương tự.
   Tổng tích luỹ theo hàng / cột để biết một đoạn liền cạnh bao nhiêu % trong O(1).
2. Lấy 8 hàng gần nhất phía trên / dưới có cạnh ngang liền quanh con trỏ (±12 px), 8 cột trái / phải tương tự,
   mép ảnh luôn là ứng viên; giữ hình nhỏ nhất có cả 4 cạnh liền ≥ 85%, mỗi chiều ≥ 16 px.

Đo trên lưới 40 px (1344 điểm mỗi ảnh 1920×1140). "Khớp UIA" = IoU ≥ 0,85 với một khung UIA chứa điểm.

| Ảnh | Ngưỡng 12: ra khung / khớp UIA | Ngưỡng 24: ra khung / bằng cả cửa sổ | Nhận xét khi xem ảnh |
|---|---|---|---|
| Chrome, trang tin | 89% / 31% | 94% / 0% | Khung đỏ phần lớn đúng khối nhìn thấy (ô tin, khối chuyên mục); 1 khung rác trong ảnh quảng cáo |
| Warp (UIA rỗng) | 100% / 0% | 100% / 100% | Ngưỡng 12: đúng thanh tiêu đề, sidebar, từng tab, từng khối lệnh |
| Orca (Electron) | 93% / 8% | 64% / 54% | Sidebar, tab, ô nhập đúng; dải nền diff bị coi là khung |
| VS Code | 100% / 65% | 100% / 68% | Editor khớp UIA; activity bar, danh sách file đúng |
| Explorer | 98% / 75% | 97% / 81% | |
| Teams | 75% / 66% | 85% / 100% (91% cả cửa sổ) | |
| Word | 100% / 57% | 100% / 57% | |
| Raycast, desktop | | | ngưỡng 24 gần như chỉ ra cả cửa sổ |

- "Khớp UIA" thấp **không** có nghĩa là sai: UIA chia theo Hyperlink / Text, ảnh chia theo khối nhìn thấy.
  Đánh giá đúng / sai cần xem ảnh; xem 4 ảnh (Chrome, Warp, Orca, VS Code): đa số khung hợp lý, lỗi chính là
  khung theo màu nền (dải diff, ảnh quảng cáo).
- Ngưỡng 24 bỏ lỡ viền nhạt (giao diện sáng hay dùng viền lệch ~10–25), thường trả cả cửa sổ. Ngưỡng 12 tốt hơn.
- **Thời gian**: dựng bản đồ cạnh 15–24 ms ở 1920×1140, **76 ms ở 4K**; mỗi truy vấn ~30 µs (4K ~60 µs).
  Bộ nhớ 4K 70 MB (prefix u32 cho 2 hướng); dùng u16 hoặc bitset thì giảm 2–8 lần.
  Dựng một lần lúc mở overlay, trên thread riêng; Freeze đã có sẵn ảnh, Live phải chụp thêm.

Chưa đo: màn hình 150% / 4K thật, giao diện theo theme khác, app Office ribbon mở rộng, game.
