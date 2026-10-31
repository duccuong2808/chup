//! Toạ độ & hình học. Mọi `Rect` ở đây là **physical px trên toàn desktop**.
//! Module thuần (không phụ thuộc Tauri) để unit test được.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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

/// Thông tin 1 màn hình, chụp nhanh từ Tauri.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    /// Khoá ổn định trong 1 phiên: tên + vị trí.
    pub key: String,
    pub rect: Rect,
    /// Vùng làm việc (trừ taskbar).
    pub work: Rect,
    pub scale: f64,
}

/// Quy đổi rect CSS px (tương đối cửa sổ overlay của `mon`) sang physical px toàn desktop.
/// Theo SPEC 6.3: floor cho cạnh trái/trên, ceil cho cạnh phải/dưới, rồi kẹp trong màn hình.
pub fn css_to_physical(mon: &MonitorInfo, x: f64, y: f64, w: f64, h: f64) -> Option<Rect> {
    let s = mon.scale;
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    let left = mon.rect.x + (x * s).floor() as i32;
    let top = mon.rect.y + (y * s).floor() as i32;
    let right = mon.rect.x + ((x + w) * s).ceil() as i32;
    let bottom = mon.rect.y + ((y + h) * s).ceil() as i32;
    let raw = Rect::new(left, top, (right - left).max(0) as u32, (bottom - top).max(0) as u32);
    raw.intersect(&mon.rect)
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

    fn mon(x: i32, y: i32, w: u32, h: u32, scale: f64) -> MonitorInfo {
        MonitorInfo {
            key: "m".into(),
            rect: Rect::new(x, y, w, h),
            work: Rect::new(x, y, w, h - 48),
            scale,
        }
    }

    #[test]
    fn css_to_physical_at_100_percent() {
        let m = mon(0, 0, 1920, 1080, 1.0);
        assert_eq!(css_to_physical(&m, 10.0, 20.0, 100.0, 50.0), Some(Rect::new(10, 20, 100, 50)));
    }

    #[test]
    fn css_to_physical_at_150_percent_rounds_outward() {
        let m = mon(1920, 0, 2560, 1440, 1.5);
        // 10.5*1.5 = 15.75 -> floor 15 ; (10.5+100.3)*1.5 = 166.2 -> ceil 167
        let r = css_to_physical(&m, 10.5, 0.0, 100.3, 10.0).unwrap();
        assert_eq!(r, Rect::new(1920 + 15, 0, 152, 15));
    }

    #[test]
    fn css_to_physical_handles_negative_drag_and_clamps() {
        let m = mon(0, 0, 800, 600, 1.0);
        let r = css_to_physical(&m, 50.0, 50.0, -100.0, -100.0).unwrap();
        assert_eq!(r, Rect::new(0, 0, 50, 50));
        assert_eq!(css_to_physical(&m, 900.0, 0.0, 10.0, 10.0), None);
    }

    #[test]
    fn css_to_physical_on_negative_monitor_origin() {
        let m = mon(-1280, -200, 1280, 1024, 1.25);
        let r = css_to_physical(&m, 0.0, 0.0, 80.0, 40.0).unwrap();
        assert_eq!(r, Rect::new(-1280, -200, 100, 50));
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
