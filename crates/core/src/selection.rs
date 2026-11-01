//! Hình học chọn và chỉnh vùng trên overlay Freeze (SPEC 14.3): 8 handle, di chuyển, resize.
//! Toạ độ là physical px trong overlay của 1 màn hình. Module thuần, unit test được.

use crate::coords::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Move,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    NW,
}

/// 8 handle là ô vuông cạnh `size`, tâm đặt ở 4 góc và trung điểm 4 cạnh.
/// Góc xếp trước cạnh, để `hit` ưu tiên góc khi vùng nhỏ làm các handle chồng nhau.
pub fn handles(r: Rect, size: i32) -> [(Grip, Rect); 8] {
    let (l, t, rt, b) = (r.x, r.y, r.right(), r.bottom());
    let (cx, cy) = (l + (rt - l) / 2, t + (b - t) / 2);
    let sq = |x: i32, y: i32| Rect::new(x - size / 2, y - size / 2, size as u32, size as u32);
    [
        (Grip::NW, sq(l, t)),
        (Grip::NE, sq(rt, t)),
        (Grip::SW, sq(l, b)),
        (Grip::SE, sq(rt, b)),
        (Grip::N, sq(cx, t)),
        (Grip::S, sq(cx, b)),
        (Grip::W, sq(l, cy)),
        (Grip::E, sq(rt, cy)),
    ]
}

/// Điểm `p` nằm trên handle nào, trong vùng (`Move`), hay ở ngoài (`None`).
pub fn hit(r: Rect, p: (i32, i32), size: i32) -> Option<Grip> {
    handles(r, size)
        .iter()
        .find(|(_, h)| h.contains_point(p.0, p.1))
        .map(|(g, _)| *g)
        .or_else(|| r.contains_point(p.0, p.1).then_some(Grip::Move))
}

/// Vùng mới khi kéo `grip` từ `from` tới `to`. Luôn tính từ vùng lúc nhấn chuột (`r`),
/// không cộng dồn qua từng lần di chuột.
/// - `Move`: giữ nguyên kích thước, kẹp trong `bounds`.
/// - Resize: kéo qua cạnh đối diện thì vùng lật chiều, tối thiểu 1×1, không vượt `bounds`.
pub fn apply(r: Rect, grip: Grip, from: (i32, i32), to: (i32, i32), bounds: Rect) -> Rect {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    if grip == Grip::Move {
        let x = (r.x + dx).clamp(bounds.x, (bounds.right() - r.w as i32).max(bounds.x));
        let y = (r.y + dy).clamp(bounds.y, (bounds.bottom() - r.h as i32).max(bounds.y));
        return Rect::new(x, y, r.w, r.h);
    }
    let (mut l, mut t, mut rt, mut b) = (r.x, r.y, r.right(), r.bottom());
    let cx = |v: i32| v.clamp(bounds.x, bounds.right());
    let cy = |v: i32| v.clamp(bounds.y, bounds.bottom());
    if matches!(grip, Grip::W | Grip::NW | Grip::SW) {
        l = cx(l + dx);
    }
    if matches!(grip, Grip::E | Grip::NE | Grip::SE) {
        rt = cx(rt + dx);
    }
    if matches!(grip, Grip::N | Grip::NW | Grip::NE) {
        t = cy(t + dy);
    }
    if matches!(grip, Grip::S | Grip::SW | Grip::SE) {
        b = cy(b + dy);
    }
    let mut out = Rect::from_points((l, t), (rt, b));
    if out.w == 0 {
        out.w = 1;
        out.x = out.x.min(bounds.right() - 1);
    }
    if out.h == 0 {
        out.h = 1;
        out.y = out.y.min(bounds.bottom() - 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect::new(100, 100, 200, 100);

    #[test]
    fn handles_sit_on_corners_and_edge_midpoints() {
        let h = handles(R, 10);
        assert_eq!(h[0], (Grip::NW, Rect::new(95, 95, 10, 10)));
        assert_eq!(h[3], (Grip::SE, Rect::new(295, 195, 10, 10)));
        assert_eq!(h[4], (Grip::N, Rect::new(195, 95, 10, 10)));
        assert_eq!(h[7], (Grip::E, Rect::new(295, 145, 10, 10)));
    }

    #[test]
    fn hit_handles_inside_outside() {
        assert_eq!(hit(R, (100, 100), 10), Some(Grip::NW));
        assert_eq!(hit(R, (304, 104), 10), Some(Grip::NE));
        assert_eq!(hit(R, (200, 100), 10), Some(Grip::N));
        assert_eq!(hit(R, (300, 150), 10), Some(Grip::E));
        assert_eq!(hit(R, (150, 150), 10), Some(Grip::Move));
        assert_eq!(hit(R, (50, 50), 10), None);
    }

    #[test]
    fn tiny_rect_prefers_corners() {
        // Vùng 4×4: mọi handle chồng nhau, góc phải thắng cạnh để resize được.
        assert_eq!(
            hit(Rect::new(100, 100, 4, 4), (102, 102), 10),
            Some(Grip::NW)
        );
    }

    const B: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn move_keeps_size_and_clamps() {
        assert_eq!(
            apply(R, Grip::Move, (150, 150), (170, 140), B),
            Rect::new(120, 90, 200, 100)
        );
        assert_eq!(
            apply(R, Grip::Move, (150, 150), (-1000, -1000), B),
            Rect::new(0, 0, 200, 100)
        );
        assert_eq!(
            apply(R, Grip::Move, (150, 150), (5000, 5000), B),
            Rect::new(1720, 980, 200, 100)
        );
    }

    #[test]
    fn resize_each_side() {
        assert_eq!(
            apply(R, Grip::SE, (300, 200), (350, 260), B),
            Rect::new(100, 100, 250, 160)
        );
        assert_eq!(
            apply(R, Grip::W, (100, 150), (50, 150), B),
            Rect::new(50, 100, 250, 100)
        );
        assert_eq!(
            apply(R, Grip::E, (300, 150), (5000, 150), B),
            Rect::new(100, 100, 1820, 100)
        );
    }

    #[test]
    fn resize_past_opposite_edge_flips() {
        assert_eq!(
            apply(R, Grip::N, (200, 100), (200, 250), B),
            Rect::new(100, 200, 200, 50)
        );
        assert_eq!(
            apply(R, Grip::NW, (100, 100), (400, 300), B),
            Rect::new(300, 200, 100, 100)
        );
    }

    #[test]
    fn resize_never_below_one_pixel() {
        assert_eq!(
            apply(R, Grip::E, (300, 150), (100, 150), B),
            Rect::new(100, 100, 1, 100)
        );
        // Kẹp ở mép phải màn hình mà vẫn giữ 1 px bên trong.
        let edge = Rect::new(1919, 0, 1, 10);
        assert_eq!(
            apply(edge, Grip::W, (1919, 5), (1920, 5), B),
            Rect::new(1919, 0, 1, 10)
        );
    }
}
