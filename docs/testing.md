# Kiểm chứng và số đo

Nhật ký test thủ công và số đo hiệu năng của Chup. Tóm tắt ở [README](../README.md).

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

Rảnh, đo lại 2026-11-03 trên máy x64 thật (Intel Core Ultra 7 255H, Windows 11, 125%), bản có Preview, OCR,
tự chọn vùng, đa ngôn ngữ (chup-3p3): vừa mở private working set 1,9 MB, commit 2,5 MB, 6 luồng; sau 15 s tự trả RAM
còn **0,1–0,2 MB**, 3 luồng; CPU **0%** (2 phút không tăng ms nào, cả khởi động tốn 47 ms). Lúc rảnh không có timer
định kỳ, chỉ luồng OCR thức dậy 5 phút/lần.

Lúc đang dùng, đo 2026-11-03 cùng máy, 2 lượt user dùng thật 5 phút (chụp gần cả màn hình, Freeze, Preview kéo bôi
đen chữ / zoom / đổi cỡ, OCR tiếng Việt, kéo thumbnail ra app khác):
- Chụp 19–48 ms, lưu 27–97 ms; F1 tới lúc ảnh Freeze hiện 47 ms; Preview giải mã ảnh 1919×1199 9–17 ms.
- `chup.exe`: private working set đỉnh ~75 MB (Preview + OCR ảnh cỡ màn hình); rảnh 15 s là về 0,1 MB, commit còn
  19–27 MB (bộ đệm trong Direct2D / DirectWrite / UI Automation và heap, gần như không chiếm RAM thật).
- CPU `chup.exe` 4 s trong 5 phút dùng liên tục. Tốn nhất ở UI thread: vẽ lại Preview khi kéo bôi đen chữ
  (2,6 ms/lần, chỉ khi vùng chọn sang ký tự khác), overlay khi rê chuột (1,5 ms/lần).
- `chup-ocr.exe`: ~40 MB khi đang giữ model; tắt sau **2 phút** không dùng (trước là 5 phút; lúc dùng các lần OCR cách
  nhau < 1 phút).

Công cụ đo có sẵn trong `chup.log`: dòng `mem <mốc>: ws …, commit …, đỉnh ws …` ở các mốc (mở overlay, chụp và lưu
xong, hiện / xong / huỷ Freeze, mở / đóng Preview, OCR xong, trước / sau khi trả RAM), và mỗi lần trả RAM thì bảng
`ui: <cửa sổ> <message>: N lần, tổng thời gian` cho các message xử lý ≥ 0,5 ms (thời gian thật, gồm cả vòng lặp
modal như kéo thả hay kéo cửa sổ, nên không phải toàn bộ là CPU).

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
- Chế độ Mờ: click xuyên qua, gõ phím và cuộn ở app bên dưới bình thường; `Ctrl+Alt+S` (nay là `Ctrl+Alt+X`) chụp
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

- OCR (bản `7475765`; VM ARM64 chạy bản x64 qua giả lập, nên số đo chậm hơn máy thật):
  - CI: vcpkg build Tesseract tĩnh (lần đầu ~30 phút, có cache ~20 s); `chup-ocr.exe` 6,4 MB chỉ phụ thuộc DLL
    hệ thống; test pipe trên runner: CER 0,85% (np200), 0,21% (np100).
  - Bộ cài: tích ô "OCR tiếng Việt" thì `--install-ocr` tải qua WinHTTP, SHA-256 khớp, config chuyển sang
    tiếng Việt. Không tích thì không có thư mục `ocr`. Gỡ cài đặt tắt cả hai exe, xoá exe và thư mục `ocr`.
  - Tiếng Việt (Notepad, 200%): sai ~0,7% ký tự ở câu thường ("Chup" thành "Chụp"); dòng chỉ gồm dấu
    đứng rời (ắ ằ ẳ ẵ ặ) thì đọc sai. Kết quả NFC, clipboard CF_UNICODETEXT xuống dòng CRLF.
    Job đầu 1,9–2,2 s (khởi động `chup-ocr.exe` ~1,7 s lần đầu, ~0,2 s khi file đã nằm trong cache), sau đó ~0,4 s.
    `chup-ocr.exe` 43–49 MB.
  - Tiếng Anh (Windows OCR): 40–60 ms, nhận thẳng `SoftwareBitmap` Gray8 (không phải chuyển Bgra8).
  - Nút [T] trên toolbar, "Chữ" khi Freeze, [T] trên Preview đều copy và hiện toast. Vùng không có chữ: toast
    "Không tìm thấy chữ". Hotkey "Chụp text" (gán Ctrl+Alt+T trong Cài đặt): overlay Freeze có gợi ý, thả chuột là
    OCR luôn, không hiện toolbar.
  - `chup-ocr.exe` tự tắt đúng 5 phút sau job cuối (log "tắt chup-ocr.exe sau 5 phút không dùng"); `chup.exe` còn 8 MB.
  - Chọn tiếng Việt khi chưa có gói rồi bấm [T]: hỏi tải (ghi 4,4 MB), có thanh tiến trình, tải xong tự OCR
    ảnh vừa yêu cầu. Chặn mạng (tường lửa): báo "Không tải được gói. Kiểm tra mạng rồi thử lại.", bấm Thử lại
    sau khi mở mạng thì tải được. Xoá gói khi `chup-ocr.exe` đang chạy: xoá được (tắt exe phụ trước).
  - Lỗi tìm thấy và đã sửa: chữ tiếng Việt trong bộ cài hỏng (script thiếu BOM UTF-8); "Xoá gói" lỗi
    os error 32 vì `chup-ocr.exe` giữ thư mục gói, rồi tải lại cũng lỗi; instance thứ hai ghi đè `chup.log`.

- Chọn chữ trên ảnh, Live Text (bản `dda136b`, Notepad 200%):
  - Tiếng Việt: kéo giữa từ ra đúng ký tự ("m chụp màn hì"), kéo nhiều dòng (xuôi và ngược) ghép hàng bằng
    xuống dòng, không thừa dấu cách đầu; zoom ×1,95 rồi chọn vẫn khớp chữ. Tiếng Anh (Windows OCR, ranh giới
    ký tự ước lượng): "n fox jumps o" khớp chỗ kéo.
  - Bấm đúp chọn từ, bấm 3 lần chọn cả dòng, `Shift`+bấm nới vùng chọn, `Ctrl+A` chọn hết, `Ctrl+C` và menu
    chuột phải "Copy chữ" copy đúng; con trỏ I-beam trên chữ. `Esc` bỏ vùng chọn, lần nữa mới đóng.
    `Alt`+kéo trên chữ và kéo chỗ trống đều di chuyển cửa sổ (vùng chọn bỏ đi).
  - Job nền mở Preview: tiếng Anh 20–60 ms, tiếng Việt ảnh 2728×1631 ~2,7 s. Bấm [T] lúc job nền đang chạy:
    nút quay "bận", chỉ một job, xong là copy. Bấm [T] khi job nền đã xong: copy ngay (cache).
  - Tắt "Tự nhận diện chữ khi mở Preview": Preview không có chữ (con trỏ mũi tên); bấm [T] thì copy cả khối và
    sau đó chọn được chữ.
  - Không tìm thấy lỗi mới.

- Clipboard (khi chụp và nút Copy, ảnh 2193×1350): có đủ `PNG` (đúng byte của file), CF_DIBV5,
  và `Bitmap`/`DIB` do Windows tự suy ra. Chuẩn bị ~16–20 ms, giữ clipboard ~9–10 ms.
- Nút mở folder (Explorer chọn sẵn file), nút bỏ khỏi stack, Xoá hết, `Ctrl+Alt+S` (nay là `Ctrl+Alt+X`) chụp lại vùng cuối.
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
6. OCR: bấm Huỷ giữa lúc đang tải gói (mạng VM tải 4,4 MB quá nhanh, không kịp bấm);
   máy x64 thật (VM là ARM64 chạy giả lập); user Windows có tên tiếng Việt (đường dẫn có dấu).
7. **Tự động chọn vùng (Phase 2.4)**: user dùng trên Windows 11 x64 thật 2026-11-03, ổn. `↑` `↓` lúc đầu không tới
   được các khung thấy khi rê chuột; đã sửa (chup-4q9) và thử lại ổn. Chưa thử: app quyền admin, app treo, cửa sổ vắt
   2 màn hình (checklist thiết kế mục 7).
8. **Cài đặt chia trang** (chup-ck1): mới chụp màn hình 5 trang trên Windows 11 x64 thật (125%), đổi trang không còn
   sót control. Chưa thử tay: Tab/phím mũi tên, ghi phím tắt, danh sách xổ của combobox, đổi DPI khi kéo sang màn khác.
9. **Bộ cài Full / Lite** (chup-0oa): `chup.exe --install-ocr` trên Windows 11 x64 thật tải engine từ Release
   `ocr-engine-v1` (WinHTTP theo redirect 302 của GitHub), đúng SHA-256, `chup-ocr.exe --version` chạy được từ
   `%LOCALAPPDATA%` (Symantec không chặn). CI (2897593) build được 2 bộ cài: Full 4,9 MB có `chup-ocr.exe`,
   `NOTICE-ocr.txt` và 2 file dữ liệu vào `ocr\vi-1\` (đúng SHA-256 manifest, xem bằng 7-Zip), Lite 0,69 MB chỉ có
   `chup.exe`. Chưa chạy thật: cài Lite có tick OCR, Full trên máy không mạng.
10. **Phím tắt** (chup-289): `Ctrl+Alt+X` đăng ký được trên máy thật (`Ctrl+Alt+S/D/T` có app khác giữ); trang Phím tắt
    có ô bật/tắt F1 (đã chụp). Menu tray hai phần ("Chụp" / "Tuỳ chọn") hiện đúng (user xem tay 2026-11-03).

**Vấn đề đang theo dõi:** kéo thumbnail đôi khi không bắt đầu kéo. *(2026-11-03, chup-t8m: đã sửa: lần nhấn thứ hai
nhanh tới dưới dạng nhấp đúp mà stack không xử lý; thêm chặn lần nhấn treo. User thử trên máy thật: kéo được mọi lần.)* Trên VM, nhấn chuột ngay khi
vừa rê vào thumbnail thì không kéo được (0/4); dừng ≥ 0,15 s thì 9/9. Có 1 lần lỗi dù đã dừng
0,5 s, chưa rõ nguyên nhân (Chup hay cách VMPal giả lập chuột).

## Ngôn ngữ giao diện

Đã chụp kiểm tra 6 trang Cài đặt bằng tiếng Anh và đổi ngôn ngữ khi đang mở Cài đặt (Windows 11 x64, 125%).
Chưa xem bằng mắt: toolbar, stack, overlay, menu tray, hộp thoại tải OCR và bộ cài bằng tiếng Anh.
