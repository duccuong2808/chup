//! Viền vùng chọn (SPEC 5.2, 5.3): cửa sổ = region nới ra `margin` mỗi cạnh; mọi thứ nhìn thấy nằm trong phần lề
//! nên không bao giờ dính vào ảnh. Vẽ bằng thao tác pixel.

use chup_core::coords::Rect;

use crate::{Color, Pixels, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

/// Pixel "gần như trong suốt nhưng vẫn nhận chuột" (layered window của Windows bỏ qua alpha 0).
pub const GRAB_PX: u32 = 0x0100_0000;

fn sz(scale: f64, dip: f64) -> i32 {
    (dip * scale).round().max(1.0) as i32
}

/// Kích thước handle và vị trí của chúng theo scale.
struct Metrics {
    /// Viền màu nhấn.
    bw: i32,
    /// Cạnh handle.
    hs: i32,
    /// Viền trắng trong handle.
    hb: i32,
    /// Khoảng từ mép cửa sổ tới handle.
    off: i32,
    /// Nới vùng bắt chuột quanh handle.
    pad: i32,
}

fn metrics(margin: i32, scale: f64) -> Metrics {
    let hs = sz(scale, 8.0);
    Metrics {
        bw: sz(scale, 2.0),
        hs,
        hb: sz(scale, 1.5),
        off: margin - sz(scale, 3.0) - hs,
        pad: sz(scale, 3.0),
    }
}

/// 8 handle theo thứ tự NW, N, NE, E, SE, S, SW, W (toạ độ góc trên-trái trong cửa sổ).
pub fn handle_origins(w: i32, h: i32, off: i32, hs: i32) -> [(i32, i32); 8] {
    let (cx, cy) = ((w - hs) / 2, (h - hs) / 2);
    let (r, b) = (w - off - hs, h - off - hs);
    [
        (off, off),
        (cx, off),
        (r, off),
        (r, cy),
        (r, b),
        (cx, b),
        (off, b),
        (off, cy),
    ]
}

/// Vẽ frame cỡ `size` (= region + lề) vào `px` (có thể lớn hơn `size`). `grab`: lề nhận chuột bằng pixel
/// gần trong suốt (Windows); OS tự quyết vùng nhận chuột (macOS) thì để `false`.
pub fn paint(
    px: &mut Pixels,
    size: (i32, i32),
    margin: i32,
    scale: f64,
    faded: bool,
    theme: &Theme,
    grab: bool,
) {
    let (w, h, m) = (size.0, size.1, margin);
    let mt = metrics(m, scale);
    let all = px.bounds();
    px.fill(all, 0);
    let region = Rect::new(m, m, (w - 2 * m).max(0) as u32, (h - 2 * m).max(0) as u32);
    let alpha = if faded { 0.3 } else { 1.0 };
    if !faded && grab {
        px.fill(Rect::new(0, 0, w as u32, h as u32), GRAB_PX);
        px.fill(region, 0);
    }
    // Viền màu nhấn nằm ngay ngoài region, viền trắng mảnh ngoài nữa.
    let band = region.expand(mt.bw);
    px.outline_outside(band, 1, Color::rgba(255, 255, 255, 0.9 * alpha).px());
    px.outline_outside(region, mt.bw, theme.accent.with_alpha(alpha).px());
    if !faded {
        for (hx, hy) in handle_origins(w, h, mt.off, mt.hs) {
            let r = Rect::new(hx, hy, mt.hs as u32, mt.hs as u32);
            px.fill(r, theme.accent.px());
            px.fill(r.expand(-mt.hb), Color::WHITE.px());
        }
    }
}

/// Phần của frame dưới điểm (lx, ly) (toạ độ trong cửa sổ). Faded: không nhận chuột.
pub fn hit(
    size: (i32, i32),
    margin: i32,
    scale: f64,
    faded: bool,
    lx: i32,
    ly: i32,
) -> Option<Part> {
    if faded {
        return None;
    }
    let (w, h, m) = (size.0, size.1, margin);
    let mt = metrics(m, scale);
    let parts = [
        Part::NW,
        Part::N,
        Part::NE,
        Part::E,
        Part::SE,
        Part::S,
        Part::SW,
        Part::W,
    ];
    for (p, (hx, hy)) in parts.iter().zip(handle_origins(w, h, mt.off, mt.hs)) {
        let side = (mt.hs + 2 * mt.pad) as u32;
        if Rect::new(hx - mt.pad, hy - mt.pad, side, side).contains_point(lx, ly) {
            return Some(*p);
        }
    }
    let inner = Rect::new(m, m, (w - 2 * m).max(0) as u32, (h - 2 * m).max(0) as u32);
    if inner.contains_point(lx, ly) {
        return None;
    }
    Rect::new(0, 0, w as u32, h as u32)
        .contains_point(lx, ly)
        .then_some(Part::Move)
}

/// Rect ngoài mới khi kéo `part` đi (dx, dy) từ `outer` (lúc nhấn). Region không nhỏ hơn 8 px.
pub fn drag(part: Part, outer: Rect, dx: i32, dy: i32, margin: i32) -> Rect {
    let min = 2 * margin + 8;
    let (mut l, mut t, mut r, mut b) = (outer.x, outer.y, outer.right(), outer.bottom());
    match part {
        Part::Move => {
            l += dx;
            r += dx;
            t += dy;
            b += dy;
        }
        p => {
            if matches!(p, Part::W | Part::NW | Part::SW) {
                l = (l + dx).min(r - min);
            }
            if matches!(p, Part::E | Part::NE | Part::SE) {
                r = (r + dx).max(l + min);
            }
            if matches!(p, Part::N | Part::NW | Part::NE) {
                t = (t + dy).min(b - min);
            }
            if matches!(p, Part::S | Part::SW | Part::SE) {
                b = (b + dy).max(t + min);
            }
        }
    }
    Rect::new(l, t, (r - l) as u32, (b - t) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_handles_edges_and_inside() {
        // Lề 12, scale 1: handle 8 px, cách mép 1 px.
        let size = (112, 112);
        assert_eq!(hit(size, 12, 1.0, false, 3, 3), Some(Part::NW));
        assert_eq!(hit(size, 12, 1.0, false, 56, 3), Some(Part::N));
        assert_eq!(hit(size, 12, 1.0, false, 108, 108), Some(Part::SE));
        assert_eq!(hit(size, 12, 1.0, false, 30, 5), Some(Part::Move));
        assert_eq!(hit(size, 12, 1.0, false, 50, 50), None);
        assert_eq!(hit(size, 12, 1.0, true, 3, 3), None);
        assert_eq!(hit(size, 12, 1.0, false, 200, 3), None);
    }

    #[test]
    fn drag_keeps_min_size() {
        let o = Rect::new(100, 100, 100, 100);
        assert_eq!(drag(Part::Move, o, 5, -5, 10), Rect::new(105, 95, 100, 100));
        assert_eq!(drag(Part::SE, o, 10, 20, 10), Rect::new(100, 100, 110, 120));
        // Kéo mép trái quá mép phải: dừng ở cỡ tối thiểu 2*10+8.
        assert_eq!(drag(Part::W, o, 500, 0, 10), Rect::new(172, 100, 28, 100));
    }

    #[test]
    fn paint_leaves_region_transparent() {
        let (w, h) = (40, 40);
        let mut buf = vec![0xDEAD_BEEFu32; (w * h) as usize];
        let mut px = Pixels::new(&mut buf, w, h);
        paint(&mut px, (w, h), 12, 1.0, false, &Theme::new(false), true);
        assert_eq!(buf[(20 * w + 20) as usize], 0);
        // Lề nhận chuột.
        assert_eq!(buf[(5 * w + 27) as usize], GRAB_PX);
        // Viền màu nhấn ngay ngoài region.
        assert_eq!(buf[(11 * w + 20) as usize], Theme::new(false).accent.px());
    }
}
