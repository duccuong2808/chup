# Chup

Chụp màn hình theo vùng cho Windows. Ảnh vừa chụp nằm thành stack ở cạnh màn hình
và kéo thả thẳng được sang app khác. Đặc tả đầy đủ ở [SPEC.md](SPEC.md).

- **Trạng thái**: Phase 1 (v1, Windows) xong, cộng bước 1.5: bỏ Tauri/WebView2, giao diện
  vẽ native bằng Win32 + Direct2D/DirectWrite. Phase 2.1 (Freeze: chụp đóng băng menu, tooltip) xong.
  Phase 2.2 (Preview khi bấm thumbnail) xong, đã test trên VM.
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
| Dùng ảnh trong stack | Kéo thumbnail thả vào app khác; click để mở Preview; hover để copy / mở folder / bỏ khỏi stack |
| Thu gọn stack | Hover stack → nút `›`; click tab nhỏ ở mép màn hình để mở lại |
| Chụp đóng băng (menu, tooltip) | `F1`, hoặc menu tray "Chụp đóng băng" / "… sau 3 giây". Màn hình đóng băng ngay; kéo chọn vùng, chỉnh bằng 8 ô vuông, kéo trong vùng để di chuyển |
| Preview | Kéo để di chuyển, kéo mép/góc để đổi cỡ, lăn chuột để zoom, `Ctrl` + lăn chuột đổi opacity. Hover: ghim (luôn nằm trên), copy, folder, đóng. `Esc` đóng. Lần sau mở đúng chỗ và cỡ cũ (đổi trong Cài đặt) |
| Lưu / Copy / huỷ (Freeze) | `Enter`, double-click trong vùng hoặc nút **Lưu** / nút **Copy** (chỉ clipboard, không lưu file) / `Esc` hoặc click phải |

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
- Đỉnh RAM khi chụp vùng 2812×1350: ~54 MB. Rảnh 15 s thì working set về dưới 2 MB, nhưng
  commit (bộ nhớ đã cấp, phần lớn nằm ngoài RAM) giữ ở ~34–37 MB vì heap của Windows không trả
  hết vùng đã dùng cho ảnh lớn.
- Bấm Chụp → thumbnail, vùng 1920×1080 (đo 2026-10-31, sau khi bỏ arboard): chụp 58–149 ms +
  lưu 215–233 ms = **280–302 ms**, có 1 lần 382 ms. Mã hoá PNG chiếm ~200 ms. Ngân sách SPEC là
  300 ms. VM này là Windows **ARM64**, `chup.exe` (x64) chạy qua giả lập Prism, nên trên máy x64
  thật sẽ nhanh hơn; cần đo lại ở đó.

Freeze (đo 2026-11-01, cùng VM ARM64 chạy x64 qua giả lập, 1 màn hình 3600×2024):
- Nhấn F1 → ảnh đóng băng hiện: **194–224 ms** (ngân sách SPEC 250 ms, tính cho 2 màn hình gồm
  1 màn 4K trên máy thật). Gồm chụp 134–165 ms + hiện overlay 52–64 ms (tô ảnh ~32 ms).
  Lần đầu sau khi khởi động 346 ms (Direct2D còn nguội: vẽ gợi ý + toolbar 49 ms).
- Lưu vùng 717×844: 40–93 ms.
- RAM (private): 25 MB trước, 81 MB trong phiên (ảnh + DIB cỡ màn hình), về 25 MB ngay khi đóng.

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
- Chế độ Mờ: click xuyên qua, gõ phím và cuộn ở app bên dưới bình thường; `Ctrl+Alt+S` chụp
  đúng vùng (ảnh là nội dung sau khi cuộn, không dính viền) và app bên dưới vẫn giữ focus.
- Chụp 7 lần liên tiếp: stack còn 5 ảnh mới nhất, ổ đĩa có đủ 7 file.
- Kéo thả thumbnail (chỉ đưa `CF_HDROP`, hiệu ứng copy): Explorer (copy file, file gốc còn
  nguyên, cùng hash), VS Code (mở Image Preview), Chrome (vùng thả và `<input type=file>`, nhận
  đúng tên, kích thước, `image/png`).
- Không cướp focus: sau khi chụp, app đang dùng vẫn giữ focus.
- Single instance: chạy lần 2 thì mở Cài đặt.
- Cài đặt: ghi phím tắt, báo lỗi tổ hợp đã bị app khác chiếm, `Esc` huỷ ghi phím.
- Freeze (bản `0aa7947`):
  - Menu chuột phải trên Desktop + `F1`: ảnh đóng băng có menu, crop ra đúng pixel và nét ở 200%.
  - Tooltip taskbar ("Microsoft Edge") + hẹn giờ 3 giây từ tray: ảnh có tooltip; icon tray mờ
    trong lúc chờ; overlay vẫn lấy được focus (`Esc` đóng được).
  - Mục tray "Chụp đóng băng": menu tray không lọt vào ảnh.
  - Kéo chọn vùng, resize bằng handle, kéo trong vùng để di chuyển (chỗ cũ vẽ lại sạch);
    hover nút toolbar đổi màu, con trỏ bàn tay.
  - Lưu bằng `Enter` và double-click: có file, vào stack. Copy: không tạo file, clipboard có ảnh,
    có toast. `Esc` và click phải: huỷ, không tạo file.
  - Stack không lọt vào ảnh đóng băng. Toolbar chế độ thường vẫn như cũ sau khi tách hàm vẽ.
  - Config v1 tự lên v2 với `freeze = "F1"`; ô hotkey "Chụp đóng băng" trong Cài đặt, đặt trùng
    thì báo lỗi.

- Preview (bản `26fd2fe`, sửa góc bo tròn ở bản sau):
  - Click thumbnail: mở sát stack, căn đáy, ảnh 1687×1125 hiện ở 1446×964 (50% màn hình), có trên taskbar.
    Từ click tới lúc hiện ~100 ms (giải mã PNG 75–90 ms, vẽ 7–15 ms).
  - Hover hiện toolbar; lăn chuột zoom quanh con trỏ (nhãn %); Ctrl + lăn chuột đổi opacity (nhãn "Opacity 50%").
  - Kéo di chuyển; kéo mép đổi cỡ giữ tỉ lệ, mép đối diện đứng yên, con trỏ mũi tên 2 chiều.
  - Đóng (`Esc`, ✕, Alt+F4) rồi mở lại: đúng chỗ và cỡ lần trước (`preview.last` được ghi). "Giữa màn hình" đúng.
  - Ghim: tắt thì Explorer che được; bật thì vẫn nằm trên khi click Explorer.
  - Copy: toast "Đã copy ảnh". Folder: Explorer chọn sẵn file. Cài đặt đổi opacity: áp ngay cho Preview đang mở.
  - RAM: đang mở 121 MB; đóng rồi trim còn 2 MB working set.
  - Lỗi tìm thấy và đã sửa: góc bo tròn (pixel alpha 0) không nhận chuột nên không kéo góc được.

- Clipboard (khi chụp và nút Copy, ảnh 2193×1350): có đủ `PNG` (đúng byte của file), CF_DIBV5,
  và `Bitmap`/`DIB` do Windows tự suy ra. Chuẩn bị ~16–20 ms, giữ clipboard ~9–10 ms.
- Nút mở folder (Explorer chọn sẵn file), nút bỏ khỏi stack, Xoá hết, `Ctrl+Alt+S` chụp lại vùng cuối.
- Menu tray (Mở folder ảnh, Cài đặt…); bật/tắt chạy cùng Windows (khoá `HKCU\…\Run`);
  đổi folder lưu qua hộp chọn folder, ảnh sau đó lưu đúng folder mới.

Lưu ý khi test trên VM VMPal: agent của VMPal đồng bộ clipboard sang máy Mac. Gặp ảnh lớn
trong clipboard, nó mất kết nối vài giây rồi ghi text cũ đè lên. Lỗi này có cả khi PowerShell
tự đặt ảnh, nên không do Chup. Muốn kiểm tra clipboard thì phải đọc ngay trong ~2 giây đầu.

**Chưa test:**
1. Nhiều màn hình, nhất là màn hình khác DPI (100% + 150%), và rút màn hình phụ khi frame đang
   ở trên đó. Phần quy đổi toạ độ có unit test. VM hiện chỉ có 1 màn hình.
2. Tooltip/menu có hiệu ứng trong suốt có lọt vào ảnh không (`xcap` chụp bằng GDI `BitBlt`
   không có cờ `CAPTUREBLT`). Menu Desktop và tooltip taskbar của Windows 11 thì có (test Freeze);
   menu của app khác (Explorer, Chrome) chưa thử.
3. Kéo thả vào Zalo PC, Slack, Word.
4. Freeze với 2 màn hình, màn 4K, màn 150%; video YouTube đang chạy (VM không có Chrome).
   Thời gian Freeze trên máy x64 thật.
5. Preview: nhiều màn hình (mở trên màn của stack, kéo sang màn khác DPI), ảnh 4K, cảm giác mượt khi kéo mép ở 4K.

**Vấn đề đang theo dõi:** kéo thumbnail đôi khi không bắt đầu kéo. Trên VM, nhấn chuột ngay khi
vừa rê vào thumbnail thì không kéo được (0/4); dừng ≥ 0,15 s thì 9/9. Có 1 lần lỗi dù đã dừng
0,5 s, chưa rõ nguyên nhân (Chup hay cách VMPal giả lập chuột).

## Khác với SPEC

- **Frame không bị ẩn khi dùng chế độ "ẩn UI lúc chụp".**
  - Viền nằm hoàn toàn ngoài vùng chụp, phần trong vùng trong suốt, nên frame không bao giờ dính vào ảnh.
  - Không ẩn frame thì không phải hiện lại nó, tránh làm mất focus app đang dùng.
- **Chưa có tooltip** cho nút trên toolbar và stack.
- **Trim khi rảnh**: sau 15 s không thao tác, app giải phóng bộ đệm và trả working set
  cho hệ điều hành. Lần chọn vùng đầu tiên sau đó chậm hơn (đo được ~113 ms so với ~35 ms).
- **Cài đặt bố cục 2 cột**.
- **Freeze, mục tray "Chụp đóng băng"** chờ ~350 ms cho menu tray tắt hẳn rồi mới chụp (F1 chụp ngay).
- **Freeze, lúc đếm ngược hẹn giờ**: icon tray mờ đi kèm tooltip, thay vì một icon riêng.
