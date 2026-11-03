//! Spike SPEC 16.6, nguồn 3: nhận diện khung chữ nhật từ ảnh chụp màn hình, theo điểm con trỏ.
//!
//! Thuật toán (không dùng ML):
//! 1. Bản đồ cạnh: cạnh ngang ở (x, y) khi pixel (x, y) khác (x, y-1) quá ngưỡng (kênh lệch nhiều nhất);
//!    cạnh dọc tương tự với (x-1, y). Tổng tích luỹ theo hàng / cột để hỏi "đoạn này liền cạnh bao nhiêu %" trong O(1).
//! 2. Với điểm p: lấy K hàng gần nhất phía trên có cạnh ngang liền quanh p.x (±12 px), K hàng phía dưới,
//!    K cột trái / phải tương tự; mép ảnh luôn là ứng viên. Thử mọi tổ hợp, giữ hình nhỏ nhất có cả 4 cạnh
//!    liền ≥ 85% và kích thước ≥ 16 px.
//!
//! Chạy: `edge-rects <thư mục ảnh> <thư mục ra> [ngưỡng ...]`. Mỗi `X.png` cần `X.uia.txt` (khung UIA làm đáp án
//! tham khảo, do `autoselect-spike --capture` tạo).

use std::time::{Duration, Instant};

use image::{imageops, Rgb, RgbImage};

#[derive(Clone, Copy, Debug, PartialEq)]
struct R {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl R {
    fn w(&self) -> i32 {
        self.x1 - self.x0
    }
    fn h(&self) -> i32 {
        self.y1 - self.y0
    }
    fn area(&self) -> i64 {
        self.w() as i64 * self.h() as i64
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
    fn iou(&self, o: &R) -> f64 {
        let ix = (self.x1.min(o.x1) - self.x0.max(o.x0)).max(0) as i64;
        let iy = (self.y1.min(o.y1) - self.y0.max(o.y0)).max(0) as i64;
        let i = ix * iy;
        i as f64 / (self.area() + o.area() - i) as f64
    }
}

struct Edges {
    w: usize,
    h: usize,
    /// hp[y * (w + 1) + x] = số cạnh ngang ở hàng y, cột < x.
    hp: Vec<u32>,
    /// vp[x * (h + 1) + y] = số cạnh dọc ở cột x, hàng < y.
    vp: Vec<u32>,
}

impl Edges {
    fn build(img: &RgbImage, t: u8) -> Self {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let px = img.as_raw();
        let diff = |a: usize, b: usize| {
            let (p, q) = (&px[a * 3..a * 3 + 3], &px[b * 3..b * 3 + 3]);
            (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap() >= t
        };
        let mut hp = vec![0u32; h * (w + 1)];
        for y in 1..h {
            let row = &mut hp[y * (w + 1)..(y + 1) * (w + 1)];
            for x in 0..w {
                row[x + 1] = row[x] + diff(y * w + x, (y - 1) * w + x) as u32;
            }
        }
        let mut vp = vec![0u32; w * (h + 1)];
        for x in 1..w {
            let col = &mut vp[x * (h + 1)..(x + 1) * (h + 1)];
            for y in 0..h {
                col[y + 1] = col[y] + diff(y * w + x, y * w + x - 1) as u32;
            }
        }
        Self { w, h, hp, vp }
    }

    /// Tỉ lệ liền cạnh ngang ở ranh giới y, cột [x0, x1). Mép ảnh coi như liền.
    fn hcov(&self, y: i32, x0: i32, x1: i32) -> f32 {
        if y <= 0 || y >= self.h as i32 {
            return 1.0;
        }
        let (x0, x1) = (x0.max(0) as usize, (x1.min(self.w as i32)).max(0) as usize);
        if x1 <= x0 {
            return 0.0;
        }
        let row = &self.hp[y as usize * (self.w + 1)..];
        (row[x1] - row[x0]) as f32 / (x1 - x0) as f32
    }

    fn vcov(&self, x: i32, y0: i32, y1: i32) -> f32 {
        if x <= 0 || x >= self.w as i32 {
            return 1.0;
        }
        let (y0, y1) = (y0.max(0) as usize, (y1.min(self.h as i32)).max(0) as usize);
        if y1 <= y0 {
            return 0.0;
        }
        let col = &self.vp[x as usize * (self.h + 1)..];
        (col[y1] - col[y0]) as f32 / (y1 - y0) as f32
    }

    fn query(&self, px: i32, py: i32) -> Option<R> {
        const K: usize = 8;
        const LOCAL: i32 = 12;
        const MIN_COV: f32 = 0.85;
        const MIN_SIDE: i32 = 16;
        let (w, h) = (self.w as i32, self.h as i32);
        let pick = |range: &mut dyn Iterator<Item = i32>, ok: &dyn Fn(i32) -> bool, edge: i32| {
            let mut v: Vec<i32> = range.filter(|&i| ok(i)).take(K).collect();
            if v.len() < K {
                v.push(edge);
            }
            v
        };
        let hok = |y: i32| self.hcov(y, px - LOCAL, px + LOCAL + 1) >= 0.9;
        let vok = |x: i32| self.vcov(x, py - LOCAL, py + LOCAL + 1) >= 0.9;
        let tops = pick(&mut (1..=py).rev(), &hok, 0);
        let bots = pick(&mut (py + 1..h), &hok, h);
        let lefts = pick(&mut (1..=px).rev(), &vok, 0);
        let rights = pick(&mut (px + 1..w), &vok, w);
        let mut best: Option<R> = None;
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
                        let r = R { x0, y0, x1, y1 };
                        if best.is_some_and(|b| b.area() <= r.area()) {
                            continue;
                        }
                        if self.hcov(y0, x0, x1) >= MIN_COV
                            && self.hcov(y1, x0, x1) >= MIN_COV
                            && self.vcov(x0, y0, y1) >= MIN_COV
                            && self.vcov(x1, y0, y1) >= MIN_COV
                        {
                            best = Some(r);
                        }
                    }
                }
            }
        }
        best
    }
}

fn load_uia(path: &str) -> Vec<R> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<i32> = l.split_whitespace().skip(1).filter_map(|s| s.parse().ok()).collect();
            (f.len() >= 4).then(|| R { x0: f[0], y0: f[1], x1: f[0] + f[2], y1: f[1] + f[3] })
        })
        .collect()
}

#[derive(Default)]
struct Score {
    points: u32,
    found: u32,
    whole: u32,
    matched: u32,
    uia_inner: u32,
    uia_inner_hit: u32,
    query: Duration,
    query_max: Duration,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (src, out) = (&args[1], &args[2]);
    let ts: Vec<u8> = args[3..].iter().filter_map(|s| s.parse().ok()).collect();
    let ts = if ts.is_empty() { vec![12, 24] } else { ts };
    let _ = std::fs::create_dir_all(out);
    let mut files: Vec<String> = std::fs::read_dir(src)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".png"))
        .collect();
    files.sort();

    println!("Điểm thử: lưới bước 40 px. \"khớp\" = IoU ≥ 0.85 với một khung UIA chứa điểm.");
    println!("\"UIA trong\" = điểm có khung UIA nhỏ hơn cả cửa sổ; \"bắt được\" = trong số đó, ảnh ra khung khớp.\n");
    println!("| ảnh | ngưỡng | dựng bản đồ cạnh | điểm | ra khung | = cả ảnh | khớp UIA | UIA trong: bắt được | truy vấn TB / max |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for f in &files {
        let stem = f.trim_end_matches(".png");
        let img = image::open(format!("{src}/{f}")).unwrap().to_rgb8();
        let uia = load_uia(&format!("{src}/{stem}.uia.txt"));
        let whole = R { x0: 0, y0: 0, x1: img.width() as i32, y1: img.height() as i32 };
        for &t in &ts {
            let t0 = Instant::now();
            let e = Edges::build(&img, t);
            let build = t0.elapsed();
            let mut s = Score::default();
            let mut viz = img.clone();
            for py in (20..img.height() as i32).step_by(40) {
                for px in (20..img.width() as i32).step_by(40) {
                    s.points += 1;
                    let t1 = Instant::now();
                    let d = e.query(px, py);
                    let dt = t1.elapsed();
                    s.query += dt;
                    s.query_max = s.query_max.max(dt);
                    let gs: Vec<&R> = uia.iter().filter(|g| g.contains(px, py)).collect();
                    let inner = gs.iter().any(|g| g.iou(&whole) < 0.85);
                    s.uia_inner += inner as u32;
                    let Some(d) = d else { continue };
                    s.found += 1;
                    let is_whole = d.iou(&whole) >= 0.85;
                    s.whole += is_whole as u32;
                    let ok = gs.iter().any(|g| g.iou(&d) >= 0.85);
                    s.matched += ok as u32;
                    s.uia_inner_hit += (inner && ok && !is_whole) as u32;
                    if px % 160 == 20 && py % 160 == 20 && !is_whole {
                        let c = if ok { Rgb([0, 170, 0]) } else { Rgb([230, 0, 0]) };
                        draw_rect(&mut viz, &d, c);
                        draw_dot(&mut viz, px, py, c);
                    }
                }
            }
            let _ = viz.save(format!("{out}/{stem}-t{t}.png"));
            println!(
                "| {stem} | {t} | {:.0} ms | {} | {} | {} | {} | {} / {} | {:.0} µs / {:.1} ms |",
                build.as_secs_f64() * 1e3,
                s.points,
                pct(s.found, s.points),
                pct(s.whole, s.found),
                pct(s.matched, s.found),
                s.uia_inner_hit,
                s.uia_inner,
                s.query.as_secs_f64() * 1e6 / s.points as f64,
                s.query_max.as_secs_f64() * 1e3,
            );
        }
    }

    // Thời gian ở 4K: phóng ảnh đầu tiên lên 3840 px rộng.
    if let Some(f) = files.first() {
        let img = image::open(format!("{src}/{f}")).unwrap().to_rgb8();
        let k = 3840.0 / img.width() as f64;
        let big = imageops::resize(&img, 3840, (img.height() as f64 * k) as u32, imageops::FilterType::Nearest);
        let t0 = Instant::now();
        let e = Edges::build(&big, ts[0]);
        let build = t0.elapsed();
        let t1 = Instant::now();
        let mut n = 0;
        for py in (20..big.height() as i32).step_by(80) {
            for px in (20..big.width() as i32).step_by(80) {
                e.query(px, py);
                n += 1;
            }
        }
        println!(
            "\n4K ({}x{}, từ {f}): dựng bản đồ cạnh {:.0} ms, bộ nhớ {:.0} MB, truy vấn TB {:.0} µs.",
            big.width(),
            big.height(),
            build.as_secs_f64() * 1e3,
            ((e.hp.len() + e.vp.len()) * 4) as f64 / 1e6,
            t1.elapsed().as_secs_f64() * 1e6 / n as f64
        );
    }
}

fn pct(a: u32, b: u32) -> String {
    if b == 0 {
        return "-".into();
    }
    format!("{:.0}%", a as f64 * 100.0 / b as f64)
}

fn draw_rect(img: &mut RgbImage, r: &R, c: Rgb<u8>) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    for k in 0..2 {
        for x in r.x0..r.x1 {
            for y in [r.y0 + k, r.y1 - 1 - k] {
                if (0..w).contains(&x) && (0..h).contains(&y) {
                    img.put_pixel(x as u32, y as u32, c);
                }
            }
        }
        for y in r.y0..r.y1 {
            for x in [r.x0 + k, r.x1 - 1 - k] {
                if (0..w).contains(&x) && (0..h).contains(&y) {
                    img.put_pixel(x as u32, y as u32, c);
                }
            }
        }
    }
}

fn draw_dot(img: &mut RgbImage, px: i32, py: i32, c: Rgb<u8>) {
    for y in py - 3..=py + 3 {
        for x in px - 3..=px + 3 {
            if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                img.put_pixel(x as u32, y as u32, c);
            }
        }
    }
}
