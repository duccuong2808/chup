//! Thao tác pixel trực tiếp trên surface BGRA premultiplied, top-down (overlay, frame: phủ cả màn hình nên
//! rẻ hơn Direct2D / CoreGraphics nhiều ở 4K). `Pixels` chỉ là view, bộ nhớ thuộc surface của vỏ OS.

use chup_core::coords::Rect;
use image::RgbaImage;

pub struct Pixels<'a> {
    pub data: &'a mut [u32],
    pub w: i32,
    pub h: i32,
}

/// View chỉ đọc (nguồn để blend: sprite chữ, toolbar vẽ sẵn).
#[derive(Clone, Copy)]
pub struct PixelsRef<'a> {
    pub data: &'a [u32],
    pub w: i32,
    pub h: i32,
}

impl<'a> Pixels<'a> {
    pub fn new(data: &'a mut [u32], w: i32, h: i32) -> Self {
        debug_assert!(data.len() >= (w.max(0) * h.max(0)) as usize);
        Self { data, w, h }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w.max(0) as u32, self.h.max(0) as u32)
    }

    /// Tô `r` bằng 1 pixel premultiplied, cắt theo biên.
    pub fn fill(&mut self, r: Rect, px: u32) {
        let Some(r) = r.intersect(&self.bounds()) else {
            return;
        };
        let w = self.w as usize;
        for y in r.y..r.bottom() {
            let row = y as usize * w;
            self.data[row + r.x as usize..row + r.right() as usize].fill(px);
        }
    }

    /// Viền hình chữ nhật dày `t` nằm **bên ngoài** `r`.
    pub fn outline_outside(&mut self, r: Rect, t: i32, px: u32) {
        let o = r.expand(t);
        self.fill(Rect::new(o.x, o.y, o.w, t as u32), px);
        self.fill(Rect::new(o.x, r.bottom(), o.w, t as u32), px);
        self.fill(Rect::new(o.x, r.y, t as u32, r.h), px);
        self.fill(Rect::new(r.right(), r.y, t as u32, r.h), px);
    }

    /// Viền nét đứt dày `t` nằm bên ngoài `r`: các đoạn dài `dash` xen kẽ màu `a` và `b`
    /// (2 màu tương phản để nhìn rõ trên mọi nền).
    pub fn outline_dashed_outside(&mut self, r: Rect, t: i32, dash: i32, a: u32, b: u32) {
        let o = r.expand(t);
        let dash = dash.max(1);
        let (mut x, mut i) = (o.x, 0);
        while x < o.right() {
            let len = dash.min(o.right() - x) as u32;
            let c = if i % 2 == 0 { a } else { b };
            self.fill(Rect::new(x, o.y, len, t as u32), c);
            self.fill(Rect::new(x, r.bottom(), len, t as u32), c);
            x += dash;
            i += 1;
        }
        let (mut y, mut i) = (r.y, 1);
        while y < r.bottom() {
            let len = dash.min(r.bottom() - y) as u32;
            let c = if i % 2 == 0 { a } else { b };
            self.fill(Rect::new(o.x, y, t as u32, len), c);
            self.fill(Rect::new(r.right(), y, t as u32, len), c);
            y += dash;
            i += 1;
        }
    }

    /// Chép góc trên-trái cỡ `size` của `src` lên vị trí `at` bằng phép "over", chỉ trong `clip`.
    ///
    /// `clip` là vùng vừa được tô lại (dirty): "over" không lặp lại được, nên blend ra ngoài vùng đó là chồng lên
    /// pixel đã blend lần trước (sprite bán trong suốt đậm dần) (chup-71a.12).
    pub fn blend_part(&mut self, src: PixelsRef, size: (i32, i32), at: (i32, i32), clip: Rect) {
        let (x, y) = at;
        let (pw, ph) = (size.0.min(src.w), size.1.min(src.h));
        if pw <= 0 || ph <= 0 {
            return;
        }
        let Some(area) = Rect::new(x, y, pw as u32, ph as u32)
            .intersect(&self.bounds())
            .and_then(|a| a.intersect(&clip))
        else {
            return;
        };
        let (sw, dw) = (src.w as usize, self.w as usize);
        for yy in area.y..area.bottom() {
            for xx in area.x..area.right() {
                let sp = src.data[(yy - y) as usize * sw + (xx - x) as usize];
                let sa = sp >> 24;
                if sa == 0 {
                    continue;
                }
                let di = yy as usize * dw + xx as usize;
                self.data[di] = if sa == 255 {
                    sp
                } else {
                    over(sp, self.data[di])
                };
            }
        }
    }

    /// Chép vùng `r` từ `src` cùng kích thước.
    pub fn copy_rect_from(&mut self, src: &[u32], r: Rect) {
        let Some(r) = r.intersect(&self.bounds()) else {
            return;
        };
        if src.len() < self.data.len() {
            return;
        }
        let w = self.w as usize;
        let (x0, x1) = (r.x as usize, r.right() as usize);
        for y in r.y as usize..r.bottom() as usize {
            self.data[y * w + x0..y * w + x1].copy_from_slice(&src[y * w + x0..y * w + x1]);
        }
    }
}

/// src over dst, cả hai premultiplied.
pub fn over(s: u32, d: u32) -> u32 {
    let inv = 255 - (s >> 24);
    let ch = |shift: u32| {
        let sc = (s >> shift) & 0xFF;
        let dc = (d >> shift) & 0xFF;
        (sc + (dc * inv + 127) / 255).min(255) << shift
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// Ảnh RGBA thẳng -> pixel BGRA premultiplied (u32 = 0xAARRGGBB), để tải lên Direct2D / CoreGraphics.
pub fn premultiply(img: &RgbaImage) -> Vec<u32> {
    img.pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let pm = |c: u8| (c as u32 * a as u32 + 127) / 255;
            ((a as u32) << 24) | (pm(r) << 16) | (pm(g) << 8) | pm(b)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(w: i32, h: i32) -> Vec<u32> {
        vec![0; (w * h) as usize]
    }

    #[test]
    fn fill_clips_to_bounds() {
        let mut b = buf(4, 3);
        let mut p = Pixels::new(&mut b, 4, 3);
        p.fill(Rect::new(-2, 1, 4, 10), 7);
        assert_eq!(b, vec![0, 0, 0, 0, 7, 7, 0, 0, 7, 7, 0, 0]);
    }

    #[test]
    fn outline_is_outside_rect() {
        let mut b = buf(5, 5);
        let mut p = Pixels::new(&mut b, 5, 5);
        p.outline_outside(Rect::new(1, 1, 3, 3), 1, 1);
        // Bên trong không bị tô.
        for y in 1..4 {
            for x in 1..4 {
                assert_eq!(b[y * 5 + x], 0);
            }
        }
        assert_eq!(b.iter().filter(|&&v| v == 1).count(), 16);
    }

    #[test]
    fn blend_over_respects_clip_and_alpha() {
        let mut d = vec![0xFF00_0000u32; 4];
        let src = [0x8080_8080u32, 0xFFFF_FFFF, 0, 0x8080_8080];
        let mut p = Pixels::new(&mut d, 2, 2);
        p.blend_part(
            PixelsRef {
                data: &src,
                w: 2,
                h: 2,
            },
            (2, 2),
            (0, 0),
            Rect::new(0, 0, 2, 1),
        );
        assert_eq!(d[0], over(0x8080_8080, 0xFF00_0000));
        assert_eq!(d[1], 0xFFFF_FFFF);
        // Ngoài clip giữ nguyên.
        assert_eq!(d[3], 0xFF00_0000);
    }

    #[test]
    fn premultiply_halves_channels() {
        let img = RgbaImage::from_raw(1, 1, vec![255, 128, 0, 128]).unwrap();
        assert_eq!(premultiply(&img), vec![0x8080_4000]);
    }

    #[test]
    fn copy_rect_copies_only_rect() {
        let src: Vec<u32> = (0..9).collect();
        let mut d = vec![0u32; 9];
        Pixels::new(&mut d, 3, 3).copy_rect_from(&src, Rect::new(1, 1, 2, 1));
        assert_eq!(d, vec![0, 0, 0, 0, 4, 5, 0, 0, 0]);
    }
}
