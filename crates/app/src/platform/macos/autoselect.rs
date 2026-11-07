//! Tự động chọn vùng (SPEC 16), phần macOS (thiết kế macOS mục 4): danh sách cửa sổ (CoreGraphics) và worker
//! Accessibility + khung từ ảnh. Cùng giao thức với vỏ Windows (`Begin` / `Query` / `End`, kết quả qua
//! `Action::AutoFrames` kèm thế hệ phiên).
//!
//! - `snapshot_windows` chạy trên UI thread ngay trước khi hiện overlay.
//! - Phần tử dưới con trỏ: `AXUIElementCopyElementAtPosition` rồi đi lên tới cửa sổ. Cần quyền Trợ năng
//!   (Accessibility); chưa có thì chỉ có khung cửa sổ và khung từ ảnh.
//! - Khung từ ảnh: chỉ ở Freeze (có ảnh đóng băng). Live trên macOS chưa chụp ngầm để nhận khung.

use std::{
    collections::HashMap,
    ffi::c_void,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
    time::Instant,
};

use chup_core::{
    autoselect::{Frame, FrameKind, WinFrame},
    coords::{MacLayout, Rect, RectPt},
    edges::{EdgeMap, THRESHOLD},
};
use image::RgbaImage;
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

use super::{app::Action, win::emit};

// ---------- Danh sách cửa sổ ----------

/// Lớp cửa sổ được tính (`kCGWindowLayer`): cửa sổ thường, nổi, modal, menu bật lên. Bỏ Dock, menu bar, status item.
const LAYERS: [i64; 5] = [0, 3, 8, 19, 101];

fn num(d: &NSDictionary<NSString, AnyObject>, key: &str) -> Option<f64> {
    let v = d.objectForKey(&NSString::from_str(key))?;
    v.downcast::<NSNumber>().ok().map(|n| n.doubleValue())
}

/// Cửa sổ trên màn hình theo z-order (trên cùng trước), bỏ cửa sổ của Chup và cửa sổ không nhìn thấy.
pub fn snapshot_windows(layout: &MacLayout) -> Vec<WinFrame> {
    let t0 = Instant::now();
    let opts = objc2_core_graphics::CGWindowListOption::OptionOnScreenOnly
        | objc2_core_graphics::CGWindowListOption::ExcludeDesktopElements;
    let Some(arr) = objc2_core_graphics::CGWindowListCopyWindowInfo(opts, 0) else { return vec![] };
    // SAFETY: CFArray của CFDictionary, toll-free bridged với NSArray<NSDictionary>.
    let arr: &NSArray<NSDictionary<NSString, AnyObject>> = unsafe { &*(&*arr as *const _ as *const _) };
    let me = std::process::id() as f64;
    let mut out = vec![];
    for d in arr.iter() {
        if num(&d, "kCGWindowOwnerPID") == Some(me) {
            continue;
        }
        let layer = num(&d, "kCGWindowLayer").unwrap_or(0.0) as i64;
        if !LAYERS.contains(&layer) || num(&d, "kCGWindowAlpha").unwrap_or(1.0) <= 0.01 {
            continue;
        }
        let Some(b) = d.objectForKey(&NSString::from_str("kCGWindowBounds")) else { continue };
        let Ok(b) = b.downcast::<NSDictionary>() else { continue };
        // SAFETY: khoá là NSString, giá trị là NSNumber.
        let b: &NSDictionary<NSString, AnyObject> = unsafe { &*(&*b as *const NSDictionary as *const _) };
        let (Some(x), Some(y), Some(w), Some(h)) = (num(b, "X"), num(b, "Y"), num(b, "Width"), num(b, "Height")) else {
            continue;
        };
        if w < 4.0 || h < 4.0 {
            continue;
        }
        let id = num(&d, "kCGWindowNumber").unwrap_or(0.0) as isize;
        out.push(WinFrame { id, rect: layout.cg_to_px(RectPt::new(x, y, w, h)) });
    }
    log::debug!("autoselect: {} cửa sổ, {:?}", out.len(), t0.elapsed());
    out
}

// ---------- Accessibility (ApplicationServices) ----------

type CFTypeRef = *const c_void;
type AXUIElementRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyElementAtPosition(app: AXUIElementRef, x: f32, y: f32, out: *mut AXUIElementRef) -> i32;
    fn AXUIElementCopyAttributeValue(el: AXUIElementRef, attr: CFTypeRef, out: *mut CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(el: AXUIElementRef, seconds: f32) -> i32;
    fn AXValueGetValue(v: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> bool;
}

const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;

/// Đã có quyền Trợ năng.
pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Xin quyền Trợ năng: macOS hiện hộp thoại dẫn tới Cài đặt hệ thống (một lần).
pub fn request_trust() -> bool {
    let key = NSString::from_str("AXTrustedCheckOptionPrompt");
    let val = NSNumber::new_bool(true);
    let dict = NSDictionary::from_retained_objects(&[&*key], &[val]);
    unsafe { AXIsProcessTrustedWithOptions(Retained::as_ptr(&dict) as CFTypeRef) }
}

/// Phần tử AX (giữ một tham chiếu, thả khi drop).
struct Ax(AXUIElementRef);

impl Drop for Ax {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

// SAFETY: AXUIElement dùng được từ thread bất kỳ; chỉ worker giữ.
unsafe impl Send for Ax {}

fn attr(el: &Ax, name: &str) -> Option<CFTypeRef> {
    let key = NSString::from_str(name);
    let mut out: CFTypeRef = std::ptr::null();
    let r = unsafe { AXUIElementCopyAttributeValue(el.0, Retained::as_ptr(&key) as CFTypeRef, &mut out) };
    (r == 0 && !out.is_null()).then_some(out)
}

fn role(el: &Ax) -> Option<String> {
    let v = attr(el, "AXRole")?;
    // SAFETY: AXRole là CFString (toll-free với NSString); thả sau khi đọc.
    let s = unsafe { (*(v as *const NSString)).to_string() };
    unsafe { CFRelease(v) };
    Some(s)
}

/// Khung của phần tử (point CoreGraphics toàn cục).
fn frame(el: &Ax) -> Option<RectPt> {
    let (p, s) = (attr(el, "AXPosition")?, attr(el, "AXSize"));
    let mut pt = [0f64; 2];
    let mut sz = [0f64; 2];
    let ok = unsafe { AXValueGetValue(p, AX_VALUE_CGPOINT, pt.as_mut_ptr().cast()) };
    unsafe { CFRelease(p) };
    let s = s?;
    let ok2 = unsafe { AXValueGetValue(s, AX_VALUE_CGSIZE, sz.as_mut_ptr().cast()) };
    unsafe { CFRelease(s) };
    (ok && ok2 && sz[0] > 1.0 && sz[1] > 1.0).then(|| RectPt::new(pt[0], pt[1], sz[0], sz[1]))
}

fn parent(el: &Ax) -> Option<Ax> {
    attr(el, "AXParent").map(Ax)
}

/// Chuỗi khung từ cửa sổ vào phần tử dưới điểm (point CG), từ ngoài vào trong.
fn ax_chain(sys: &Ax, pt: (f64, f64), layout: &MacLayout) -> Vec<Frame> {
    let mut el: AXUIElementRef = std::ptr::null();
    if unsafe { AXUIElementCopyElementAtPosition(sys.0, pt.0 as f32, pt.1 as f32, &mut el) } != 0 || el.is_null() {
        return vec![];
    }
    let mut cur = Some(Ax(el));
    let mut up = vec![];
    for _ in 0..64 {
        let Some(e) = cur.take() else { break };
        let r = role(&e).unwrap_or_default();
        if r == "AXApplication" {
            break;
        }
        if let Some(f) = frame(&e) {
            let rect = layout.cg_to_px(f);
            // Bỏ phần tử trùng khít phần tử con (nhiều lớp group trong suốt).
            if up.last().is_none_or(|l: &Frame| l.rect != rect) {
                up.push(Frame::new(rect, FrameKind::from_ax(&r)));
            }
        }
        if r == "AXWindow" {
            break;
        }
        let p = parent(&e);
        // Vòng lặp lạ: phần tử là cha của chính nó.
        if p.as_ref().is_some_and(|p| unsafe { CFEqual(p.0, e.0) }) {
            break;
        }
        cur = p;
    }
    up.reverse();
    up
}

// ---------- Worker ----------

enum Req {
    Begin { gen: u32, frozen: Vec<(Rect, Arc<RgbaImage>)>, image_edges: bool, layout: MacLayout },
    Query { gen: u32, p: (i32, i32), win: WinFrame, mon: Rect },
    End { gen: u32 },
}

pub struct Worker {
    tx: Sender<Req>,
}

impl Worker {
    pub fn start() -> Worker {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("autoselect".into()).spawn(move || run(rx)).ok();
        Worker { tx }
    }

    pub fn begin(&self, gen: u32, frozen: Vec<(Rect, Arc<RgbaImage>)>, image_edges: bool, layout: MacLayout) {
        let _ = self.tx.send(Req::Begin { gen, frozen, image_edges, layout });
    }

    pub fn query(&self, gen: u32, p: (i32, i32), win: WinFrame, mon: Rect) {
        let _ = self.tx.send(Req::Query { gen, p, win, mon });
    }

    pub fn end(&self, gen: u32) {
        let _ = self.tx.send(Req::End { gen });
    }
}

const LADDER_MAX: usize = 6;
/// Timeout mỗi lời gọi Accessibility (app treo không làm overlay đứng).
const AX_TIMEOUT_S: f32 = 0.25;

struct Session {
    gen: u32,
    frozen: Vec<(Rect, Arc<RgbaImage>)>,
    image_edges: bool,
    layout: MacLayout,
    edges: HashMap<(i32, i32, u32, u32), Option<EdgeMap>>,
}

fn run(rx: Receiver<Req>) {
    let sys = Ax(unsafe { AXUIElementCreateSystemWide() });
    unsafe { AXUIElementSetMessagingTimeout(sys.0, AX_TIMEOUT_S) };
    let mut sess: Option<Session> = None;
    while let Ok(first) = rx.recv() {
        let mut reqs = vec![first];
        reqs.extend(rx.try_iter());
        let last_q = reqs.iter().rposition(|r| matches!(r, Req::Query { .. }));
        for (i, r) in reqs.into_iter().enumerate() {
            match r {
                Req::Begin { gen, frozen, image_edges, layout } => {
                    sess = Some(Session { gen, frozen, image_edges, layout, edges: HashMap::new() });
                }
                Req::End { gen } => {
                    if sess.as_ref().is_some_and(|s| s.gen == gen) {
                        sess = None;
                    }
                }
                Req::Query { gen, p, win, mon } if Some(i) == last_q => {
                    let Some(s) = sess.as_mut().filter(|s| s.gen == gen) else { continue };
                    let chain = objc2::rc::autoreleasepool(|_| s.chain(&sys, p, win, mon));
                    emit(Action::AutoFrames { gen, p, window: win.id, chain });
                }
                Req::Query { .. } => {}
            }
        }
    }
}

impl Session {
    fn chain(&mut self, sys: &Ax, p: (i32, i32), win: WinFrame, mon: Rect) -> Vec<Frame> {
        let mut chain = vec![Frame::new(win.rect, FrameKind::Window)];
        if trusted() {
            let pt = self.layout.px_to_cg(p.0, p.1);
            let ax = ax_chain(sys, pt, &self.layout);
            // Bỏ khung cửa sổ của AX (đã có khung cửa sổ từ CoreGraphics), chỉ giữ phần tử nằm trong cửa sổ.
            chain.extend(ax.into_iter().filter(|f| f.kind != FrameKind::Window && win.rect.intersect(&f.rect).is_some()));
        }
        if self.image_edges {
            let within = chain.last().map(|f| f.rect).unwrap_or(win.rect);
            for r in self.detect(p, within, mon).into_iter().rev() {
                chain.push(Frame::new(r, FrameKind::Detected));
            }
        }
        chain
    }

    fn detect(&mut self, p: (i32, i32), within: Rect, mon: Rect) -> Vec<Rect> {
        let Some(within) = within.intersect(&mon) else { return vec![] };
        let key = (mon.x, mon.y, mon.w, mon.h);
        if !self.edges.contains_key(&key) {
            let img = self.frozen.iter().find(|(r, _)| *r == mon).map(|(_, i)| i.clone());
            let map = img.map(|img| {
                let t0 = Instant::now();
                let m = EdgeMap::build(img.as_raw(), img.width(), img.height(), THRESHOLD);
                log::debug!("autoselect: bản đồ cạnh {}×{} {:?}", img.width(), img.height(), t0.elapsed());
                m
            });
            self.edges.insert(key, map);
        }
        let Some(Some(map)) = self.edges.get(&key) else { return vec![] };
        map.ladder((p.0 - mon.x, p.1 - mon.y), within.offset(-mon.x, -mon.y), LADDER_MAX)
            .into_iter()
            .map(|r| r.offset(mon.x, mon.y))
            .collect()
    }
}
