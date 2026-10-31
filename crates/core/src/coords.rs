//! Toạ độ & hình học. Mọi `Rect` ở đây là **physical px trên toàn desktop**.
//! Module thuần (không phụ thuộc OS) để unit test được.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Hình chữ nhật từ 2 góc bất kỳ (ví dụ điểm bắt đầu và điểm hiện tại khi kéo chuột).
    pub fn from_points(a: (i32, i32), b: (i32, i32)) -> Self {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// Hình chữ nhật nhỏ nhất chứa cả 2 (rect rỗng bị bỏ qua).
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = self.right().max(other.right());
        let b = self.bottom().max(other.bottom());
        Rect::new(x, y, (r - x) as u32, (b - y) as u32)
    }

    /// Dời theo (dx, dy).
    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub fn right(&self) -> i32 {
        self.x + self.w as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h as i32
    }

    pub fn center(&self) -> (i32, i32) {
        (self.x + (self.w / 2) as i32, self.y + (self.h / 2) as i32)
    }

    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        (r > x && b > y).then(|| Rect::new(x, y, (r - x) as u32, (b - y) as u32))
    }

    /// Nới rộng (m > 0) hoặc thu hẹp (m < 0) đều 4 cạnh.
    pub fn expand(&self, m: i32) -> Rect {
        let w = (self.w as i64 + 2 * m as i64).max(0) as u32;
        let h = (self.h as i64 + 2 * m as i64).max(0) as u32;
        Rect::new(self.x - m, self.y - m, w, h)
    }
}

/// Thông tin 1 màn hình, do lớp platform đọc từ OS.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    /// Khoá ổn định trong 1 phiên: tên + vị trí.
    pub key: String,
    pub rect: Rect,
    /// Vùng làm việc (trừ taskbar).
    pub work: Rect,
    pub scale: f64,
    pub primary: bool,
}

/// Lề (physical) bao quanh region trong cửa sổ frame: chứa viền + handle.
/// Viền và handle nằm hoàn toàn ngoài region nên không bao giờ dính vào ảnh.
pub fn frame_margin(scale: f64) -> i32 {
    (FRAME_MARGIN_CSS * scale).ceil() as i32
}

pub const FRAME_MARGIN_CSS: f64 = 12.0;

/// Đặt toolbar cạnh region: ưu tiên dưới-phải, rồi trên-phải, cuối cùng nằm trong region.
/// `gap` đã gồm lề frame để toolbar không đè lên handle.
pub fn place_toolbar(region: &Rect, size: (u32, u32), work: &Rect, gap: i32) -> (i32, i32) {
    let (w, h) = (size.0 as i32, size.1 as i32);
    let min_x = work.x;
    let max_x = (work.right() - w).max(min_x);
    let x = (region.right() - w).clamp(min_x, max_x);

    let below = region.bottom() + gap;
    if below + h <= work.bottom() {
        return (x, below);
    }
    let above = region.y - gap - h;
    if above >= work.y {
        return (x, above);
    }
    let inside = (region.bottom() - h - gap).clamp(work.y, (work.bottom() - h).max(work.y));
    (x, inside)
}

/// Vị trí stack: góc dưới của work area, cạnh trái hoặc phải.
pub fn place_stack(size: (u32, u32), work: &Rect, right_edge: bool, margin: i32) -> (i32, i32) {
    let x = if right_edge {
        work.right() - size.0 as i32 - margin
    } else {
        work.x + margin
    };
    let y = work.bottom() - size.1 as i32 - margin;
    (x.max(work.x), y.max(work.y))
}

/// Màn hình chứa điểm, hoặc màn hình đầu tiên nếu không có.
pub fn monitor_at(mons: &[MonitorInfo], px: i32, py: i32) -> Option<&MonitorInfo> {
    mons.iter().find(|m| m.rect.contains_point(px, py))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_points_union_offset() {
        assert_eq!(Rect::from_points((50, 60), (10, 20)), Rect::new(10, 20, 40, 40));
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.union(&Rect::new(20, 5, 5, 10)), Rect::new(0, 0, 25, 15));
        assert_eq!(a.union(&Rect::default()), a);
        assert_eq!(a.offset(-5, 3), Rect::new(-5, 3, 10, 10));
    }

    #[test]
    fn expand_and_shrink_roundtrip() {
        let r = Rect::new(100, 100, 50, 40);
        assert_eq!(r.expand(12).expand(-12), r);
        assert_eq!(Rect::new(0, 0, 10, 10).expand(-20).w, 0);
    }

    #[test]
    fn toolbar_goes_below_then_above_then_inside() {
        let work = Rect::new(0, 0, 1000, 800);
        let size = (200, 40);
        // đủ chỗ bên dưới
        assert_eq!(place_toolbar(&Rect::new(100, 100, 300, 200), size, &work, 10), (200, 310));
        // sát đáy -> lên trên
        assert_eq!(place_toolbar(&Rect::new(100, 600, 300, 180), size, &work, 10), (200, 550));
        // chiếm gần hết màn hình -> vào trong
        assert_eq!(place_toolbar(&Rect::new(0, 20, 1000, 770), size, &work, 10), (800, 740));
        // region sát mép trái, toolbar rộng hơn region -> kẹp trong work area
        assert_eq!(place_toolbar(&Rect::new(0, 100, 50, 50), size, &work, 10).0, 0);
    }

    #[test]
    fn stack_sits_in_bottom_corner_of_work_area() {
        let work = Rect::new(0, 0, 1920, 1032);
        assert_eq!(place_stack((200, 300), &work, true, 12), (1708, 720));
        assert_eq!(place_stack((200, 300), &work, false, 12), (12, 720));
    }

    #[test]
    fn frame_margin_scales_up() {
        assert_eq!(frame_margin(1.0), 12);
        assert_eq!(frame_margin(1.5), 18);
        assert_eq!(frame_margin(1.25), 15);
    }
}
