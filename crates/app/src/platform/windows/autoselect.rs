//! Tự động chọn vùng (SPEC 16), phần Windows: chụp danh sách cửa sổ và worker UI Automation + khung từ ảnh.
//! Thiết kế: docs/plans/2026-11-03-autoselect-design.md mục 2. Spike: 2026-11-03-autoselect-spike.md.
//!
//! - `snapshot_windows` chạy trên UI thread ngay trước khi hiện overlay (spike: 0,1–1 ms).
//! - Worker là 1 thread COM MTA cố định, ngủ khi không có việc. Chỉ chạy yêu cầu `Query` mới nhất; kết quả gửi về
//!   App bằng `emit(Action::AutoFrames)`, kèm số thế hệ `gen` để App bỏ kết quả của phiên cũ.
//! - Lỗi UIA / chụp chỉ ghi log; khi đó chuỗi khung chỉ có khung cửa sổ.

use std::{
    collections::HashMap,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
    time::{Duration, Instant},
};

use chup_core::{
    autoselect::{Frame, FrameKind, WinFrame},
    capture,
    coords::Rect,
    edges::{EdgeMap, THRESHOLD},
};
use image::RgbaImage;
use windows::{
    core::{Interface, BOOL},
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS},
        System::{
            Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED},
            Threading::GetCurrentProcessId,
        },
        UI::{
            Accessibility::{
                CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest, IUIAutomationCondition,
                IUIAutomationElement, TreeScope_Children, UIA_BoundingRectanglePropertyId, UIA_ControlTypePropertyId,
            },
            WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindowVisible},
        },
    },
};

use super::{app::Action, win::emit};

// ---------- Danh sách cửa sổ ----------

unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> BOOL {
    let v = &mut *(l.0 as *mut Vec<HWND>);
    v.push(h);
    BOOL(1)
}

/// Cửa sổ top-level theo z-order (trên cùng trước): bỏ ẩn, thu nhỏ, cloak (desktop ảo khác...), cửa sổ của Chup.
/// Khung lấy bằng `DWMWA_EXTENDED_FRAME_BOUNDS`: không dính viền resize vô hình (SPEC 16.3).
pub fn snapshot_windows() -> Vec<WinFrame> {
    let t0 = Instant::now();
    let mut hs: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut hs as *mut _ as isize));
    }
    let me = unsafe { GetCurrentProcessId() };
    let out: Vec<WinFrame> = hs
        .into_iter()
        .filter_map(|h| unsafe {
            if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if pid == me {
                return None;
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
            if cloaked != 0 {
                return None;
            }
            let mut r = RECT::default();
            DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut _, size_of::<RECT>() as u32)
                .ok()?;
            let rect = rect_of(&r)?;
            Some(WinFrame { id: h.0 as isize, rect })
        })
        .collect();
    log::debug!("autoselect: {} cửa sổ, {:?}", out.len(), t0.elapsed());
    out
}

fn rect_of(r: &RECT) -> Option<Rect> {
    (r.right > r.left && r.bottom > r.top).then(|| Rect::new(r.left, r.top, (r.right - r.left) as u32, (r.bottom - r.top) as u32))
}

// ---------- Worker ----------

enum Req {
    Begin { gen: u32, frozen: Vec<(Rect, Arc<RgbaImage>)>, image_edges: bool, live_capture: bool },
    Query { gen: u32, p: (i32, i32), win: WinFrame, mon: Rect },
    End { gen: u32 },
}

pub struct Worker {
    tx: Sender<Req>,
}

impl Worker {
    pub fn start() -> Worker {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("autoselect".into()).spawn(move || run(rx)).expect("tạo thread autoselect");
        Worker { tx }
    }

    /// Phiên mới (overlay vừa hiện). `frozen`: ảnh Freeze theo màn hình (rỗng ở Live).
    /// `live_capture`: Live được chụp màn hình để nhận khung từ ảnh (overlay đã bị loại khỏi ảnh chụp).
    pub fn begin(&self, gen: u32, frozen: Vec<(Rect, Arc<RgbaImage>)>, image_edges: bool, live_capture: bool) {
        let _ = self.tx.send(Req::Begin { gen, frozen, image_edges, live_capture });
    }

    /// Chuỗi khung cho điểm `p` (desktop) trong cửa sổ `win`, cắt theo màn hình `mon` khi nhận khung từ ảnh.
    pub fn query(&self, gen: u32, p: (i32, i32), win: WinFrame, mon: Rect) {
        let _ = self.tx.send(Req::Query { gen, p, win, mon });
    }

    /// Hết phiên: bỏ cache cây UIA, ảnh và bản đồ cạnh.
    pub fn end(&self, gen: u32) {
        let _ = self.tx.send(Req::End { gen });
    }
}

/// Hạn cho một truy vấn: hết hạn thì trả chuỗi sâu nhất đã có, lần sau đi tiếp từ cache (spike: Firefox cây sâu).
const QUERY_BUDGET: Duration = Duration::from_millis(80);
/// Timeout UIA. Spike: Word lần đầu 276 ms, 200 ms (SPEC cũ) làm lỗi timeout.
const UIA_TIMEOUT_MS: u32 = 1_000;
/// `UIA_E_TIMEOUT`.
const E_TIMEOUT: i32 = 0x8013_1505u32 as i32;

fn run(rx: Receiver<Req>) {
    if let Err(e) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
        log::warn!("autoselect: CoInitializeEx: {e}");
    }
    let mut uia: Option<Uia> = None;
    let mut sess: Option<Session> = None;
    while let Ok(first) = rx.recv() {
        let mut reqs = vec![first];
        reqs.extend(rx.try_iter());
        // Chỉ chạy Query cuối cùng (thao tác chuột trước đó đã cũ); Begin / End chạy đủ, theo thứ tự.
        let last_q = reqs.iter().rposition(|r| matches!(r, Req::Query { .. }));
        for (i, r) in reqs.into_iter().enumerate() {
            match r {
                Req::Begin { gen, frozen, image_edges, live_capture } => {
                    if uia.is_none() {
                        uia = Uia::new().map_err(|e| log::warn!("autoselect: không tạo được UIA: {e}")).ok();
                    }
                    sess = Some(Session::new(gen, frozen, image_edges, live_capture));
                }
                Req::End { gen } => {
                    if sess.as_ref().is_some_and(|s| s.gen == gen) {
                        sess = None;
                    }
                }
                Req::Query { gen, p, win, mon } if Some(i) == last_q => {
                    let Some(s) = sess.as_mut().filter(|s| s.gen == gen) else { continue };
                    let t0 = Instant::now();
                    let chain = s.chain(uia.as_ref(), p, win, mon);
                    log::trace!("autoselect: {} khung, {:?}", chain.len(), t0.elapsed());
                    emit(Action::AutoFrames { gen, p, window: win.id, chain });
                }
                Req::Query { .. } => {}
            }
        }
    }
}

struct Uia {
    a: IUIAutomation,
    cr: IUIAutomationCacheRequest,
    view: IUIAutomationCondition,
}

impl Uia {
    fn new() -> windows::core::Result<Self> {
        unsafe {
            let a: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
            if let Ok(a2) = a.cast::<IUIAutomation2>() {
                a2.SetConnectionTimeout(UIA_TIMEOUT_MS)?;
                a2.SetTransactionTimeout(UIA_TIMEOUT_MS)?;
            }
            // Một lượt cross-process lấy đủ khung và loại (SPEC 16.4).
            let cr = a.CreateCacheRequest()?;
            cr.AddProperty(UIA_BoundingRectanglePropertyId)?;
            cr.AddProperty(UIA_ControlTypePropertyId)?;
            let view = a.ControlViewCondition()?;
            cr.SetTreeFilter(&view)?;
            Ok(Self { a, cr, view })
        }
    }
}

/// Nút cây UIA; con lấy lười và giữ lại trong phiên.
struct Node {
    el: IUIAutomationElement,
    rect: Option<Rect>,
    kind: FrameKind,
    kids: Option<Vec<Node>>,
}

impl Node {
    fn new(el: IUIAutomationElement) -> Self {
        let rect = unsafe { el.CachedBoundingRectangle() }.ok().as_ref().and_then(rect_of);
        let kind = unsafe { el.CachedControlType() }.map(|c| FrameKind::from_uia(c.0)).unwrap_or(FrameKind::Other);
        Self { el, rect, kind, kids: None }
    }

    fn contains(&self, p: (i32, i32)) -> bool {
        self.rect.is_some_and(|r| r.contains_point(p.0, p.1))
    }

    /// Con (1 lượt cross-process nếu chưa có). `Err(())` = timeout: app treo hoặc quá chậm.
    fn kids(&mut self, u: &Uia) -> Result<&mut Vec<Node>, ()> {
        if self.kids.is_none() {
            let v = match unsafe { self.el.FindAllBuildCache(TreeScope_Children, &u.view, &u.cr) } {
                Ok(arr) => {
                    let n = unsafe { arr.Length() }.unwrap_or(0);
                    (0..n).filter_map(|i| unsafe { arr.GetElement(i) }.ok()).map(Node::new).collect()
                }
                Err(e) if e.code().0 == E_TIMEOUT => return Err(()),
                // Phần tử đã mất (menu đóng, trang đổi...): coi như không có con.
                Err(_) => Vec::new(),
            };
            self.kids = Some(v);
        }
        Ok(self.kids.as_mut().unwrap())
    }
}

struct Tree {
    root: Option<Node>,
    /// Timeout hoặc lỗi: trong phiên này chỉ dùng khung cửa sổ (+ khung từ ảnh).
    dead: bool,
}

struct Session {
    gen: u32,
    frozen: Vec<(Rect, Arc<RgbaImage>)>,
    image_edges: bool,
    live_capture: bool,
    trees: HashMap<isize, Tree>,
    /// Bản đồ cạnh theo màn hình (khoá = rect màn hình); `None` = đã thử, không dựng được.
    edges: HashMap<(i32, i32, u32, u32), Option<EdgeMap>>,
}

impl Session {
    fn new(gen: u32, frozen: Vec<(Rect, Arc<RgbaImage>)>, image_edges: bool, live_capture: bool) -> Self {
        Self { gen, frozen, image_edges, live_capture, trees: HashMap::new(), edges: HashMap::new() }
    }

    /// Chuỗi khung thô từ ngoài vào trong (phần tử đầu là cửa sổ). Overlay tự `normalize`.
    fn chain(&mut self, uia: Option<&Uia>, p: (i32, i32), win: WinFrame, mon: Rect) -> Vec<Frame> {
        let deadline = Instant::now() + QUERY_BUDGET;
        let mut chain = vec![Frame::new(win.rect, FrameKind::Window)];
        let mut bottom = true;
        if let Some(u) = uia {
            let tree = self.trees.entry(win.id).or_insert_with(|| Tree { root: None, dead: false });
            if !tree.dead && tree.root.is_none() {
                match unsafe { u.a.ElementFromHandleBuildCache(HWND(win.id as *mut _), &u.cr) } {
                    Ok(el) => tree.root = Some(Node::new(el)),
                    Err(e) => {
                        // Cửa sổ đã đóng (menu Freeze), app quyền admin, app treo.
                        log::debug!("autoselect: UIA cửa sổ {:#x}: {e}", win.id);
                        tree.dead = true;
                    }
                }
            }
            if let (false, Some(root)) = (tree.dead, tree.root.as_mut()) {
                match descend(root, p, u, deadline, &mut chain) {
                    Ok(done) => bottom = done,
                    Err(()) => {
                        log::info!("autoselect: UIA timeout ở cửa sổ {:#x}, chỉ dùng khung cửa sổ", win.id);
                        tree.dead = true;
                    }
                }
            }
        }
        // Nguồn 3, phạm vi hẹp: chỉ khi đã xuống tới đáy cây UIA (không còn con chứa con trỏ).
        if self.image_edges && bottom {
            let within = chain.last().map(|f| f.rect).unwrap_or(win.rect);
            if let Some(r) = self.detect(p, within, mon) {
                chain.push(Frame::new(r, FrameKind::Detected));
            }
        }
        chain
    }

    fn detect(&mut self, p: (i32, i32), within: Rect, mon: Rect) -> Option<Rect> {
        let within = within.intersect(&mon)?;
        let key = (mon.x, mon.y, mon.w, mon.h);
        if !self.edges.contains_key(&key) {
            let map = self.build_edges(mon);
            self.edges.insert(key, map);
        }
        let map = self.edges.get(&key)?.as_ref()?;
        let r = map.query((p.0 - mon.x, p.1 - mon.y), within.offset(-mon.x, -mon.y))?;
        Some(r.offset(mon.x, mon.y))
    }

    fn build_edges(&self, mon: Rect) -> Option<EdgeMap> {
        let t0 = Instant::now();
        // Ghép theo góc trên-trái như show_freeze_overlays: kích thước xcap có thể lệch vài px.
        let frozen = self.frozen.iter().find(|(r, _)| (r.x, r.y) == (mon.x, mon.y)).map(|(_, img)| img.clone());
        let img = match frozen {
            Some(img) => img,
            None if self.live_capture => match capture::grab_region(mon) {
                Ok(img) => Arc::new(img),
                Err(e) => {
                    log::warn!("autoselect: chụp màn hình để nhận khung lỗi: {e}");
                    return None;
                }
            },
            None => return None,
        };
        let map = EdgeMap::build(img.as_raw(), img.width(), img.height(), THRESHOLD);
        log::debug!(
            "autoselect: bản đồ cạnh {}×{} ({} MB) {:?}",
            img.width(),
            img.height(),
            map.bytes() / 1_000_000,
            t0.elapsed()
        );
        Some(map)
    }
}

/// Đi xuống từ `root` tới phần tử sâu nhất chứa `p`, đẩy từng khung vào `chain`.
/// - Chọn con nhỏ nhất chứa điểm. Nếu nó không có con chứa điểm mà có anh em trùng khít khung (Chromium có Pane
///   trùng khít, cái rỗng), thử anh em đó.
/// - `Ok(true)`: đã tới đáy. `Ok(false)`: hết hạn giữa chừng. `Err(())`: timeout UIA.
fn descend(root: &mut Node, p: (i32, i32), u: &Uia, deadline: Instant, chain: &mut Vec<Frame>) -> Result<bool, ()> {
    let mut cur = root;
    for _ in 0..64 {
        if Instant::now() > deadline {
            return Ok(false);
        }
        let pick = {
            let kids = cur.kids(u)?;
            let mut cand: Vec<usize> = (0..kids.len()).filter(|&i| kids[i].contains(p)).collect();
            cand.sort_by_key(|&i| kids[i].rect.map(|r| r.w as u64 * r.h as u64).unwrap_or(u64::MAX));
            let Some(&first) = cand.first() else { return Ok(true) };
            let first_rect = kids[first].rect;
            let mut chosen = first;
            if cand.len() > 1 {
                let has_inner = |n: &mut Node| -> Result<bool, ()> { Ok(n.kids(u)?.iter().any(|k| k.contains(p))) };
                if !has_inner(&mut kids[first])? {
                    let twins: Vec<usize> = cand.iter().skip(1).copied().filter(|&j| kids[j].rect == first_rect).collect();
                    for j in twins {
                        if has_inner(&mut kids[j])? {
                            chosen = j;
                            break;
                        }
                    }
                }
            }
            chosen
        };
        let c = cur;
        cur = &mut c.kids.as_mut().unwrap()[pick];
        if let Some(r) = cur.rect {
            chain.push(Frame::new(r, cur.kind));
        }
    }
    Ok(true)
}
