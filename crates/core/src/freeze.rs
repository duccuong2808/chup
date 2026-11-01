//! Pixel cho overlay Freeze (SPEC 14.3, 14.4): chép ảnh đóng băng 1:1 vào DIB, phần ngoài vùng
//! chọn tối 40%; crop luôn từ `RgbaImage` gốc.

use image::RgbaImage;

use crate::coords::Rect;

/// 1 pixel RGBA -> BGRA premultiplied, alpha 255 (mọi chỗ trên overlay nhận chuột).
/// `dim`: nhân 154/256 ≈ 0,6 (tối 40%).
#[inline]
pub fn to_bgra(px: [u8; 4], dim: bool) -> u32 {
    let k = |c: u8| if dim { (c as u32 * 154) >> 8 } else { c as u32 };
    0xFF00_0000 | (k(px[0]) << 16) | (k(px[1]) << 8) | k(px[2])
}

/// Vẽ phần `dirty` của DIB `dst` (rộng `dst_w`, toạ độ overlay) từ `img`.
/// Trong `sel` giữ nguyên màu, ngoài `sel` tô tối. Phần DIB không có ảnh (ảnh nhỏ hơn màn hình) tô đen.
pub fn compose(img: &RgbaImage, dst: &mut [u32], dst_w: usize, dirty: Rect, sel: Option<Rect>) {
    let dst_h = dst.len() / dst_w.max(1);
    let Some(d) = dirty.intersect(&Rect::new(0, 0, dst_w as u32, dst_h as u32)) else { return };
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let raw = img.as_raw();
    // Chia mỗi hàng thành các đoạn liền nhau (tối | sáng | tối | không có ảnh) để vòng lặp trong
    // không rẽ nhánh theo từng pixel: ở 4K đây là phần tốn nhất khi hiện overlay.
    let (x0, x1) = (d.x, d.right());
    let img_end = iw.clamp(x0, x1);
    for y in d.y..d.bottom() {
        let row = &mut dst[y as usize * dst_w..(y as usize + 1) * dst_w];
        if y >= ih {
            row[x0 as usize..x1 as usize].fill(0xFF00_0000);
            continue;
        }
        let src = &raw[y as usize * iw as usize * 4..(y as usize + 1) * iw as usize * 4];
        let (l0, l1) = match sel {
            Some(s) if y >= s.y && y < s.bottom() => (s.x.clamp(x0, img_end), s.right().clamp(x0, img_end)),
            _ => (img_end, img_end),
        };
        for (a, b, dim) in [(x0, l0, true), (l0, l1, false), (l1, img_end, true)] {
            let (a, b) = (a as usize, b.max(a) as usize);
            for (o, p) in row[a..b].iter_mut().zip(src[a * 4..b * 4].chunks_exact(4)) {
                *o = to_bgra([p[0], p[1], p[2], p[3]], dim);
            }
        }
        row[img_end as usize..x1 as usize].fill(0xFF00_0000);
    }
}

/// Cắt `rect` (physical px toàn desktop) từ ảnh đóng băng của màn hình đặt tại `mon`.
/// `None` nếu vùng không cắt qua ảnh.
pub fn crop(img: &RgbaImage, mon: Rect, rect: Rect) -> Option<RgbaImage> {
    let img_r = Rect::new(mon.x, mon.y, img.width(), img.height());
    let r = rect.intersect(&img_r)?;
    Some(image::imageops::crop_imm(img, (r.x - mon.x) as u32, (r.y - mon.y) as u32, r.w, r.h).to_image())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn bgra_and_dim() {
        assert_eq!(to_bgra([255, 128, 0, 255], false), 0xFFFF_8000);
        // Tối 40%: 255 -> 153, 128 -> 77. Alpha luôn 255.
        assert_eq!(to_bgra([255, 128, 0, 255], true), 0xFF99_4D00);
        assert_eq!(to_bgra([10, 20, 30, 0], false) >> 24, 0xFF);
    }

    #[test]
    fn compose_dims_outside_selection_only() {
        let img = RgbaImage::from_pixel(4, 2, Rgba([255, 255, 255, 255]));
        let mut dst = vec![0u32; 4 * 2];
        compose(&img, &mut dst, 4, Rect::new(0, 0, 4, 2), Some(Rect::new(1, 0, 2, 1)));
        let (lit, dim) = (0xFFFF_FFFF, 0xFF99_9999);
        assert_eq!(dst, vec![dim, lit, lit, dim, dim, dim, dim, dim]);
    }

    #[test]
    fn compose_touches_only_dirty_and_pads_black() {
        // Ảnh 2×1 nhỏ hơn DIB 3×2: phần thiếu tô đen. Ngoài dirty giữ nguyên.
        let img = RgbaImage::from_pixel(2, 1, Rgba([0, 0, 255, 255]));
        let mut dst = vec![7u32; 3 * 2];
        compose(&img, &mut dst, 3, Rect::new(1, 0, 2, 2), Some(Rect::new(0, 0, 3, 2)));
        assert_eq!(dst, vec![7, 0xFF00_00FF, 0xFF00_0000, 7, 0xFF00_0000, 0xFF00_0000]);
    }

    #[test]
    fn crop_handles_negative_monitor_origin() {
        // Màn hình phụ bên trái màn hình chính: toạ độ desktop âm.
        let mon = Rect::new(-1920, 0, 1920, 1080);
        let mut img = RgbaImage::new(1920, 1080);
        img.put_pixel(10, 20, Rgba([255, 0, 0, 255]));
        let out = crop(&img, mon, Rect::new(-1910, 20, 5, 5)).unwrap();
        assert_eq!(out.dimensions(), (5, 5));
        assert_eq!(out.get_pixel(0, 0), &Rgba([255, 0, 0, 255]));
        // Vùng tràn qua màn hình bên phải: chỉ lấy phần thuộc màn hình này.
        assert_eq!(crop(&img, mon, Rect::new(-5, 0, 10, 10)).unwrap().dimensions(), (5, 10));
        assert!(crop(&img, mon, Rect::new(10, 10, 5, 5)).is_none());
    }
}
