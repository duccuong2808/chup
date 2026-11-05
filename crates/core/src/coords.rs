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

// ---------- macOS: point <-> physical px toàn desktop (SPEC 11, thiết kế macOS mục 2.4) ----------

/// Hình chữ nhật theo point của macOS: gốc ở góc dưới-trái màn hình chính, trục y hướng lên.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectPt {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl RectPt {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
}

/// Một `NSScreen`: `frame`, `visibleFrame` (trừ menu bar và Dock), `backingScaleFactor`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenPt {
    pub key: String,
    pub frame: RectPt,
    pub visible: RectPt,
    pub scale: f64,
}

/// Bố cục desktop của macOS quy về physical px như Windows.
///
/// - Lật trục: `top = cao(màn hình chính) - (y + h)`.
/// - `s` = scale lớn nhất. Gốc màn hình (px) = gốc (point, đã lật) × `s`; cỡ (px) = cỡ (point) × scale của chính nó.
///   Hai màn hình không bao giờ chồng nhau trong toạ độ px; màn scale nhỏ hơn `s` để lại khoảng trống không có pixel
///   (vô hại: vùng chọn luôn nằm trong một màn hình, SPEC 2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MacLayout {
    pub s: f64,
    primary_h: f64,
    screens: Vec<(ScreenPt, MonitorInfo)>,
}

impl MacLayout {
    /// `screens[0]` là màn hình chính (thứ tự của `NSScreen.screens`).
    pub fn new(screens: &[ScreenPt]) -> Self {
        let s = screens.iter().map(|x| x.scale).fold(1.0, f64::max);
        let primary_h = screens.first().map_or(0.0, |x| x.frame.h);
        let mut out = MacLayout { s, primary_h, screens: vec![] };
        for (i, sc) in screens.iter().enumerate() {
            let rect = out.screen_px(&sc.frame, sc);
            let work = out.rect_in_screen(&sc.visible, sc, rect);
            let key = format!("{}@{},{}", sc.key, rect.x, rect.y);
            let mon = MonitorInfo { key, rect, work, scale: sc.scale, primary: i == 0 };
            out.screens.push((sc.clone(), mon));
        }
        out
    }

    pub fn monitors(&self) -> Vec<MonitorInfo> {
        self.screens.iter().map(|(_, m)| m.clone()).collect()
    }

    fn top(&self, r: &RectPt) -> f64 {
        self.primary_h - (r.y + r.h)
    }

    fn screen_px(&self, f: &RectPt, sc: &ScreenPt) -> Rect {
        let (x, y) = ((f.x * self.s).round() as i32, (self.top(f) * self.s).round() as i32);
        Rect::new(x, y, (f.w * sc.scale).round() as u32, (f.h * sc.scale).round() as u32)
    }

    /// Rect point `r` nằm trong màn hình `sc` (có rect px `mon`) -> px.
    fn rect_in_screen(&self, r: &RectPt, sc: &ScreenPt, mon: Rect) -> Rect {
        let k = sc.scale;
        let x = mon.x + ((r.x - sc.frame.x) * k).round() as i32;
        let y = mon.y + ((self.top(r) - self.top(&sc.frame)) * k).round() as i32;
        Rect::new(x, y, (r.w * k).round().max(0.0) as u32, (r.h * k).round().max(0.0) as u32)
    }

    /// Màn hình chứa điểm point (x, y) (trục y lên), hoặc gần nhất.
    fn screen_at_pt(&self, x: f64, y: f64) -> Option<&(ScreenPt, MonitorInfo)> {
        let inside = |f: &RectPt| x >= f.x && x < f.x + f.w && y >= f.y && y < f.y + f.h;
        let dist = |f: &RectPt| {
            let dx = (f.x - x).max(0.0).max(x - (f.x + f.w));
            let dy = (f.y - y).max(0.0).max(y - (f.y + f.h));
            dx * dx + dy * dy
        };
        self.screens.iter().find(|(sc, _)| inside(&sc.frame)).or_else(|| {
            self.screens.iter().min_by(|a, b| dist(&a.0.frame).total_cmp(&dist(&b.0.frame)))
        })
    }

    /// Màn hình chứa điểm px, hoặc gần nhất.
    fn screen_at_px(&self, x: i32, y: i32) -> Option<&(ScreenPt, MonitorInfo)> {
        let dist = |r: &Rect| {
            let dx = (r.x - x).max(0).max(x - r.right()) as i64;
            let dy = (r.y - y).max(0).max(y - r.bottom()) as i64;
            dx * dx + dy * dy
        };
        self.screens
            .iter()
            .find(|(_, m)| m.rect.contains_point(x, y))
            .or_else(|| self.screens.iter().min_by_key(|(_, m)| dist(&m.rect)))
    }

    /// Điểm point toàn cục (`NSEvent.mouseLocation`) -> px toàn desktop.
    pub fn pt_to_px(&self, x: f64, y: f64) -> (i32, i32) {
        let Some((sc, m)) = self.screen_at_pt(x, y) else { return (x.round() as i32, y.round() as i32) };
        let k = sc.scale;
        let px = m.rect.x + ((x - sc.frame.x) * k).floor() as i32;
        let py = m.rect.y + (((self.primary_h - y) - self.top(&sc.frame)) * k).floor() as i32;
        (px, py)
    }

    /// Rect px -> rect point (frame của `NSWindow`), theo màn hình chứa tâm của nó. Trả về cả scale của màn hình đó.
    pub fn px_to_pt(&self, r: Rect) -> (RectPt, f64) {
        let (cx, cy) = r.center();
        let Some((sc, m)) = self.screen_at_px(cx, cy) else {
            return (RectPt::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64), 1.0);
        };
        let k = sc.scale;
        let (w, h) = (r.w as f64 / k, r.h as f64 / k);
        let x = sc.frame.x + (r.x - m.rect.x) as f64 / k;
        let top = self.top(&sc.frame) + (r.y - m.rect.y) as f64 / k;
        (RectPt::new(x, self.primary_h - top - h, w, h), k)
    }

    /// Scale của màn hình chứa điểm px.
    pub fn scale_at_px(&self, x: i32, y: i32) -> f64 {
        self.screen_at_px(x, y).map_or(1.0, |(sc, _)| sc.scale)
    }
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

    fn screen(key: &str, frame: RectPt, menu: f64, scale: f64) -> ScreenPt {
        let visible = RectPt::new(frame.x, frame.y, frame.w, frame.h - menu);
        ScreenPt { key: key.into(), frame, visible, scale }
    }

    #[test]
    fn mac_single_retina() {
        let l = MacLayout::new(&[screen("built-in", RectPt::new(0.0, 0.0, 1512.0, 982.0), 33.0, 2.0)]);
        let m = &l.monitors()[0];
        assert_eq!(m.rect, Rect::new(0, 0, 3024, 1964));
        // Menu bar 33 pt ở trên: work area bắt đầu ở y = 66 px.
        assert_eq!(m.work, Rect::new(0, 66, 3024, 1898));
        assert!(m.primary);
        // Góc trên-trái của màn hình (point: y = 982) -> (0, 0); góc dưới-phải -> cuối màn hình.
        assert_eq!(l.pt_to_px(0.0, 981.75), (0, 0));
        assert_eq!(l.pt_to_px(1511.5, 0.25), (3023, 1963));
        // Rect px -> frame NSWindow: trục y lật lại.
        let (r, k) = l.px_to_pt(Rect::new(200, 100, 400, 300));
        assert_eq!((r, k), (RectPt::new(100.0, 982.0 - 50.0 - 150.0, 200.0, 150.0), 2.0));
    }

    #[test]
    fn mac_retina_with_external_1x_on_right_and_above() {
        let screens = [
            screen("built-in", RectPt::new(0.0, 0.0, 1512.0, 982.0), 33.0, 2.0),
            screen("dell", RectPt::new(1512.0, 0.0, 1920.0, 1080.0), 25.0, 1.0),
            screen("top", RectPt::new(0.0, 982.0, 1920.0, 1080.0), 25.0, 1.0),
        ];
        let l = MacLayout::new(&screens);
        assert_eq!(l.s, 2.0);
        let mons = l.monitors();
        // Màn phải: gốc = (1512, 982 - 1080) × 2 = (3024, -196); cỡ theo scale 1.
        assert_eq!(mons[1].rect, Rect::new(3024, -196, 1920, 1080));
        // Màn trên: gốc y = (982 - 2062) × 2.
        assert_eq!(mons[2].rect, Rect::new(0, -2160, 1920, 1080));
        // Không màn nào chồng lên màn khác.
        for (i, a) in mons.iter().enumerate() {
            for b in &mons[i + 1..] {
                assert_eq!(a.rect.intersect(&b.rect), None, "{} chồng {}", a.key, b.key);
            }
        }
        // Chuột trên màn phải, cách góc trên-trái (10, 20) pt -> (10, 20) px của màn đó.
        assert_eq!(l.pt_to_px(1522.0, 1060.0), (3034, -176));
        // Ngược lại: cửa sổ trên màn phải.
        let (r, k) = l.px_to_pt(Rect::new(3034, -176, 100, 50));
        assert_eq!((r, k), (RectPt::new(1522.0, 1010.0, 100.0, 50.0), 1.0));
    }

    #[test]
    fn mac_primary_external_at_left_of_retina() {
        // Màn chính 1x, Retina nằm bên trái (gốc âm).
        let screens = [
            screen("dell", RectPt::new(0.0, 0.0, 1920.0, 1080.0), 25.0, 1.0),
            screen("built-in", RectPt::new(-1512.0, 0.0, 1512.0, 982.0), 33.0, 2.0),
        ];
        let mons = MacLayout::new(&screens).monitors();
        assert_eq!(mons[0].rect, Rect::new(0, 0, 1920, 1080));
        assert_eq!(mons[1].rect, Rect::new(-3024, 196, 3024, 1964));
        assert_eq!(mons[0].rect.intersect(&mons[1].rect), None);
    }
}
