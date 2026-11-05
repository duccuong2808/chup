# Chup

App chụp màn hình theo vùng cho Windows (macOS/Linux X11 ở Phase 3).
Rust thuần, **không WebView**: giao diện Win32 + Direct2D/DirectWrite. Lõi tách riêng, không phụ thuộc OS.

- **Đặc tả là nguồn sự thật: [SPEC.md](SPEC.md).** Mọi thay đổi hành vi phải khớp SPEC; nếu cần lệch thì cập nhật SPEC và ghi vào mục "Khác với SPEC" trong README.
- Trạng thái: Phase 1 xong và đã test trên Windows 11 thật; bước 1.5 (bỏ Tauri, UI native) xong; 2.1 Freeze (SPEC mục 14, thiết kế ở `docs/plans/2026-11-01-freeze-design.md`) xong và đã test trên VM. Kết quả test và số đo ở `docs/testing.md` (README chỉ tóm tắt).
- 2.2 Preview khi bấm thumbnail (SPEC mục 17, thiết kế `docs/plans/2026-11-01-preview-design.md`) xong, đã test trên VM.
- 2.3 OCR (SPEC mục 15, thiết kế `docs/plans/2026-11-01-ocr-design.md`) xong, đã test trên VM (epic beads `chup-2fy`).
  Tiếng Anh = Windows OCR; tiếng Việt = `chup-ocr.exe` (Tesseract link tĩnh) + dữ liệu tessdata_fast. Cả hai ghim
  trong `crates/core/src/ocr/manifest.json`; engine phát hành ở Release `ocr-engine-vN` của repo này (public).
  Bộ cài Full có sẵn cả hai; Lite / exe rời tải khi cần (chup-0oa, `docs/plans/2026-11-03-ocr-full-lite-design.md`).
- Preview chọn chữ trên ảnh kiểu Live Text (SPEC 15.2, 17.2; thiết kế `docs/plans/2026-11-01-live-text-design.md`,
  beads `chup-4fx`) xong, đã test trên VM. Chọn theo ký tự; tự nhận diện khi mở Preview (mặc định bật).
- 2.4 Tự động chọn vùng (SPEC mục 16; spike `docs/plans/2026-11-03-autoselect-spike.md`, thiết kế
  `docs/plans/2026-11-03-autoselect-design.md`, epic beads `chup-pwn`): xong, đã vào main, user dùng trên máy thật.
  Công cụ spike ở `tools/` (ngoài workspace).
- Cài đặt chia trang kiểu Windows 11 (SPEC 5.6, `chup-ck1`), trang Giới thiệu, đa ngôn ngữ vi/en (`chup-81y`), bộ cài
  Full/Lite (`chup-0oa`): đã vào main, thử trên Windows 11 x64 thật.
- Phase 3.1 macOS (SPEC 11, 12; thiết kế `docs/plans/2026-11-05-macos-design.md`, epic beads `chup-mac`): đã lên kế
  hoạch, chưa code. Việc đầu tiên: Mốc 0 tách crate `ui` (trait `Canvas`) và refactor vỏ Windows (`chup-mac.1`).

## Lệnh

```bash
cargo test --workspace                                   # test lõi (chạy được trên macOS)
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p chup --target x86_64-pc-windows-msvc -- -D warnings   # check code Windows từ macOS (không link)
# chup-ocr.exe nằm ngoài workspace (cần Tesseract: macOS `brew install tesseract pkg-config`, Windows vcpkg static)
CHUP_TESSDATA=<thư mục vie/eng.traineddata> cargo test --release --manifest-path crates/ocr-helper/Cargo.toml
cargo build --release -p chup                            # trên Windows
makensis /DLITE /DVERSION=0.2.0 /DEXE=target\release\chup.exe installer\chup.nsi   # bộ cài Lite; Full: xem README (fetch-ocr.ps1)
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

- Chuỗi người dùng thấy đi qua `core::i18n` (`t().ten_chuoi`, tham số `{}` điền bằng `fill`), có cả tiếng Việt lẫn
  tiếng Anh; không viết cứng chuỗi giao diện trong code. Log, comment code: tiếng Việt, không dịch. Lỗi kỹ thuật của crate chỉ ghi log (`%LOCALAPPDATA%\app.chup.desktop\chup.log`), không hiện cho người dùng.
- Version crate được pin bằng `=`; kiểm tra signature trên docs.rs trước khi nâng (đặc biệt `xcap`, `windows`).
- Hotkey dùng định dạng của crate `global-hotkey` (`Ctrl+Alt+A`, `F1`...).
- Việc cần làm theo dõi bằng beads (`bd`, xem khối bên dưới). Memory chia hai nơi:
  - Kiến thức về project (quyết định thiết kế, bẫy kỹ thuật, cách test) ghi bằng `bd remember`: đi theo repo qua Dolt sync, agent nào cũng thấy.
  - Thông tin riêng của máy/người dùng (VM test, đường dẫn cục bộ, ID MCP) giữ trong auto-memory của Claude Code, **không** đưa vào `bd remember`.
  - Câu "do NOT use MEMORY.md files" trong khối beads chỉ áp dụng cho kiến thức về project.
- Khối `BEGIN/END BEADS INTEGRATION` do `bd setup claude` quản lý (có hash): không sửa tay, muốn cập nhật thì chạy lại lệnh đó.


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:46cd31e7 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   bd dolt push
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->
