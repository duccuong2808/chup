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
}
