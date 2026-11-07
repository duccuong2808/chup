//! Đổi path icon (`chup_core::icons::Cmd`, cung kiểu SVG) sang đoạn thẳng và Bézier bậc ba, cho API vẽ không có
//! cung SVG (CoreGraphics, Cairo). Direct2D có `AddArc` nên không cần.

use chup_core::icons::Cmd;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// Điểm điều khiển 1, 2, điểm cuối.
    Cubic((f32, f32), (f32, f32), (f32, f32)),
    Close,
}

/// Path với mọi cung đã đổi sang Bézier.
pub fn segments(cmds: &[Cmd]) -> Vec<Seg> {
    let mut out = Vec::with_capacity(cmds.len());
    let (mut cur, mut start) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
    for c in cmds {
        match *c {
            Cmd::Move(x, y) => {
                out.push(Seg::Move(x, y));
                cur = (x, y);
                start = cur;
            }
            Cmd::Line(x, y) => {
                out.push(Seg::Line(x, y));
                cur = (x, y);
            }
            Cmd::Arc { rx, ry, rot, large, sweep, x, y } => {
                arc(&mut out, cur, rx, ry, rot, large, sweep, (x, y));
                cur = (x, y);
            }
            Cmd::Close => {
                out.push(Seg::Close);
                cur = start;
            }
        }
    }
    out
}

/// Cung SVG từ `p0` tới `p1` (W3C SVG 1.1, phụ lục F.6.5: tham số điểm cuối -> tâm), chia ≤ 90° mỗi đoạn.
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<Seg>, p0: (f32, f32), rx: f32, ry: f32, rot: f32, large: bool, sweep: bool, p1: (f32, f32)) {
    let (x1, y1, x2, y2) = (p0.0 as f64, p0.1 as f64, p1.0 as f64, p1.1 as f64);
    let (mut rx, mut ry) = ((rx as f64).abs(), (ry as f64).abs());
    if (x1 - x2).abs() < 1e-9 && (y1 - y2).abs() < 1e-9 {
        return;
    }
    if rx < 1e-9 || ry < 1e-9 {
        out.push(Seg::Line(p1.0, p1.1));
        return;
    }
    let phi = (rot as f64).to_radians();
    let (cos, sin) = (phi.cos(), phi.sin());
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let (x1p, y1p) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    // Bán kính quá nhỏ: phóng lên vừa đủ.
    let lambda = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lambda > 1.0 {
        let k = lambda.sqrt();
        rx *= k;
        ry *= k;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut co = (num / den).max(0.0).sqrt();
    if large == sweep {
        co = -co;
    }
    let (cxp, cyp) = (co * rx * y1p / ry, -co * ry * x1p / rx);
    let (cx, cy) = (cos * cxp - sin * cyp + (x1 + x2) / 2.0, sin * cxp + cos * cyp + (y1 + y2) / 2.0);
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let (vx, vy) = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut dtheta = angle(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= std::f64::consts::TAU;
    } else if sweep && dtheta < 0.0 {
        dtheta += std::f64::consts::TAU;
    }
    let n = (dtheta.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = dtheta / n as f64;
    let t = 4.0 / 3.0 * (step / 4.0).tan();
    let map = |ux: f64, uy: f64| {
        let (x, y) = (rx * ux, ry * uy);
        ((cx + cos * x - sin * y) as f32, (cy + sin * x + cos * y) as f32)
    };
    let mut a = theta1;
    for i in 0..n {
        let b = a + step;
        let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
        let c1 = map(ca - t * sa, sa + t * ca);
        let c2 = map(cb + t * sb, sb - t * cb);
        // Đoạn cuối kết thúc đúng điểm đích (tránh sai số tích luỹ).
        let end = if i + 1 == n { p1 } else { map(cb, sb) };
        out.push(Seg::Cubic(c1, c2, end));
        a = b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn quarter_circle_is_one_cubic() {
        // Từ (20, 12) tới (12, 20), r = 8, theo chiều kim đồng hồ (y hướng xuống): tâm (12, 12).
        let segs = segments(&[Cmd::Move(20.0, 12.0), Cmd::Arc { rx: 8.0, ry: 8.0, rot: 0.0, large: false, sweep: true, x: 12.0, y: 20.0 }]);
        assert_eq!(segs.len(), 2);
        let Seg::Cubic(c1, c2, p) = segs[1] else { panic!("{segs:?}") };
        assert_eq!(p, (12.0, 20.0));
        // Điểm điều khiển chuẩn của 1/4 đường tròn: k = 0.5523 × r.
        assert!(close(c1, (20.0, 12.0 + 8.0 * 0.5523)), "{c1:?}");
        assert!(close(c2, (12.0 + 8.0 * 0.5523, 20.0)), "{c2:?}");
    }

    #[test]
    fn large_arc_splits_into_quarters() {
        // Nửa đường tròn trên (ngược kim đồng hồ khi y hướng xuống): 2 đoạn.
        let segs = segments(&[Cmd::Move(4.0, 12.0), Cmd::Arc { rx: 8.0, ry: 8.0, rot: 0.0, large: true, sweep: true, x: 20.0, y: 12.0 }]);
        assert_eq!(segs.iter().filter(|s| matches!(s, Seg::Cubic(..))).count(), 2);
        // Đi qua đỉnh (12, 4).
        let Seg::Cubic(_, _, mid) = segs[1] else { panic!() };
        assert!(close(mid, (12.0, 4.0)), "{mid:?}");
    }

    #[test]
    fn close_returns_to_start_and_tiny_radius_is_line() {
        let segs = segments(&[
            Cmd::Move(1.0, 1.0),
            Cmd::Arc { rx: 0.0, ry: 0.0, rot: 0.0, large: false, sweep: false, x: 5.0, y: 5.0 },
            Cmd::Close,
        ]);
        assert_eq!(segs, vec![Seg::Move(1.0, 1.0), Seg::Line(5.0, 5.0), Seg::Close]);
    }
}
