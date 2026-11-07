//! Overlay chọn vùng (SPEC 5.1), Freeze (14.3), tự động chọn vùng (16): phần vẽ pixel và vị trí nhãn.
//! State machine của chuột (kéo, handle, toolbar trong overlay) ở vỏ OS.

use chup_core::{coords::Rect, selection};

use crate::{Color, Pixels};

/// Lớp tối phủ màn hình (Live).
pub const DIM: Color = Color::rgba(0, 0, 0, 0.4);
/// Phủ màn hình chụp hỏng (Freeze).
pub const FAILED_DIM: Color = Color::rgba(0, 0, 0, 0.6);
/// Gần như trong suốt nhưng vẫn nhận chuột (layered window của Windows bỏ qua pixel alpha 0).
pub const HOLE_PX: u32 = 0x0100_0000;
pub const WHITE: u32 = 0xFFFF_FFFF;
pub const DARK: u32 = 0xFF20_2020;

/// Freeze: handle vẽ 8 DIP, vùng bắt chuột 14 DIP cho dễ trúng.
pub const HANDLE_DIP: f64 = 8.0;
pub const HANDLE_HIT_DIP: f64 = 14.0;
/// Freeze: toolbar trong overlay cao 44 DIP, cách vùng chọn 6 DIP.
pub const BAR_H_DIP: f64 = 44.0;
pub const BAR_GAP_DIP: f64 = 6.0;
/// Tự chọn vùng: viền nét đứt dày 2 DIP, đoạn 6 DIP.
pub const EDGE_DIP: f64 = 2.0;
pub const DASH_DIP: f64 = 6.0;

/// Dòng gợi ý: chữ 13 DIP, đệm 12×6, cách mép trên 24 DIP.
pub const HINT_FONT_DIP: f32 = 13.0;
pub const HINT_PAD: (f32, f32) = (12.0, 6.0);
pub const HINT_TOP_DIP: f64 = 24.0;
pub const HINT_BG: Color = Color::rgba(0, 0, 0, 0.7);
/// Nhãn "W × H": chữ 12 DIP, đệm 6×2, bo 4.
pub const LABEL_FONT_DIP: f32 = 12.0;
pub const LABEL_PAD: (f32, f32) = (6.0, 2.0);
pub const LABEL_RADIUS_DIP: f32 = 4.0;
pub const LABEL_BG: Color = Color::rgba(0, 0, 0, 0.75);

/// Live: phủ tối phần `dirty`, làm sáng khung tự động `hl` và vùng chọn `sel`, viền trắng quanh `sel`.
/// `hole`: pixel cho phần sáng (Windows: [`HOLE_PX`] để vẫn nhận chuột).
pub fn paint_live(px: &mut Pixels, dirty: Rect, hl: Option<Rect>, sel: Option<Rect>, hole: u32) {
    px.fill(dirty, DIM.px());
    if let Some(inside) = hl.and_then(|h| h.intersect(&dirty)) {
        px.fill(inside, hole);
    }
    if let Some(sel) = sel {
        if let Some(r) = sel.intersect(&dirty) {
            px.fill(r, hole);
        }
        px.outline_outside(sel, 1, Color::rgba(255, 255, 255, 0.95).px());
    }
}

/// Freeze: viền trắng quanh vùng chọn và 8 handle (cạnh `handle`) khi không đang kéo vùng mới.
pub fn paint_frozen_sel(px: &mut Pixels, dirty: Rect, sel: Rect, handle: i32, handles: bool) {
    px.outline_outside(sel, 1, WHITE);
    if handles {
        for (_, hr) in selection::handles(sel, handle) {
            if hr.intersect(&dirty).is_some() {
                px.fill(hr.expand(1), DARK);
                px.fill(hr, WHITE);
            }
        }
    }
}

/// Chỗ đặt nhãn "W × H" cỡ `label` cho vùng `sel` trên overlay cỡ `mon`: dưới vùng chọn, sát đáy thì lên trên.
/// `gap` = 6 DIP, `right_room` = 110 DIP (không để nhãn tràn mép phải).
pub fn label_rect(
    sel: Rect,
    label: (i32, i32),
    mon: (i32, i32),
    gap: i32,
    right_room: i32,
) -> Rect {
    let (lw, lh) = label;
    let max_x = mon.0 - right_room;
    let below = sel.bottom() + gap;
    let ly = if below + lh > mon.1 {
        sel.y - gap - lh
    } else {
        below
    };
    Rect::new(sel.x.min(max_x).max(0), ly.max(2), lw as u32, lh as u32)
}

/// Nhãn "W × H · Loại" của khung tự động `r`: dưới khung, sát đáy thì lên trên, không còn chỗ thì nằm trong góc
/// trên-trái khung.
pub fn auto_label_rect(r: Rect, label: (i32, i32), mon: (i32, i32), gap: i32) -> Rect {
    let (lw, lh) = label;
    let (w, h) = mon;
    let y = if r.bottom() + gap + lh <= h {
        r.bottom() + gap
    } else if r.y - gap - lh >= 0 {
        r.y - gap - lh
    } else {
        r.y + gap
    };
    let x = (r.x + if y == r.y + gap { gap } else { 0 }).clamp(0, (w - lw).max(0));
    Rect::new(x, y, lw as u32, lh as u32)
}

/// Chỗ đặt dòng gợi ý cỡ `size`: giữa, cách mép trên `top`.
pub fn hint_rect(size: (i32, i32), mon_w: i32, top: i32) -> Rect {
    Rect::new((mon_w - size.0) / 2, top, size.0 as u32, size.1 as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_goes_below_then_above() {
        let mon = (1000, 800);
        assert_eq!(
            label_rect(Rect::new(100, 100, 50, 50), (60, 20), mon, 6, 110),
            Rect::new(100, 156, 60, 20)
        );
        // Sát đáy: lên trên vùng chọn.
        assert_eq!(
            label_rect(Rect::new(100, 700, 50, 90), (60, 20), mon, 6, 110),
            Rect::new(100, 674, 60, 20)
        );
        // Sát mép phải: lùi vào.
        assert_eq!(
            label_rect(Rect::new(950, 100, 50, 50), (60, 20), mon, 6, 110).x,
            890
        );
    }

    #[test]
    fn auto_label_falls_inside_when_no_room() {
        let mon = (1000, 800);
        // Khung phủ cả màn hình: nhãn nằm trong góc trên-trái.
        assert_eq!(
            auto_label_rect(Rect::new(0, 0, 1000, 800), (80, 20), mon, 6),
            Rect::new(6, 6, 80, 20)
        );
        assert_eq!(
            auto_label_rect(Rect::new(10, 10, 100, 100), (80, 20), mon, 6),
            Rect::new(10, 116, 80, 20)
        );
    }

    #[test]
    fn live_paint_dims_outside_selection() {
        let (w, h) = (20, 20);
        let mut buf = vec![0u32; (w * h) as usize];
        let mut px = Pixels::new(&mut buf, w, h);
        paint_live(
            &mut px,
            Rect::new(0, 0, 20, 20),
            None,
            Some(Rect::new(5, 5, 5, 5)),
            HOLE_PX,
        );
        assert_eq!(buf[0], DIM.px());
        assert_eq!(buf[(7 * w + 7) as usize], HOLE_PX);
        assert_eq!(
            buf[(4 * w + 7) as usize],
            Color::rgba(255, 255, 255, 0.95).px()
        );
    }
}
