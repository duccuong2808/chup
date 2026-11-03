//! Nhận khung chữ nhật từ ảnh chụp màn hình (SPEC 16.3 nguồn 3), theo điểm con trỏ. Không dùng ML.
//! Kết quả spike: docs/plans/2026-11-03-autoselect-spike.md mục 3.
//!
//! 1. Bản đồ cạnh: cạnh ngang ở (x, y) khi kênh màu lệch nhiều nhất giữa pixel (x, y) và (x, y-1) ≥ ngưỡng;
//!    cạnh dọc tương tự với (x-1, y). Tổng tích luỹ theo hàng / cột (u16) để biết một đoạn liền cạnh bao nhiêu %.
//! 2. Với điểm p: lấy vài hàng gần nhất phía trên / dưới có cạnh ngang liền quanh p, vài cột trái / phải tương tự,
//!    mép vùng `within` luôn là ứng viên. Giữ hình nhỏ nhất có cả 4 cạnh liền, rồi nới ra ôm đường viền mảnh.
//!
//! Toạ độ là pixel của ảnh (lớp platform tự đổi sang toạ độ desktop).

use crate::coords::Rect;

/// Ngưỡng lệch màu (0–255). Spike: 24 bỏ lỡ viền nhạt của giao diện sáng, 12 bắt được.
pub const THRESHOLD: u8 = 12;
/// Số ứng viên mỗi phía.
const K: usize = 8;
/// Nửa bề rộng đoạn quanh con trỏ dùng để chọn ứng viên.
const LOCAL: i32 = 12;
/// Tỉ lệ liền tối thiểu: đoạn quanh con trỏ / cả cạnh của khung.
const LOCAL_COV: f32 = 0.9;
const SIDE_COV: f32 = 0.85;
/// Cạnh khung tối thiểu (px).
const MIN_SIDE: i32 = 16;
/// Đường viền dày tối đa chừng này thì nới khung ra ôm luôn.
const BORDER_MAX: i32 = 2;

pub struct EdgeMap {
    w: usize,
    h: usize,
    /// hp[y * (w + 1) + x] = số cạnh ngang ở hàng y, cột < x.
    hp: Vec<u16>,
    /// vp[x * (h + 1) + y] = số cạnh dọc ở cột x, hàng < y.
    vp: Vec<u16>,
}

impl EdgeMap {
    /// `px`: 4 byte mỗi pixel (RGBA hay BGRA đều được, bỏ qua kênh thứ 4), hàng liền nhau, rộng `w`.
    pub fn build(px: &[u8], w: u32, h: u32, threshold: u8) -> Self {
        let (w, h) = (w as usize, h as usize);
        assert!(px.len() >= w * h * 4, "ảnh thiếu pixel");
        let differs = |a: usize, b: usize| {
            let (p, q) = (&px[a * 4..a * 4 + 3], &px[b * 4..b * 4 + 3]);
            p[0].abs_diff(q[0]).max(p[1].abs_diff(q[1])).max(p[2].abs_diff(q[2])) >= threshold
        };
        let mut hp = vec![0u16; h * (w + 1)];
        for y in 1..h {
            let row = &mut hp[y * (w + 1)..(y + 1) * (w + 1)];
            for x in 0..w {
                row[x + 1] = row[x].saturating_add(differs(y * w + x, (y - 1) * w + x) as u16);
            }
        }
        let mut vp = vec![0u16; w * (h + 1)];
        for x in 1..w {
            let col = &mut vp[x * (h + 1)..(x + 1) * (h + 1)];
            for y in 0..h {
                col[y + 1] = col[y].saturating_add(differs(y * w + x, y * w + x - 1) as u16);
            }
        }
        Self { w, h, hp, vp }
    }

    /// Bộ nhớ đang giữ (byte), để ghi log.
    pub fn bytes(&self) -> usize {
        (self.hp.len() + self.vp.len()) * 2
    }

    /// Tỉ lệ liền cạnh ngang ở ranh giới `y` (giữa hàng y-1 và y), cột [x0, x1).
    fn hcov(&self, y: i32, x0: i32, x1: i32) -> f32 {
        if y <= 0 || y >= self.h as i32 {
            return 0.0;
        }
        let (x0, x1) = (x0.clamp(0, self.w as i32) as usize, x1.clamp(0, self.w as i32) as usize);
        if x1 <= x0 {
            return 0.0;
        }
        let row = &self.hp[y as usize * (self.w + 1)..];
        (row[x1] - row[x0]) as f32 / (x1 - x0) as f32
    }

    fn vcov(&self, x: i32, y0: i32, y1: i32) -> f32 {
        if x <= 0 || x >= self.w as i32 {
            return 0.0;
        }
        let (y0, y1) = (y0.clamp(0, self.h as i32) as usize, y1.clamp(0, self.h as i32) as usize);
        if y1 <= y0 {
            return 0.0;
        }
        let col = &self.vp[x as usize * (self.h + 1)..];
        (col[y1] - col[y0]) as f32 / (y1 - y0) as f32
    }

    /// Khung nhỏ nhất chứa `p`, nằm trong `within` (mép `within` coi như cạnh liền).
    /// `None` nếu không tìm được, hoặc khung tìm được chính là `within` (không thêm cấp nào).
    pub fn query(&self, p: (i32, i32), within: Rect) -> Option<Rect> {
        self.query_around(p, within, None)
    }

    /// Các khung lồng nhau chứa `p` trong `within`, từ trong ra ngoài (tối đa `max`): khung nhỏ nhất, rồi khung nhỏ
    /// nhất bao trọn khung đó, … Cho ↑↓ đi qua từng lớp (thẻ, khối, cột) như khi rê chuột thấy (chup-4q9).
    pub fn ladder(&self, p: (i32, i32), within: Rect, max: usize) -> Vec<Rect> {
        let mut out: Vec<Rect> = Vec::new();
        while out.len() < max {
            match self.query_around(p, within, out.last().copied()) {
                Some(r) => out.push(r),
                None => break,
            }
        }
        out
    }

    /// Như `query`; có `inner` thì chỉ xét khung bao trọn `inner` (cạnh nằm ngoài cả đường viền của nó).
    fn query_around(&self, p: (i32, i32), within: Rect, inner: Option<Rect>) -> Option<Rect> {
        let img = Rect::new(0, 0, self.w as u32, self.h as u32);
        let within = within.intersect(&img)?;
        if !within.contains_point(p.0, p.1) {
            return None;
        }
        let (l, t, r, b) = (within.x, within.y, within.right(), within.bottom());
        let (px, py) = p;
        // Mép `within` luôn liền.
        let hc = |y: i32, x0: i32, x1: i32| if y == t || y == b { 1.0 } else { self.hcov(y, x0, x1) };
        let vc = |x: i32, y0: i32, y1: i32| if x == l || x == r { 1.0 } else { self.vcov(x, y0, y1) };
        let hok = |y: i32| self.hcov(y, px - LOCAL, px + LOCAL + 1) >= LOCAL_COV;
        let vok = |x: i32| self.vcov(x, py - LOCAL, py + LOCAL + 1) >= LOCAL_COV;
        let pick = |it: &mut dyn Iterator<Item = i32>, ok: &dyn Fn(i32) -> bool, edge: i32| {
            let mut v: Vec<i32> = it.filter(|&i| ok(i)).take(K).collect();
            v.push(edge);
            v
        };
        // Có `inner`: cạnh phải nằm ngoài nó, cách hơn bề dày đường viền (không lấy mặt ngoài của chính viền đó).
        let gap = BORDER_MAX + 1;
        let (top_max, bot_min, left_max, right_min) = match inner {
            Some(i) => (i.y - gap, i.bottom() + gap, i.x - gap, i.right() + gap),
            None => (py, py + 1, px, px + 1),
        };
        let tops = pick(&mut (t + 1..=top_max).rev(), &hok, t);
        let bots = pick(&mut (bot_min..b), &hok, b);
        let lefts = pick(&mut (l + 1..=left_max).rev(), &vok, l);
        let rights = pick(&mut (right_min..r), &vok, r);

        let mut best: Option<(i32, i32, i32, i32)> = None;
        let area = |(x0, y0, x1, y1): (i32, i32, i32, i32)| (x1 - x0) as i64 * (y1 - y0) as i64;
        for &y0 in &tops {
            for &y1 in &bots {
                if y1 - y0 < MIN_SIDE {
                    continue;
                }
                for &x0 in &lefts {
                    for &x1 in &rights {
                        if x1 - x0 < MIN_SIDE {
                            continue;
                        }
                        let c = (x0, y0, x1, y1);
                        if best.is_some_and(|bst| area(bst) <= area(c)) {
                            continue;
                        }
                        if hc(y0, x0, x1) >= SIDE_COV
                            && hc(y1, x0, x1) >= SIDE_COV
                            && vc(x0, y0, y1) >= SIDE_COV
                            && vc(x1, y0, y1) >= SIDE_COV
                        {
                            best = Some(c);
                        }
                    }
                }
            }
        }
        let (mut x0, mut y0, mut x1, mut y1) = best?;
        // Nới ra ôm đường viền mảnh: đường viền 1 px cho 2 ranh giới cạnh nhau (trước và sau nét).
        // Chỉ nới khi ranh giới bên ngoài cũng liền cạnh cùng độ dài.
        let grow = |start: i32, dir: i32, ok: &dyn Fn(i32) -> bool, limit: i32| {
            (1..=BORDER_MAX)
                .map(|d| start + dir * d)
                .take_while(|&v| if dir < 0 { v >= limit } else { v <= limit })
                .filter(|&v| ok(v))
                .last()
                .unwrap_or(start)
        };
        let (cx0, cy0, cx1, cy1) = (x0, y0, x1, y1);
        if cy0 != t {
            y0 = grow(cy0, -1, &|y| hc(y, cx0, cx1) >= SIDE_COV, t);
        }
        if cy1 != b {
            y1 = grow(cy1, 1, &|y| hc(y, cx0, cx1) >= SIDE_COV, b);
        }
        if cx0 != l {
            x0 = grow(cx0, -1, &|x| vc(x, cy0, cy1) >= SIDE_COV, l);
        }
        if cx1 != r {
            x1 = grow(cx1, 1, &|x| vc(x, cy0, cy1) >= SIDE_COV, r);
        }
        let out = Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32);
        let same = (out.x - within.x).abs() <= 2
            && (out.y - within.y).abs() <= 2
            && (out.right() - within.right()).abs() <= 2
            && (out.bottom() - within.bottom()).abs() <= 2;
        (!same).then_some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ảnh RGBA nền trắng.
    struct Img {
        w: u32,
        h: u32,
        px: Vec<u8>,
    }

    impl Img {
        fn new(w: u32, h: u32) -> Self {
            Self { w, h, px: vec![255; (w * h * 4) as usize] }
        }
        fn set(&mut self, x: i32, y: i32, v: u8) {
            let i = ((y as u32 * self.w + x as u32) * 4) as usize;
            self.px[i..i + 3].fill(v);
        }
        fn fill(&mut self, r: Rect, v: u8) {
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    self.set(x, y, v);
                }
            }
        }
        /// Đường viền dày `t` px nằm trong `r`.
        fn border(&mut self, r: Rect, t: i32, v: u8) {
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    let inner = x >= r.x + t && x < r.right() - t && y >= r.y + t && y < r.bottom() - t;
                    if !inner {
                        self.set(x, y, v);
                    }
                }
            }
        }
        fn map(&self) -> EdgeMap {
            EdgeMap::build(&self.px, self.w, self.h, THRESHOLD)
        }
        fn all(&self) -> Rect {
            Rect::new(0, 0, self.w, self.h)
        }
    }

    #[test]
    fn finds_box_including_1px_border() {
        let mut img = Img::new(400, 300);
        let card = Rect::new(50, 40, 200, 160);
        img.border(card, 1, 200);
        assert_eq!(img.map().query((120, 100), img.all()), Some(card));
    }

    #[test]
    fn finds_box_including_2px_border() {
        let mut img = Img::new(400, 300);
        let card = Rect::new(50, 40, 200, 160);
        img.border(card, 2, 120);
        assert_eq!(img.map().query((120, 100), img.all()), Some(card));
    }

    #[test]
    fn filled_block_without_border() {
        let mut img = Img::new(400, 300);
        let block = Rect::new(60, 50, 150, 100);
        img.fill(block, 235);
        assert_eq!(img.map().query((100, 90), img.all()), Some(block));
    }

    #[test]
    fn nested_boxes_pick_smallest_containing() {
        let mut img = Img::new(500, 400);
        let outer = Rect::new(20, 20, 460, 360);
        let inner = Rect::new(100, 100, 120, 80);
        img.border(outer, 1, 180);
        img.border(inner, 1, 180);
        let m = img.map();
        assert_eq!(m.query((150, 130), img.all()), Some(inner));
        assert_eq!(m.query((350, 300), img.all()), Some(outer));
    }

    #[test]
    fn ladder_goes_through_every_nested_box() {
        // Cột > thẻ > ảnh trong thẻ, như trang web: ↑↓ phải đi qua cả ba.
        let mut img = Img::new(600, 500);
        let column = Rect::new(20, 20, 520, 440);
        let card = Rect::new(60, 60, 300, 240);
        let photo = Rect::new(90, 90, 160, 100);
        img.border(column, 1, 190);
        img.border(card, 1, 170);
        img.border(photo, 2, 120);
        let m = img.map();
        assert_eq!(m.ladder((150, 130), img.all(), 6), vec![photo, card, column]);
        // Trong thẻ nhưng ngoài ảnh: thẻ rồi cột.
        assert_eq!(m.ladder((300, 250), img.all(), 6), vec![card, column]);
        assert_eq!(m.ladder((150, 130), img.all(), 2), vec![photo, card], "giới hạn số lớp");
        // Trong `within` = thẻ: chỉ còn ảnh (khung bằng `within` không tính).
        assert_eq!(m.ladder((150, 130), card, 6), vec![photo]);
        assert!(Img::new(300, 200).map().ladder((100, 100), Rect::new(0, 0, 300, 200), 6).is_empty());
    }

    #[test]
    fn faint_border_detected_at_threshold_too_faint_not() {
        let mut img = Img::new(400, 300);
        let card = Rect::new(50, 40, 200, 160);
        img.border(card, 1, 255 - 14);
        assert_eq!(img.map().query((120, 100), img.all()), Some(card));
        let mut img = Img::new(400, 300);
        img.border(card, 1, 255 - 8);
        assert_eq!(img.map().query((120, 100), img.all()), None);
    }

    #[test]
    fn short_strokes_like_text_are_not_boxes() {
        let mut img = Img::new(400, 300);
        let card = Rect::new(20, 20, 360, 260);
        img.border(card, 1, 200);
        // "Chữ": nét ngang và dọc ngắn 6–10 px rải trong thẻ.
        for row in 0..8 {
            for col in 0..20 {
                let (x, y) = (40 + col * 16, 50 + row * 24);
                img.fill(Rect::new(x, y, 10, 1), 0);
                img.fill(Rect::new(x, y, 1, 8), 0);
                img.fill(Rect::new(x + 9, y, 1, 8), 0);
            }
        }
        assert_eq!(img.map().query((45, 55), img.all()), Some(card));
    }

    #[test]
    fn within_limits_search_and_same_as_within_is_none() {
        let mut img = Img::new(400, 300);
        let card = Rect::new(50, 40, 200, 160);
        img.border(card, 1, 200);
        let m = img.map();
        // Phần tử UIA sâu nhất chính là thẻ: không có cấp nào mới.
        assert_eq!(m.query((120, 100), card), None);
        // `within` nhỏ hơn thẻ: không được trả khung vượt ra ngoài `within`.
        let small = Rect::new(60, 50, 100, 80);
        assert!(m.query((120, 100), small).is_none_or(|r| small.intersect(&r) == Some(r)));
        // Điểm ngoài `within`.
        assert_eq!(m.query((10, 10), card), None);
    }

    #[test]
    fn plain_image_gives_none() {
        let img = Img::new(200, 100);
        assert_eq!(img.map().query((50, 50), img.all()), None);
    }

    /// Chạy tay: `cargo test --release -p chup-core edges -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn build_4k_time() {
        let mut img = Img::new(3840, 2160);
        for i in 0..40 {
            img.border(Rect::new((i * 90) % 3200, (i * 50) % 1700, 600, 400), 1, 180);
        }
        let t = std::time::Instant::now();
        let m = img.map();
        let build = t.elapsed();
        let t = std::time::Instant::now();
        for y in (10..2160).step_by(80) {
            for x in (10..3840).step_by(80) {
                m.query((x, y), img.all());
            }
        }
        println!("4K: dựng {build:?}, {} MB, 1296 truy vấn {:?}", m.bytes() / 1_000_000, t.elapsed());
    }
}
