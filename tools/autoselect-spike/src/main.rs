//! Spike SPEC 16.6 (beads chup-pwn.1): đo liệt kê cửa sổ (DWM) và UI Automation trên các app đang mở.
//!
//! Chỉ đọc, không thay đổi gì trên máy. Không in tên phần tử hay tiêu đề cửa sổ (tránh lộ nội dung màn hình),
//! chỉ in tên exe, class và loại khung.
//!
//! Chạy trên Windows: `cargo run --release -- [--grid N] [--subtree]`. Kết quả là Markdown ra stdout.

use std::time::{Duration, Instant};

use windows::{
    core::{Interface, Result, BOOL, PWSTR},
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT},
        Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS},
        System::{
            Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED},
            Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::{
            Accessibility::{
                CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest, IUIAutomationCondition,
                IUIAutomationElement, TreeScope_Children, TreeScope_Subtree, UIA_BoundingRectanglePropertyId,
                UIA_ControlTypePropertyId, UIA_NamePropertyId,
            },
            HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
            WindowsAndMessaging::{
                EnumWindows, GetClassNameW, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
                IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
            },
        },
    },
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let grid: i32 = args
        .iter()
        .position(|a| a == "--grid")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let subtree = args.iter().any(|a| a == "--subtree");

    unsafe {
        // Giống Chup: Per-Monitor v2, toạ độ DWM và UIA là physical px.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Thread UIA của Chup sẽ là MTA (SPEC 16.4).
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }

    let wins = measure_windows();
    if let Some(i) = args.iter().position(|a| a == "--dump").and_then(|i| args.get(i + 1)).and_then(|s| s.parse::<usize>().ok()) {
        // In cấu trúc cây của cửa sổ thứ i (loại + khung, không in tên).
        let u = Uia::new(10_000)?;
        let el = unsafe { u.a.ElementFromHandleBuildCache(wins[i].hwnd, &u.cr) }?;
        let mut stat = Stat::default();
        dump(&mut Node::new(el), &u, &mut stat, 0);
        return Ok(());
    }
    if let Some(dir) = args.iter().position(|a| a == "--capture").and_then(|i| args.get(i + 1)) {
        return capture_all(&wins, dir);
    }
    let u = Uia::new(200)?;
    measure_uia(&u, &wins, grid);
    if subtree {
        // Timeout dài để thấy thời gian thật của cả cây.
        measure_subtree(&Uia::new(10_000)?, &wins);
    }
    Ok(())
}

// ---------- Cửa sổ top-level ----------

struct Win {
    hwnd: HWND,
    frame: RECT,
    wrect: RECT,
    tool: bool,
}

unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> BOOL {
    let v = &mut *(l.0 as *mut Vec<HWND>);
    v.push(h);
    BOOL(1)
}

/// Đúng phần việc Chup sẽ làm trước khi hiện overlay: liệt kê theo z-order, bỏ ẩn / thu nhỏ / cloak,
/// lấy khung DWM. Trả thêm tổng số HWND.
fn top_windows() -> (Vec<Win>, usize) {
    let mut hs: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut hs as *mut _ as isize));
    }
    let total = hs.len();
    (hs.into_iter().filter_map(win_info).collect(), total)
}

fn win_info(h: HWND) -> Option<Win> {
    unsafe {
        if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            return None;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
        if cloaked != 0 {
            return None;
        }
        let mut frame = RECT::default();
        DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
        .ok()?;
        if w(&frame) < 8 || hgt(&frame) < 8 {
            return None;
        }
        let mut wrect = RECT::default();
        let _ = GetWindowRect(h, &mut wrect);
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        Some(Win { hwnd: h, frame, wrect, tool: ex & WS_EX_TOOLWINDOW.0 != 0 })
    }
}

fn measure_windows() -> Vec<Win> {
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..50 {
        let t = Instant::now();
        let r = top_windows();
        times.push(t.elapsed());
        last = Some(r);
    }
    let (wins, total) = last.unwrap();
    let first = times[0];
    times.sort();
    println!("## Liệt kê cửa sổ (EnumWindows + DWM), 50 lần\n");
    println!(
        "Lần đầu {}, trung vị {}, max {}. {} cửa sổ hiển thị / {} HWND.\n",
        ms(first),
        ms(times[times.len() / 2]),
        ms(*times.last().unwrap()),
        wins.len(),
        total
    );
    println!("| # | exe | class | tool | khung DWM (WxH) | GetWindowRect rộng hơn (trái, trên, phải, dưới) |");
    println!("|---|---|---|---|---|---|");
    for (i, x) in wins.iter().enumerate() {
        println!(
            "| {} | {} | {} | {} | {}x{} | {}, {}, {}, {} |",
            i,
            exe_name(x.hwnd),
            class_name(x.hwnd),
            if x.tool { "có" } else { "" },
            w(&x.frame),
            hgt(&x.frame),
            x.frame.left - x.wrect.left,
            x.frame.top - x.wrect.top,
            x.wrect.right - x.frame.right,
            x.wrect.bottom - x.frame.bottom,
        );
    }
    println!();
    wins
}

// ---------- UI Automation ----------

struct Uia {
    a: IUIAutomation,
    cr: IUIAutomationCacheRequest,
    view: IUIAutomationCondition,
}

impl Uia {
    fn new(timeout_ms: u32) -> Result<Self> {
        unsafe {
            let a: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
            if let Ok(a2) = a.cast::<IUIAutomation2>() {
                a2.SetConnectionTimeout(timeout_ms)?;
                a2.SetTransactionTimeout(timeout_ms)?;
            }
            // Một lượt cross-process lấy đủ 3 thuộc tính SPEC 16.4 cần.
            let cr = a.CreateCacheRequest()?;
            cr.AddProperty(UIA_BoundingRectanglePropertyId)?;
            cr.AddProperty(UIA_ControlTypePropertyId)?;
            cr.AddProperty(UIA_NamePropertyId)?;
            let view = a.ControlViewCondition()?;
            cr.SetTreeFilter(&view)?;
            Ok(Self { a, cr, view })
        }
    }
}

/// Một nút của cây khung; con được lấy lazy và giữ lại (cache theo cửa sổ như SPEC 16.4).
struct Node {
    el: IUIAutomationElement,
    rect: RECT,
    ctype: i32,
    kids: Option<Vec<Node>>,
}

impl Node {
    fn new(el: IUIAutomationElement) -> Self {
        let rect = unsafe { el.CachedBoundingRectangle() }.unwrap_or_default();
        let ctype = unsafe { el.CachedControlType() }.map(|c| c.0).unwrap_or(0);
        Self { el, rect, ctype, kids: None }
    }

    /// Lấy con (1 lượt gọi cross-process nếu chưa có). Lỗi (timeout, quyền) coi như không có con.
    fn kids(&mut self, u: &Uia, stat: &mut Stat) -> &mut Vec<Node> {
        if self.kids.is_none() {
            stat.calls += 1;
            let v = match unsafe { self.el.FindAllBuildCache(TreeScope_Children, &u.view, &u.cr) } {
                Ok(arr) => {
                    let n = unsafe { arr.Length() }.unwrap_or(0);
                    (0..n).filter_map(|i| unsafe { arr.GetElement(i) }.ok()).map(Node::new).collect()
                }
                Err(e) => {
                    stat.errors += 1;
                    stat.last_error = Some(format!("{:#010x}", e.code().0));
                    Vec::new()
                }
            };
            self.kids = Some(v);
        }
        self.kids.as_mut().unwrap()
    }
}

#[derive(Default)]
struct Stat {
    calls: u32,
    errors: u32,
    last_error: Option<String>,
}

/// Đi xuống phần tử sâu nhất chứa điểm. Thử mọi con chứa điểm (Chromium có nhiều Pane trùng khít nhau,
/// có cái rỗng), lấy nhánh sâu nhất; bằng nhau thì lấy khung nhỏ hơn. Trả (độ sâu, rect, loại).
fn descend(n: &mut Node, pt: POINT, u: &Uia, stat: &mut Stat) -> (u32, RECT, i32) {
    descend_at(n, pt, u, stat, 0)
}

fn descend_at(n: &mut Node, pt: POINT, u: &Uia, stat: &mut Stat, depth: u32) -> (u32, RECT, i32) {
    let mut best = (depth, n.rect, n.ctype);
    if depth >= 64 {
        return best;
    }
    for k in n.kids(u, stat).iter_mut() {
        if !(contains(&k.rect, pt) && w(&k.rect) > 0 && hgt(&k.rect) > 0) {
            continue;
        }
        let r = descend_at(k, pt, u, stat, depth + 1);
        let area = |r: &RECT| w(r) as i64 * hgt(r) as i64;
        if r.0 > best.0 || (r.0 == best.0 && area(&r.1) < area(&best.1)) {
            best = r;
        }
    }
    best
}

fn measure_uia(u: &Uia, wins: &[Win], grid: i32) {
    println!("## UI Automation: đi xuống từ ElementFromHandle (lưới {grid}x{grid} điểm mỗi cửa sổ)\n");
    println!("- root: ElementFromHandleBuildCache. Điểm đầu: điểm đầu tiên, chưa có cache.");
    println!("- Các điểm sau: vẫn lượt 1, dùng lại phần cây đã lấy. Lượt 2: lặp lại mọi điểm, cây đã cache hết.");
    println!("- gọi = số lần FindAllBuildCache (mỗi lần 1 lượt cross-process).\n");
    println!("| # | exe | class | root | điểm đầu (gọi) | điểm sau: trung vị / max | lượt 2 max | tổng gọi | độ sâu | số khung khác nhau | loại khung cuối | lỗi |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|");
    for (i, x) in wins.iter().enumerate() {
        if w(&x.frame) < 100 || hgt(&x.frame) < 100 {
            continue;
        }
        let exe = exe_name(x.hwnd);
        let class = class_name(x.hwnd);
        let t = Instant::now();
        let root = unsafe { u.a.ElementFromHandleBuildCache(x.hwnd, &u.cr) };
        let t_root = t.elapsed();
        let mut root = match root {
            Ok(el) => Node::new(el),
            Err(e) => {
                println!("| {i} | {exe} | {class} | {} | | | | | | | | root lỗi {:#010x} |", ms(t_root), e.code().0);
                continue;
            }
        };
        let pts: Vec<POINT> = (0..grid * grid)
            .map(|k| POINT {
                x: x.frame.left + (w(&x.frame) * (2 * (k % grid) + 1)) / (2 * grid),
                y: x.frame.top + (hgt(&x.frame) * (2 * (k / grid) + 1)) / (2 * grid),
            })
            .collect();
        let mut stat = Stat::default();
        let mut first = (Duration::ZERO, 0);
        let mut rest = Vec::new();
        let mut finals = Vec::new();
        for (k, p) in pts.iter().enumerate() {
            let before = stat.calls;
            let t = Instant::now();
            let r = descend(&mut root, *p, u, &mut stat);
            let d = t.elapsed();
            if k == 0 {
                first = (d, stat.calls - before);
            } else {
                rest.push(d);
            }
            finals.push(r);
        }
        let mut pass2 = Duration::ZERO;
        for p in &pts {
            let t = Instant::now();
            descend(&mut root, *p, u, &mut stat);
            pass2 = pass2.max(t.elapsed());
        }
        rest.sort();
        let dmin = finals.iter().map(|f| f.0).min().unwrap_or(0);
        let dmax = finals.iter().map(|f| f.0).max().unwrap_or(0);
        let mut rects: Vec<(i32, i32, i32, i32)> =
            finals.iter().map(|f| (f.1.left, f.1.top, f.1.right, f.1.bottom)).collect();
        rects.sort();
        rects.dedup();
        let mut kinds: Vec<&str> = finals.iter().map(|f| ctype_name(f.2)).collect();
        kinds.sort();
        kinds.dedup();
        println!(
            "| {i} | {exe} | {class} | {} | {} ({}) | {} / {} | {} | {} | {dmin}-{dmax} | {} | {} | {} |",
            ms(t_root),
            ms(first.0),
            first.1,
            rest.get(rest.len() / 2).map(|d| ms(*d)).unwrap_or_default(),
            rest.last().map(|d| ms(*d)).unwrap_or_default(),
            ms(pass2),
            stat.calls,
            rects.len(),
            kinds.join(", "),
            if stat.errors > 0 {
                format!("{} lần, {}", stat.errors, stat.last_error.unwrap_or_default())
            } else {
                String::new()
            },
        );
    }
    println!();
}

/// Phương án khác: lấy cả cây một lượt (TreeScope_Subtree). Đo thời gian và số phần tử.
fn measure_subtree(u: &Uia, wins: &[Win]) {
    println!("## UI Automation: lấy cả cây một lượt (BuildUpdatedCache, TreeScope_Subtree, timeout 10 s)\n");
    println!("| # | exe | class | thời gian | số phần tử | lỗi |");
    println!("|---|---|---|---|---|---|");
    let cr = unsafe {
        let cr = u.a.CreateCacheRequest().unwrap();
        cr.AddProperty(UIA_BoundingRectanglePropertyId).unwrap();
        cr.AddProperty(UIA_ControlTypePropertyId).unwrap();
        cr.AddProperty(UIA_NamePropertyId).unwrap();
        cr.SetTreeFilter(&u.view).unwrap();
        cr.SetTreeScope(TreeScope_Subtree).unwrap();
        cr
    };
    for (i, x) in wins.iter().enumerate() {
        if w(&x.frame) < 100 || hgt(&x.frame) < 100 {
            continue;
        }
        let t = Instant::now();
        let r = unsafe { u.a.ElementFromHandleBuildCache(x.hwnd, &cr) };
        let d = t.elapsed();
        let (n, err) = match r {
            Ok(el) => (count_cached(&el), String::new()),
            Err(e) => (0, format!("{:#010x}", e.code().0)),
        };
        println!("| {i} | {} | {} | {} | {n} | {err} |", exe_name(x.hwnd), class_name(x.hwnd), ms(d));
    }
    println!();
}

// ---------- Chụp ảnh + khung UIA làm đáp án tham khảo cho nguồn 3 ----------

/// Mỗi cửa sổ: `NN-exe.png` (nội dung trong khung DWM, chụp bằng PrintWindow nên cửa sổ bị che vẫn chụp được)
/// và `NN-exe.uia.txt` (mỗi dòng: loại x y w h độ_sâu, toạ độ trong ảnh, đã cắt theo khung, bỏ khung < 8 px).
fn capture_all(wins: &[Win], dir: &str) -> Result<()> {
    let _ = std::fs::create_dir_all(dir);
    let u = Uia::new(10_000)?;
    let cr = unsafe {
        let cr = u.a.CreateCacheRequest()?;
        cr.AddProperty(UIA_BoundingRectanglePropertyId)?;
        cr.AddProperty(UIA_ControlTypePropertyId)?;
        cr.SetTreeFilter(&u.view)?;
        cr.SetTreeScope(TreeScope_Subtree)?;
        cr
    };
    for (i, x) in wins.iter().enumerate() {
        if w(&x.frame) < 100 || hgt(&x.frame) < 100 {
            continue;
        }
        let exe = exe_name(x.hwnd).to_lowercase().trim_end_matches(".exe").replace(' ', "_");
        let name = format!("{dir}/{i:02}-{exe}");
        match grab(x) {
            Some(img) => {
                if let Err(e) = img.save(format!("{name}.png")) {
                    eprintln!("{name}: lưu ảnh lỗi: {e}");
                }
            }
            None => eprintln!("{name}: PrintWindow lỗi"),
        }
        match unsafe { u.a.ElementFromHandleBuildCache(x.hwnd, &cr) } {
            Ok(el) => {
                let mut out = String::new();
                walk_cached(&el, 0, &x.frame, &mut out);
                let _ = std::fs::write(format!("{name}.uia.txt"), out);
            }
            Err(e) => eprintln!("{name}: UIA lỗi {:#010x}", e.code().0),
        }
        println!("{name}");
    }
    Ok(())
}

fn grab(x: &Win) -> Option<image::RgbaImage> {
    use windows::Win32::{
        Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
            BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        },
        Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS},
        UI::WindowsAndMessaging::PW_RENDERFULLCONTENT,
    };
    let (ww, wh) = (w(&x.wrect), hgt(&x.wrect));
    unsafe {
        let dc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: ww,
                biHeight: -wh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(dc, bmp.into());
        let ok = PrintWindow(x.hwnd, dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
        let px = std::slice::from_raw_parts(bits as *const u8, (ww * wh * 4) as usize).to_vec();
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        if !ok {
            return None;
        }
        // Cắt về khung DWM (bỏ viền resize vô hình), BGRA -> RGBA.
        let (ox, oy) = (x.frame.left - x.wrect.left, x.frame.top - x.wrect.top);
        let (fw, fh) = (w(&x.frame) as u32, hgt(&x.frame) as u32);
        Some(image::RgbaImage::from_fn(fw, fh, |cx, cy| {
            let i = (((oy + cy as i32) * ww + ox + cx as i32) * 4) as usize;
            image::Rgba([px[i + 2], px[i + 1], px[i], 255])
        }))
    }
}

fn walk_cached(el: &IUIAutomationElement, depth: u32, frame: &RECT, out: &mut String) {
    use std::fmt::Write;
    let r = unsafe { el.CachedBoundingRectangle() }.unwrap_or_default();
    let ct = unsafe { el.CachedControlType() }.map(|c| c.0).unwrap_or(0);
    let (l, t) = (r.left.max(frame.left), r.top.max(frame.top));
    let (rr, b) = (r.right.min(frame.right), r.bottom.min(frame.bottom));
    if rr - l >= 8 && b - t >= 8 {
        let _ = writeln!(out, "{} {} {} {} {} {depth}", ctype_name(ct), l - frame.left, t - frame.top, rr - l, b - t);
    }
    let Ok(arr) = (unsafe { el.GetCachedChildren() }) else { return };
    let n = unsafe { arr.Length() }.unwrap_or(0);
    for c in (0..n).filter_map(|i| unsafe { arr.GetElement(i) }.ok()) {
        walk_cached(&c, depth + 1, frame, out);
    }
}

fn dump(n: &mut Node, u: &Uia, stat: &mut Stat, depth: usize) {
    println!(
        "{}{} [{}, {}, {}x{}]",
        "  ".repeat(depth),
        ctype_name(n.ctype),
        n.rect.left,
        n.rect.top,
        w(&n.rect),
        hgt(&n.rect)
    );
    if depth >= 12 {
        return;
    }
    let kids = n.kids(u, stat);
    let shown = kids.len().min(8);
    for k in kids.iter_mut().take(shown) {
        dump(k, u, stat, depth + 1);
    }
    if kids.len() > shown {
        println!("{}... còn {} con", "  ".repeat(depth + 1), kids.len() - shown);
    }
}

fn count_cached(el: &IUIAutomationElement) -> usize {
    let Ok(arr) = (unsafe { el.GetCachedChildren() }) else { return 1 };
    let n = unsafe { arr.Length() }.unwrap_or(0);
    1 + (0..n).filter_map(|i| unsafe { arr.GetElement(i) }.ok()).map(|c| count_cached(&c)).sum::<usize>()
}

// ---------- Tiện ích ----------

fn w(r: &RECT) -> i32 {
    r.right - r.left
}

fn hgt(r: &RECT) -> i32 {
    r.bottom - r.top
}

fn contains(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn exe_name(h: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return format!("pid {pid} (không mở được)");
        };
        let mut buf = [0u16; 520];
        let mut n = buf.len() as u32;
        let r = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n);
        let _ = CloseHandle(p);
        if r.is_err() {
            return format!("pid {pid}");
        }
        let s = String::from_utf16_lossy(&buf[..n as usize]);
        s.rsplit('\\').next().unwrap_or(&s).to_string()
    }
}

fn class_name(h: HWND) -> String {
    let mut b = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut b) };
    String::from_utf16_lossy(&b[..n.max(0) as usize])
}

fn ctype_name(id: i32) -> &'static str {
    match id {
        50000 => "Button",
        50001 => "Calendar",
        50002 => "CheckBox",
        50003 => "ComboBox",
        50004 => "Edit",
        50005 => "Hyperlink",
        50006 => "Image",
        50007 => "ListItem",
        50008 => "List",
        50009 => "Menu",
        50010 => "MenuBar",
        50011 => "MenuItem",
        50012 => "ProgressBar",
        50013 => "RadioButton",
        50014 => "ScrollBar",
        50015 => "Slider",
        50016 => "Spinner",
        50017 => "StatusBar",
        50018 => "Tab",
        50019 => "TabItem",
        50020 => "Text",
        50021 => "ToolBar",
        50022 => "ToolTip",
        50023 => "Tree",
        50024 => "TreeItem",
        50025 => "Custom",
        50026 => "Group",
        50027 => "Thumb",
        50028 => "DataGrid",
        50029 => "DataItem",
        50030 => "Document",
        50031 => "SplitButton",
        50032 => "Window",
        50033 => "Pane",
        50034 => "Header",
        50035 => "HeaderItem",
        50036 => "Table",
        50037 => "TitleBar",
        50038 => "Separator",
        50039 => "SemanticZoom",
        50040 => "AppBar",
        _ => "?",
    }
}
