//! Hình học cửa sổ Preview (SPEC 17): chỗ mở và cỡ mở, zoom quanh con trỏ, kéo cạnh để đổi cỡ, opacity.
//! Mọi toạ độ là physical px trên toàn desktop. Ảnh luôn giữ tỉ lệ; cửa sổ = ảnh × zoom (không nhỏ hơn `min`).

use crate::{coords::Rect, selection::Grip};

/// Opacity thấp nhất cho phép (%): thấp hơn thì gần như không thấy cửa sổ để bấm lại.
pub const MIN_OPACITY: u8 = 20;
/// Mỗi nấc lăn chuột nhân zoom với số này.
pub const ZOOM_STEP: f64 = 1.25;
const MIN_ZOOM: f64 = 0.05;
const MAX_ZOOM: f64 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Rect cửa sổ.
    pub rect: Rect,
    /// Tỉ lệ ảnh hiển thị / ảnh gốc.
    pub zoom: f64,
}

impl View {
    /// Rect của ảnh trong cửa sổ (toạ độ cửa sổ): nằm giữa khi cửa sổ lớn hơn ảnh (cỡ tối thiểu).
    pub fn image_rect(&self, img: (u32, u32)) -> Rect {
        let (w, h) = scaled(img, self.zoom);
        Rect::new((self.rect.w as i32 - w as i32) / 2, (self.rect.h as i32 - h as i32) / 2, w, h)
    }
}

fn scaled(img: (u32, u32), zoom: f64) -> (u32, u32) {
    (((img.0 as f64 * zoom).round() as u32).max(1), ((img.1 as f64 * zoom).round() as u32).max(1))
}

fn window_size(img: (u32, u32), zoom: f64, min: (u32, u32)) -> (u32, u32) {
    let (w, h) = scaled(img, zoom);
    (w.max(min.0), h.max(min.1))
}

/// Zoom để ảnh vừa khung `bw × bh`.
fn fit(img: (u32, u32), bw: f64, bh: f64) -> f64 {
    (bw / img.0.max(1) as f64).min(bh / img.1.max(1) as f64)
}

/// Khoảng zoom cho phép: tới vừa khít work area (tối đa 800%).
fn zoom_range(img: (u32, u32), work: Rect) -> (f64, f64) {
    let hi = fit(img, work.w as f64, work.h as f64).min(MAX_ZOOM);
    (MIN_ZOOM.min(hi), hi)
}

fn clamp_into(x: i32, len: u32, lo: i32, span: u32) -> i32 {
    let max = lo + span as i32 - len as i32;
    if max < lo {
        lo
    } else {
        x.clamp(lo, max)
    }
}

fn clamp_view(rect: Rect, work: Rect) -> Rect {
    Rect::new(clamp_into(rect.x, rect.w, work.x, work.w), clamp_into(rect.y, rect.h, work.y, work.h), rect.w, rect.h)
}

/// Zoom khi mở: vừa khung, không phóng to quá 1:1 (ảnh chụp phóng to chỉ mờ đi).
fn open_zoom(img: (u32, u32), bw: f64, bh: f64, work: Rect) -> f64 {
    let (lo, hi) = zoom_range(img, work);
    fit(img, bw, bh).min(1.0).clamp(lo, hi)
}

/// Mở giữa work area; ảnh tối đa `frac_pct`% work area mỗi chiều.
pub fn open_center(img: (u32, u32), work: Rect, frac_pct: u8, min: (u32, u32)) -> View {
    let f = frac_pct as f64 / 100.0;
    let zoom = open_zoom(img, work.w as f64 * f, work.h as f64 * f, work);
    let (w, h) = window_size(img, zoom, min);
    let x = work.x + (work.w as i32 - w as i32) / 2;
    let y = work.y + (work.h as i32 - h as i32) / 2;
    View { rect: clamp_view(Rect::new(x, y, w, h), work), zoom }
}

/// Mở sát cạnh stack (cùng cạnh màn hình, căn đáy work area), cách stack `gap`.
/// `stack` = rect cửa sổ stack nếu đang hiện; không có thì sát cạnh màn hình.
pub fn open_beside_stack(
    img: (u32, u32),
    work: Rect,
    frac_pct: u8,
    min: (u32, u32),
    stack: Option<Rect>,
    edge_right: bool,
    gap: i32,
) -> View {
    let f = frac_pct as f64 / 100.0;
    let zoom = open_zoom(img, work.w as f64 * f, work.h as f64 * f, work);
    let (w, h) = window_size(img, zoom, min);
    let x = if edge_right {
        stack.map(|s| s.x).unwrap_or(work.right()) - gap - w as i32
    } else {
        stack.map(|s| s.right()).unwrap_or(work.x) + gap
    };
    let y = work.bottom() - gap - h as i32;
    View { rect: clamp_view(Rect::new(x, y, w, h), work), zoom }
}

/// Mở trong khung người dùng để lại lần trước: ảnh vừa khung (tối đa 1:1), bám cạnh khung gần cạnh màn hình hơn
/// (khung để ở góc dưới-phải thì ảnh mới cũng nằm sát góc đó).
pub fn open_in_box(img: (u32, u32), bx: Rect, work: Rect, min: (u32, u32)) -> View {
    let zoom = open_zoom(img, bx.w as f64, bx.h as f64, work);
    let (w, h) = window_size(img, zoom, min);
    let (bcx, bcy) = bx.center();
    let (wcx, wcy) = work.center();
    let x = if bcx > wcx { bx.right() - w as i32 } else { bx.x };
    let y = if bcy > wcy { bx.bottom() - h as i32 } else { bx.y };
    View { rect: clamp_view(Rect::new(x, y, w, h), work), zoom }
}

/// Zoom `steps` nấc (dương = to ra) quanh `anchor` (toạ độ desktop), giữ điểm dưới con trỏ đứng yên.
pub fn zoom_at(v: View, img: (u32, u32), steps: i32, anchor: (i32, i32), work: Rect, min: (u32, u32)) -> View {
    let (lo, hi) = zoom_range(img, work);
    let zoom = (v.zoom * ZOOM_STEP.powi(steps)).clamp(lo, hi);
    resize_to(v, img, zoom, anchor, work, min)
}

/// Về zoom `zoom`, giữ `anchor` đứng yên, kẹp vào `work`.
pub fn resize_to(v: View, img: (u32, u32), zoom: f64, anchor: (i32, i32), work: Rect, min: (u32, u32)) -> View {
    // Work area có thể đã đổi (kéo sang màn hình nhỏ hơn): zoom cũ có thể không còn vừa.
    let (lo, hi) = zoom_range(img, work);
    let zoom = zoom.clamp(lo, hi);
    let (w, h) = window_size(img, zoom, min);
    // Vị trí tương đối của anchor trong cửa sổ cũ (0..1), giữ nguyên trong cửa sổ mới.
    let fx = (anchor.0 - v.rect.x) as f64 / v.rect.w.max(1) as f64;
    let fy = (anchor.1 - v.rect.y) as f64 / v.rect.h.max(1) as f64;
    let x = anchor.0 - (fx * w as f64).round() as i32;
    let y = anchor.1 - (fy * h as f64).round() as i32;
    View { rect: clamp_view(Rect::new(x, y, w, h), work), zoom }
}

/// Điểm `p` (toạ độ cửa sổ) nằm trên cạnh/góc nào để đổi cỡ. Góc nhận vùng rộng gấp đôi cho dễ bắt.
pub fn edge_hit(size: (u32, u32), p: (i32, i32), band: i32) -> Option<Grip> {
    let (w, h) = (size.0 as i32, size.1 as i32);
    let (x, y) = p;
    if x < 0 || y < 0 || x >= w || y >= h {
        return None;
    }
    let (l, r, t, b) = (x < band, x >= w - band, y < band, y >= h - band);
    let c = band * 2;
    let (lc, rc, tc, bc) = (x < c, x >= w - c, y < c, y >= h - c);
    Some(match () {
        _ if (t && lc) || (l && tc) => Grip::NW,
        _ if (t && rc) || (r && tc) => Grip::NE,
        _ if (b && lc) || (l && bc) => Grip::SW,
        _ if (b && rc) || (r && bc) => Grip::SE,
        _ if t => Grip::N,
        _ if b => Grip::S,
        _ if l => Grip::W,
        _ if r => Grip::E,
        _ => return None,
    })
}

/// Kéo cạnh/góc `grip` của cửa sổ từ `from` tới `to` (bắt đầu từ view `start`): đổi cỡ giữ tỉ lệ ảnh,
/// cạnh đối diện đứng yên (kéo cạnh trên/dưới thì giữ tâm ngang, kéo cạnh trái/phải thì giữ tâm dọc).
pub fn drag_edge(
    start: View,
    img: (u32, u32),
    grip: Grip,
    from: (i32, i32),
    to: (i32, i32),
    work: Rect,
    min: (u32, u32),
) -> View {
    let (dx, dy) = ((to.0 - from.0) as f64, (to.1 - from.1) as f64);
    let (iw, ih) = scaled(img, start.zoom);
    let horiz = match grip {
        Grip::E | Grip::NE | Grip::SE => Some(dx),
        Grip::W | Grip::NW | Grip::SW => Some(-dx),
        _ => None,
    };
    let vert = match grip {
        Grip::S | Grip::SE | Grip::SW => Some(dy),
        Grip::N | Grip::NE | Grip::NW => Some(-dy),
        _ => None,
    };
    let zw = horiz.map(|d| (iw as f64 + d) / img.0.max(1) as f64);
    let zh = vert.map(|d| (ih as f64 + d) / img.1.max(1) as f64);
    let z = match (zw, zh) {
        (Some(a), Some(b)) => a.max(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return start,
    };
    let (lo, hi) = zoom_range(img, work);
    let zoom = z.clamp(lo, hi);
    let (w, h) = window_size(img, zoom, min);
    let s = start.rect;
    let x = match grip {
        Grip::W | Grip::NW | Grip::SW => s.right() - w as i32,
        Grip::E | Grip::NE | Grip::SE => s.x,
        _ => s.x + (s.w as i32 - w as i32) / 2,
    };
    let y = match grip {
        Grip::N | Grip::NE | Grip::NW => s.bottom() - h as i32,
        Grip::S | Grip::SE | Grip::SW => s.y,
        _ => s.y + (s.h as i32 - h as i32) / 2,
    };
    View { rect: clamp_view(Rect::new(x, y, w, h), work), zoom }
}

/// Opacity sau `steps` nấc 10%, kẹp trong `MIN_OPACITY..=100`.
pub fn step_opacity(cur: u8, steps: i32) -> u8 {
    (cur as i32 + steps * 10).clamp(MIN_OPACITY as i32, 100) as u8
}

pub fn clamp_opacity(pct: u8) -> u8 {
    pct.clamp(MIN_OPACITY, 100)
}

/// % -> alpha 0..255 cho `UpdateLayeredWindow`.
pub fn alpha(pct: u8) -> u8 {
    ((clamp_opacity(pct) as u32 * 255 + 50) / 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect { x: 0, y: 0, w: 1920, h: 1040 };
    const MIN: (u32, u32) = (160, 48);

    #[test]
    fn center_opens_one_to_one_or_fits_fraction() {
        let v = open_center((800, 600), WORK, 90, MIN);
        assert_eq!(v.zoom, 1.0);
        assert_eq!(v.rect, Rect::new(560, 220, 800, 600));
        // 4K trong 50%: 520 / 2160 (chiều cao quyết định).
        let v = open_center((3840, 2160), WORK, 50, MIN);
        assert!((v.zoom - 520.0 / 2160.0).abs() < 1e-9);
        assert_eq!((v.rect.w, v.rect.h), (924, 520));
    }

    #[test]
    fn beside_stack_hugs_stack_and_bottom() {
        let stack = Rect::new(1700, 600, 208, 428);
        let v = open_beside_stack((800, 600), WORK, 50, MIN, Some(stack), true, 12);
        // 50%: 960 × 520 -> zoom 520/600.
        assert_eq!(v.rect.right(), 1700 - 12);
        assert_eq!(v.rect.bottom(), 1040 - 12);
        // Stack bên trái, chưa hiện: sát cạnh trái work area.
        let left = Rect::new(-1920, 0, 1920, 1040);
        let v = open_beside_stack((400, 300), left, 50, MIN, None, false, 12);
        assert_eq!((v.rect.x, v.rect.bottom(), v.zoom), (-1908, 1028, 1.0));
    }

    #[test]
    fn box_fits_image_and_sticks_to_nearest_corner() {
        // Khung ở góc dưới-phải: ảnh dọc mới nằm sát góc dưới-phải của khung.
        let bx = Rect::new(1400, 640, 500, 380);
        let v = open_in_box((1000, 1000), bx, WORK, MIN);
        assert_eq!(v.zoom, 0.38);
        assert_eq!((v.rect.right(), v.rect.bottom(), v.rect.w), (1900, 1020, 380));
        // Khung ở góc trên-trái, ảnh nhỏ: 1:1, bám góc trên-trái.
        let v = open_in_box((200, 100), Rect::new(10, 10, 600, 400), WORK, MIN);
        assert_eq!(v.rect, Rect::new(10, 10, 200, 100));
    }

    #[test]
    fn tiny_image_gets_min_window_and_is_centered_inside() {
        let v = open_center((40, 10), WORK, 50, MIN);
        assert_eq!((v.rect.w, v.rect.h), (160, 48));
        assert_eq!(v.image_rect((40, 10)), Rect::new(60, 19, 40, 10));
    }

    #[test]
    fn zoom_keeps_point_under_cursor() {
        let v = open_center((800, 600), WORK, 90, MIN);
        let z = zoom_at(v, (800, 600), 1, (960, 520), WORK, MIN);
        assert_eq!(z.zoom, 1.25);
        assert_eq!(z.rect, Rect::new(460, 145, 1000, 750));
        let z = zoom_at(v, (800, 600), -1, (560, 220), WORK, MIN);
        assert_eq!(z.rect, Rect::new(560, 220, 640, 480));
    }

    #[test]
    fn zoom_is_clamped_to_work_area_and_limits() {
        let v = open_center((800, 600), WORK, 90, MIN);
        let z = zoom_at(v, (800, 600), 20, (1900, 1000), WORK, MIN);
        assert!((z.zoom - 1040.0 / 600.0).abs() < 1e-9);
        assert_eq!(z.rect.h, 1040);
        assert!(z.rect.x >= 0 && z.rect.right() <= 1920 && z.rect.y == 0);
        let z = zoom_at(v, (800, 600), -30, (960, 520), WORK, MIN);
        assert!((z.zoom - 0.05).abs() < 1e-9);
        // Về zoom cũ trên màn hình nhỏ hơn: kẹp cho vừa work area.
        let small = Rect::new(0, 0, 1280, 300);
        assert_eq!(resize_to(v, (800, 600), 1.0, (100, 100), small, MIN).rect.h, 300);
        assert_eq!((z.rect.w, z.rect.h), (160, 48));
    }

    #[test]
    fn edge_hit_finds_edges_and_wide_corners() {
        let s = (400, 300);
        assert_eq!(edge_hit(s, (2, 150), 6), Some(Grip::W));
        assert_eq!(edge_hit(s, (398, 150), 6), Some(Grip::E));
        assert_eq!(edge_hit(s, (200, 1), 6), Some(Grip::N));
        assert_eq!(edge_hit(s, (200, 297), 6), Some(Grip::S));
        // Góc: trong 12 px dọc theo cạnh.
        assert_eq!(edge_hit(s, (10, 2), 6), Some(Grip::NW));
        assert_eq!(edge_hit(s, (397, 290), 6), Some(Grip::SE));
        assert_eq!(edge_hit(s, (200, 150), 6), None);
        assert_eq!(edge_hit(s, (-1, 150), 6), None);
    }

    #[test]
    fn drag_edge_keeps_aspect_and_opposite_edge() {
        let v = View { rect: Rect::new(100, 100, 800, 600), zoom: 1.0 };
        // Kéo cạnh phải vào 200 px: rộng 600, cao 450, cạnh trái đứng yên, giữ tâm dọc.
        let r = drag_edge(v, (800, 600), Grip::E, (900, 400), (700, 400), WORK, MIN);
        assert_eq!(r.zoom, 0.75);
        assert_eq!(r.rect, Rect::new(100, 175, 600, 450));
        // Kéo góc trên-trái ra ngoài: góc dưới-phải đứng yên.
        let r = drag_edge(v, (800, 600), Grip::NW, (100, 100), (20, 60), WORK, MIN);
        assert_eq!(r.zoom, 1.1);
        assert_eq!((r.rect.right(), r.rect.bottom(), r.rect.w, r.rect.h), (900, 700, 880, 660));
        // Kéo quá nhỏ: dừng ở cỡ tối thiểu.
        let r = drag_edge(v, (800, 600), Grip::S, (500, 700), (500, -5000), WORK, MIN);
        assert!(r.rect.w >= 160 && r.rect.h >= 48);
    }

    #[test]
    fn opacity_steps_and_alpha() {
        assert_eq!(step_opacity(100, 1), 100);
        assert_eq!(step_opacity(100, -3), 70);
        assert_eq!(step_opacity(30, -5), 20);
        assert_eq!(clamp_opacity(5), 20);
        assert_eq!(alpha(100), 255);
        assert_eq!(alpha(50), 128);
        assert_eq!(alpha(0), 51);
    }
}
