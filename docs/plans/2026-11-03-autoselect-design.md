# Thiết kế Phase 2.4: Tự động chọn vùng

Ngày: 2026-11-03. Beads: epic `chup-pwn`. Đặc tả gốc: [SPEC mục 16](../../SPEC.md#16-phase-2--tự-động-chọn-vùng).
Spike: [2026-11-03-autoselect-spike.md](2026-11-03-autoselect-spike.md). Tài liệu này chốt cách làm; chỗ lệch SPEC
ghi ở mục 6 và sẽ cập nhật vào SPEC.

## Người dùng chốt (2026-11-03)

- Nguồn 3 (khung từ ảnh): **làm, bật mặc định, phạm vi hẹp**: chỉ khi UIA không có phần tử con chứa con trỏ
  (hoặc cửa sổ không có UIA), khung cắt trong phần tử UIA sâu nhất. Tắt được trong Cài đặt.
- Cuộn chuột / `↑` `↓` đổi cấp: **giữ số cấp đã lên khi di chuột trong cùng cửa sổ**; sang cửa sổ khác thì về khung
  trong cùng.
- Kiểu tô: **bên trong khung sáng (không phủ tối) + viền nét đứt màu accent** + nhãn `W × H · Loại`.
- Freeze: đã có vùng chọn thì rê ra **ngoài** vùng vẫn tô khung; click ngoài = chọn khung mới. Trong vùng, trên
  handle, trên toolbar thì không tô.
- Live mode dùng được nguồn 3: overlay được loại khỏi ảnh chụp (`WDA_EXCLUDEFROMCAPTURE`), ảnh màn hình chụp ngầm
  sau khi overlay hiện. App chia sẻ màn hình sẽ không thấy overlay. Máy không hỗ trợ exclusion: Live tắt nguồn 3.

## Hướng đã chọn

- **Logic thuần nằm ở core, test được**: chuỗi khung (cắt, lọc, đổi cấp), luật click / kéo, tìm cửa sổ dưới con trỏ
  trong danh sách đã chụp, nhận khung từ ảnh (port từ `tools/edge-rects`).
- **`Model` chỉ thêm config.** Khung đang tô, cấp đang chọn là state của overlay, giống lý do của Freeze (vùng đang
  chọn là state của overlay). `Model` không biết đến UIA.
- **Một worker riêng (thread COM MTA)** làm UIA và khung từ ảnh. Overlay gửi yêu cầu, worker `emit` kết quả về App;
  không gọi thẳng vào App hay overlay (quy tắc một UI thread).
- Danh sách cửa sổ chụp **đồng bộ trên UI thread** ngay trước khi hiện overlay (spike: 0,1–1,1 ms), nên khung cửa sổ
  tô được ngay ở lần di chuột đầu tiên, không chờ worker.

## 1. Core

### `core::autoselect` (module mới, thuần)

```rust
pub enum FrameKind { Window, Button, Edit, Text, Image, Link, List, ListItem, Menu, MenuItem, Tab, ToolBar,
                     Document, Table, Group, Pane, Other, Detected /* từ ảnh */ }
impl FrameKind {
    pub fn from_uia(control_type: i32) -> Self;   // mã UIA_*ControlTypeId
    pub fn label(self) -> &'static str;           // "Cửa sổ", "Nút", "Ô nhập", "Chữ", "Ảnh", "Liên kết",
                                                  // "Danh sách", "Mục", "Menu", "Mục menu", "Tab", "Thanh công cụ",
                                                  // "Tài liệu", "Bảng", "Nhóm", "Vùng", "Vùng", "Vùng ảnh"
}
pub struct Frame { pub rect: Rect, pub kind: FrameKind }

/// Cửa sổ top-level đã chụp lúc mở overlay, theo z-order (trên cùng trước).
pub struct WinFrame { pub id: isize /* HWND */, pub rect: Rect /* khung DWM */ }
pub fn window_at(wins: &[WinFrame], p: (i32, i32)) -> Option<&WinFrame>;

/// Chuỗi khung từ ngoài vào trong cho điểm p: cắt theo `clip` (màn hình ∩ cửa sổ), bỏ khung không chứa p,
/// nhỏ hơn 8×8, hoặc trùng khung ngay ngoài nó (chênh ≤ 2 px mỗi cạnh). Phần tử đầu luôn là cửa sổ.
pub fn normalize(raw: &[Frame], p: (i32, i32), clip: Rect) -> Vec<Frame>;

/// Chọn cấp. `up` = số cấp đã lên tính từ khung trong cùng.
pub struct Picker { window: Option<isize>, up: usize }
impl Picker {
    pub fn pick<'a>(&mut self, window: isize, chain: &'a [Frame]) -> Option<&'a Frame>; // đổi cửa sổ: up = 0
    pub fn wheel(&mut self, steps: i32, chain_len: usize);   // dương = lên cha; kẹp trong [0, len-1]
}

/// Nhả chuột cách chỗ nhấn không quá 4 px (theo cả 2 trục) là click (SPEC 16.2).
pub fn is_click(down: (i32, i32), up: (i32, i32)) -> bool;
```

### `core::edges` (module mới, thuần): nguồn 3

Port thuật toán của `tools/edge-rects` (spike mục 3), đổi:

- Đầu vào là pixel BGRA (cả `RgbaImage` của Freeze lẫn ảnh chụp Live), ngưỡng 12 (spike: 24 bỏ lỡ viền nhạt).
- Tổng tích luỹ kiểu `u16` (rộng / cao ≤ 65 535): 4K khoảng 33 MB thay vì 70 MB.
- `EdgeMap::build(px, w, h, threshold) -> EdgeMap` và `EdgeMap::query(p, within: Rect) -> Option<Rect>`:
  ứng viên cạnh chỉ lấy trong `within` (phần tử UIA sâu nhất), mép `within` luôn là ứng viên; trả `None` nếu
  khung tìm được bằng `within` (không thêm cấp vô ích).

### Config

```rust
pub struct AutoDetectConfig { pub enabled: bool, pub image_edges: bool }   // mặc định true, true
// Config: pub auto_detect: AutoDetectConfig   (JSON: "autoDetect": { "enabled", "imageEdges" })
```

Field mới có serde default nên **không tăng version** (như `preview`, `ocr`).

## 2. Worker (`platform/windows/autoselect.rs`, mới)

Một thread cố định, tạo lúc app khởi động, ngủ khi không có việc (`recv` chặn). COM MTA, một `IUIAutomation`
(`CUIAutomation8`), một `CacheRequest` lấy `BoundingRectangle`, `ControlType` qua Control View (như spike).

```rust
enum Req {
    /// Phiên mới (overlay vừa hiện): xoá cache, nhớ ảnh Freeze (nếu có) để dựng EdgeMap khi cần.
    Begin { gen: u32, frozen: Vec<(Rect /*màn hình*/, Arc<RgbaImage>)>, live_edges: bool },
    Query { gen: u32, p: (i32, i32), win: WinFrame, mon: Rect },
    End { gen: u32 },
}
// Kết quả: emit(Action::AutoFrames { gen, p, window: isize, chain: Vec<Frame> })
```

- **Chỉ chạy yêu cầu mới nhất**: trước mỗi việc, rút hết hàng đợi, giữ `Query` cuối (spike: truy vấn đã cache
  ~0 ms, nên không debounce cố định).
- **Số thế hệ `gen`** trên mọi yêu cầu và kết quả; overlay bỏ kết quả khác `gen` hiện tại (tránh lỗi loại
  `chup-71a.12` mục 1).
- **Cây cache theo cửa sổ** trong một phiên (struct `Node` của spike: con lấy lười, giữ lại); `End` xoá hết.
- **Đi xuống**: chọn con nhỏ nhất chứa điểm; nhánh đó không có con chứa điểm thì thử các anh em trùng khít khung
  (Chromium có Pane trùng khít, có cái rỗng). Hạn 80 ms mỗi truy vấn: hết hạn trả chuỗi sâu nhất đã có, lần sau đi
  tiếp từ cache.
- **Timeout UIA 1 s** (`IUIAutomation2` Connection/Transaction). Cửa sổ timeout lần đầu: đánh dấu "không UIA" trong
  phiên đó, chỉ dùng khung cửa sổ (+ nguồn 3).
- **Hỏi sớm**: `Begin` kèm luôn một `Query` tại vị trí con trỏ, để Chromium / Electron kịp bật cây accessibility
  (spike: lần hỏi đầu VS Code chỉ trả 1 Pane).
- **Nguồn 3**: khi phần tử sâu nhất không có con chứa điểm (hoặc cửa sổ không UIA) và `imageEdges` bật:
  - Freeze: dựng `EdgeMap` từ ảnh đóng băng của màn hình đó (lần đầu cần, ~20 ms 1080p, ~76 ms 4K).
  - Live (`live_edges` = exclusion OK): lần đầu cần thì chụp cả màn hình bằng `capture` (overlay đã bị loại khỏi
    ảnh), rồi dựng `EdgeMap`.
  - Giữ `EdgeMap` theo màn hình tới `End`; `End` giải phóng (4K ~33 MB + ảnh Live ~33 MB).
- Lỗi UIA / chụp chỉ ghi log; kết quả lúc đó là chuỗi chỉ có cửa sổ.

## 3. Overlay

### Danh sách cửa sổ

`App` chụp `Vec<WinFrame>` (EnumWindows theo z-order; bỏ ẩn, thu nhỏ, cloak, cửa sổ của chính Chup) rồi đưa cho
overlay khi hiện:

- Live: trong `show_overlays`, trước khi hiện overlay.
- Freeze: trong `start_freeze_grab`, **trước khi chụp và trước khi overlay lấy focus** (overlay lấy focus làm menu
  chuột phải đóng lại; chụp sớm thì menu còn trong danh sách, chọn được khung menu).

### Rê chuột (khi `autoDetect.enabled`, không giữ `Shift`, không đang kéo)

1. `window_at` → cửa sổ dưới con trỏ. Chưa có kết quả worker cho cửa sổ này thì tô ngay khung cửa sổ (≤ 16 ms).
2. Gửi `Query` cho worker. Kết quả về (`Action::AutoFrames`, App chuyển cho overlay chứa điểm): `normalize` theo
   màn hình của overlay, `Picker::pick`, tô lại nếu khung đổi.
3. Kết quả cho điểm cũ vẫn dùng nếu điểm hiện tại còn nằm trong khung trong cùng của nó; không thì bỏ, chờ kết quả mới.

Freeze: chỉ tô khi con trỏ ở **ngoài** vùng chọn, không trên handle, không trên toolbar.

### Vẽ

- Bên trong khung: Live như vùng đang kéo (`HOLE_PX`, không phủ tối); Freeze vẽ ảnh gốc không làm tối.
- Viền nét đứt 2 px, màu `Theme::current().accent`, nằm ngoài khung (như `outline_outside`). Thêm vào `Surface` một
  hàm `outline_dashed_outside(rect, t, dash, px)`.
- Nhãn `W × H · Nút` dùng `PillPainter` của nhãn kích thước, nới bộ đệm lên ~260 DIP; vị trí như `place_label`.
- Dirty = khung cũ ∪ khung mới (nới cỡ viền) ∪ nhãn cũ ∪ nhãn mới.

### Chuột và phím

- **Live** thêm trạng thái "đã nhấn, chưa kéo": `WM_LBUTTONDOWN` chỉ ghi điểm nhấn; di quá 4 px mới thành kéo tự do
  (vẽ như hiện tại, bỏ khung tô); nhả khi chưa quá 4 px:
  - có khung đang tô → `Action::RegionSelected(khung)` (dùng lại luồng Phase 1);
  - không có → huỷ như hiện tại.
  Thêm xử lý `WM_CAPTURECHANGED` như Freeze (mất capture giữa chừng thì chốt ở vị trí hiện tại, không bấm).
- **Freeze**: `frozen_down` ở ngoài vùng ghi điểm nhấn như `Drag::New`; nhả trong 4 px mà có khung tô thì `sel` = khung
  (hiện handle + toolbar như vừa kéo xong); "Chụp text" thì OCR luôn.
- **Cuộn chuột / `↑` `↓`**: `WM_MOUSEWHEEL` và phím chỉ tới overlay đang có focus, nên overlay `emit(Action::AutoLevel(steps))`,
  App chuyển cho overlay đang chứa con trỏ → `Picker::wheel`, vẽ lại.
- **`Shift`**: `WM_KEYDOWN/KEYUP` của `VK_SHIFT` (và kiểm `GetKeyState` khi di chuột) → bỏ / hiện lại khung tô.
- `Esc`, chuột phải: như cũ.

### Ảnh Freeze dùng chung

`Frozen.img` đổi sang `Arc<RgbaImage>` để worker đọc được; `take_frozen` (crop khi Lưu / Copy / OCR) lấy `Arc` ra,
worker nhả bản của nó khi `End`.

## 4. Luồng trong `App`

```
hotkey Chọn vùng ─> Model.start_selection ─> ShowOverlays
  App: wins = snapshot_windows(); overlays.show(.., wins); worker.Begin{gen, frozen: [], live_edges}; Query(con trỏ)
F1 ─> FreezeGrab
  App: wins = snapshot_windows() (trước khi chụp) ─> thread chụp ─> ShowFreezeOverlays
  App: overlays.show_frozen(.., wins); worker.Begin{gen, frozen: ảnh Arc}; Query(con trỏ)
overlay rê chuột ─> worker.Query ─> emit AutoFrames{gen,..} ─> App ─> overlay chứa điểm
cuộn / ↑↓ ─> emit AutoLevel ─> App ─> overlay chứa con trỏ
HideOverlays ─> worker.End{gen}
```

- `sync_overlays` đặt `WDA_EXCLUDEFROMCAPTURE` cho mọi overlay (cả overlay tạo lại khi đổi màn hình).
  `live_edges = capture_exclusion_ok && config.auto_detect.image_edges`.
- `gen` tăng mỗi lần hiện overlay; App giữ `gen` hiện tại, bỏ `AutoFrames` cũ.

## 5. Cài đặt

Mục "Chọn vùng" thêm 2 ô:

- `[x] Tự nhận khung cửa sổ và phần tử khi rê chuột`
- `[x] Nhận khung từ ảnh (cho app không khai báo khung)`: mờ đi khi ô trên tắt.

## 6. Khác với SPEC 16 (sẽ sửa SPEC)

| SPEC | Thiết kế | Lý do |
|---|---|---|
| 16.4: timeout UIA ~200 ms | 1 s, timeout thì cửa sổ đó chỉ dùng khung cửa sổ trong phiên | Spike: Word lần đầu 276 ms, lỗi timeout |
| 16.4: debounce ~30 ms | Chỉ chạy yêu cầu mới nhất, không chờ | Truy vấn đã cache ~0 ms; chờ cố định chỉ thêm trễ |
| 16.4: lấy danh sách cửa sổ **và UIA** lúc chụp Freeze | Chỉ danh sách cửa sổ; UIA vẫn lười | Không biết trước điểm cần; menu đóng khi overlay lấy focus nên chỉ chọn được khung cả menu, không chọn từng mục |
| 16.4: `autoDetect.imageEdges: false`, gộp vào config v2 | `imageEdges: true`, phạm vi hẹp; field có serde default, không tăng version | User chốt sau spike; v2 đã phát hành |
| 16.2: tô viền nét đứt | Thêm: bên trong khung không phủ tối | User chốt |
| 16.2: chưa nói cấp có giữ khi di chuột | Giữ trong cùng cửa sổ | User chốt |
| 16.3: khung nhỏ nhất chứa con trỏ | Thêm: thử anh em trùng khít khi nhánh rỗng; hạn 80 ms mỗi truy vấn | Spike: Chromium có Pane trùng khít rỗng; Firefox cây sâu |
| (không có) | Overlay bị loại khỏi ảnh chụp | Live chụp ngầm cho nguồn 3 |

## 7. Test

### Unit test trong core

- `FrameKind::from_uia` và `label` cho các mã thường gặp; mã lạ → `Other`.
- `window_at`: theo z-order (cửa sổ trên che cửa sổ dưới), điểm ngoài mọi cửa sổ.
- `normalize`: cắt theo màn hình (cửa sổ vắt 2 màn hình), bỏ khung < 8×8, bỏ khung trùng cha ≤ 2 px, bỏ khung
  không chứa điểm, phần tử ngoài màn hình (toạ độ âm).
- `Picker`: lên / xuống bị kẹp, giữ `up` khi cùng cửa sổ và chuỗi ngắn đi (kẹp lại), đổi cửa sổ thì về 0.
- `is_click`: 0, 4, 5 px theo từng trục.
- `edges`: ảnh tổng hợp có khung lồng nhau, viền 1 px và 2 px, viền nhạt (lệch 12–20), chữ giả (nhiều nét ngắn)
  không thành khung, `within` cắt đúng, khung bằng `within` trả `None`; ảnh 3840×2160 dựng < 150 ms (release).
- Config: thiếu `autoDetect` → mặc định bật cả 2; giữ giá trị người dùng.

### Check

`cargo clippy -p chup --target x86_64-pc-windows-msvc -- -D warnings` trên WSL; `cargo test -p chup-core` trên
Windows host (WSL thiếu thư viện cho `xcap`).

### Test trên máy Windows thật (SPEC 16.7 + bổ sung)

- Explorer, Chrome, Edge, VS Code, Word, Excel: khung cửa sổ khớp mép, không dính bóng / viền vô hình.
- Chrome: thanh địa chỉ, ảnh trong trang, khối bài viết; cuộn lên / xuống cấp; giữ cấp khi rê trong cửa sổ.
- Click chọn khung rồi chụp: ảnh đúng kích thước khung, đúng pixel ở 100% và 150%.
- `Shift`: không tô; kéo tay như cũ. Nhấn rồi di < 4 px vẫn là click.
- Freeze: menu chuột phải + F1 → chọn được khung menu; đã có vùng chọn, click ngoài chọn khung mới; "Chụp text".
- Warp (UIA rỗng): khung từ ảnh; Live và Freeze.
- App treo (giả lập bằng app test dừng message loop) và app chạy quyền admin: overlay không đứng, vẫn có khung cửa sổ.
- Cửa sổ vắt 2 màn hình (khi có máy 2 màn hình).
- Kiểm chứng: xcap không chụp overlay sau khi đặt `WDA_EXCLUDEFROMCAPTURE` (như frame / toolbar Phase 1).
- Log thời gian: hiện overlay thêm ≤ 10 ms; khung cửa sổ ≤ 16 ms; khung UIA lần đầu ≤ 100 ms (trừ app chậm đã biết);
  RAM trước / trong / sau phiên (EdgeMap được giải phóng).
