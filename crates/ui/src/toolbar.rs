//! Thanh nút cạnh vùng chọn: dùng cho toolbar Live (Chụp / Mờ / Chụp text / Huỷ, SPEC 5.1, 5.2, 15.2) và toolbar
//! vẽ trong overlay Freeze (Lưu / Copy / Chữ / Huỷ, SPEC 14.3).

use chup_core::{coords::Rect, icons::Icon};

use crate::{rectf, Canvas, Color, FontMetrics, Theme};

/// 1 nút trên thanh. `primary`: nền màu nhấn. `active`: icon màu nhấn (nút Mờ khi đang Faded).
#[derive(Clone, Copy, Debug)]
pub struct BarButton {
    pub icon: Icon,
    pub label: &'static str,
    pub primary: bool,
    pub active: bool,
}

/// Toolbar Faded: cả thanh mờ 45% (alpha 115/255), rõ lại khi chuột vào.
pub const FADED_ALPHA: u8 = 115;

fn button_width<F: FontMetrics>(font: Option<&F>, k: f32, b: &BarButton, icons_only: bool) -> f32 {
    if icons_only || b.label.is_empty() {
        28.0 * k
    } else {
        let tw = font
            .map(|f| f.measure(b.label, 1000.0).0)
            .unwrap_or(30.0 * k);
        (10.0 + 16.0 + 6.0) * k + tw + 10.0 * k
    }
}

/// Chiều rộng thanh (physical) cho bộ nút `items`.
pub fn bar_width<F: FontMetrics>(
    font: Option<&F>,
    k: f32,
    items: &[BarButton],
    icons_only: bool,
) -> f32 {
    4.0 * k * (items.len() as f32 + 1.0)
        + items
            .iter()
            .map(|b| button_width(font, k, b, icons_only))
            .sum::<f32>()
}

/// Vẽ thanh nút căn phải trong khung `size` (giữa `Canvas::begin` và `end`). Trả về rect từng nút.
/// Bố cục (DIP): thanh cao 36, nút cao 28, đệm 4, cách mép phải 2.
#[allow(clippy::too_many_arguments)]
pub fn paint_bar<C: Canvas>(
    c: &C,
    font: Option<&C::Font>,
    theme: &Theme,
    k: f32,
    size: (i32, i32),
    items: &[BarButton],
    icons_only: bool,
    hover: Option<usize>,
    pressed: Option<usize>,
) -> Vec<Rect> {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let (bh, bh_btn, pad) = (36.0 * k, 28.0 * k, 4.0 * k);
    let bar_w = bar_width(font, k, items, icons_only);
    let (bar_x, bar_y) = (w - bar_w - 2.0 * k, (h - bh) / 2.0);
    let by = bar_y + (bh - bh_btn) / 2.0;
    let mut x = bar_x + pad;
    let mut rects = Vec::with_capacity(items.len());
    for b in items {
        let bw = button_width(font, k, b, icons_only);
        rects.push(Rect::new(x as i32, by as i32, bw as u32, bh_btn as u32));
        x += bw + pad;
    }

    let bar = rectf(bar_x, bar_y, bar_w, bh);
    c.shadow(bar, 10.0 * k, 3.0 * k, 1.0 * k, theme.shadow);
    c.fill_round(bar, 10.0 * k, theme.surface);
    c.stroke_round(bar, 10.0 * k, 1.0, theme.line);
    for (i, (b, r)) in items.iter().zip(&rects).enumerate() {
        let rf = rectf(r.x as f32, r.y as f32, r.w as f32, r.h as f32);
        let down = if pressed == Some(i) { 1.0 * k } else { 0.0 };
        let hovered = hover == Some(i);
        let (bg, fg) = if b.primary {
            (
                Some(if hovered {
                    theme.accent_strong
                } else {
                    theme.accent
                }),
                Color::WHITE,
            )
        } else if b.active {
            (hovered.then_some(theme.surface2), theme.accent)
        } else {
            (hovered.then_some(theme.surface2), theme.text)
        };
        if let Some(bg) = bg {
            c.fill_round(rf, 6.0 * k, bg);
        }
        let icon_sz = 16.0 * k;
        let iy = rf.top + (r.h as f32 - icon_sz) / 2.0 + down;
        if icons_only || b.label.is_empty() {
            c.icon(
                b.icon,
                rf.left + (r.w as f32 - icon_sz) / 2.0,
                iy,
                icon_sz,
                fg,
            );
        } else {
            c.icon(b.icon, rf.left + 10.0 * k, iy, icon_sz, fg);
            if let Some(f) = font {
                let tx = rf.left + (10.0 + 16.0 + 6.0) * k;
                c.text(
                    b.label,
                    f,
                    rectf(tx, rf.top + down, rf.right - tx, rf.bottom - rf.top),
                    fg,
                );
            }
        }
    }
    rects
}

/// Nút của toolbar Live, theo thứ tự trên thanh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveBtn {
    Capture,
    Fade,
    /// Chụp vùng rồi OCR, copy chữ (không lưu file).
    Text,
    Close,
}

pub const LIVE_BUTTONS: [LiveBtn; 4] = [
    LiveBtn::Capture,
    LiveBtn::Fade,
    LiveBtn::Text,
    LiveBtn::Close,
];

/// Bộ nút toolbar Live (chữ theo ngôn ngữ giao diện). `faded`: nút Mờ đang bật.
pub fn live_items(faded: bool) -> [BarButton; 4] {
    let t = chup_core::i18n::t();
    [
        BarButton {
            icon: Icon::Camera,
            label: t.btn_capture,
            primary: true,
            active: false,
        },
        BarButton {
            icon: Icon::Fade,
            label: t.btn_fade,
            primary: false,
            active: faded,
        },
        BarButton {
            icon: Icon::Text,
            label: "",
            primary: false,
            active: false,
        },
        BarButton {
            icon: Icon::Close,
            label: "",
            primary: false,
            active: false,
        },
    ]
}

/// Bộ nút toolbar Freeze (chữ theo ngôn ngữ giao diện).
pub fn freeze_items() -> [BarButton; 4] {
    let t = chup_core::i18n::t();
    [
        BarButton {
            icon: Icon::Check,
            label: t.btn_save,
            primary: true,
            active: false,
        },
        BarButton {
            icon: Icon::Copy,
            label: "Copy",
            primary: false,
            active: false,
        },
        BarButton {
            icon: Icon::Text,
            label: t.btn_text,
            primary: false,
            active: false,
        },
        BarButton {
            icon: Icon::Close,
            label: "",
            primary: false,
            active: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FixedFont, Op, RecordingCanvas};

    const FONT: FixedFont = FixedFont {
        char_w: 7.0,
        line_h: 16.0,
    };

    fn items() -> [BarButton; 3] {
        [
            BarButton {
                icon: Icon::Camera,
                label: "Chụp",
                primary: true,
                active: false,
            },
            BarButton {
                icon: Icon::Fade,
                label: "",
                primary: false,
                active: true,
            },
            BarButton {
                icon: Icon::Close,
                label: "",
                primary: false,
                active: false,
            },
        ]
    }

    #[test]
    fn width_sums_buttons_and_padding() {
        // Nút có chữ: 32 + 4*7 + 10 = 70; 2 nút icon: 28; đệm 4 × 4.
        assert_eq!(
            bar_width(Some(&FONT), 1.0, &items(), false),
            70.0 + 56.0 + 16.0
        );
        assert_eq!(
            bar_width(Some(&FONT), 1.0, &items(), true),
            3.0 * 28.0 + 16.0
        );
        assert_eq!(
            bar_width::<FixedFont>(None, 2.0, &items(), false),
            2.0 * (32.0 + 30.0 + 10.0 + 56.0 + 16.0)
        );
    }

    #[test]
    fn buttons_are_right_aligned_and_vertically_centered() {
        let c = RecordingCanvas::default();
        let w = bar_width(Some(&FONT), 1.0, &items(), false);
        let rects = paint_bar(
            &c,
            Some(&FONT),
            &Theme::new(false),
            1.0,
            (300, 40),
            &items(),
            false,
            None,
            None,
        );
        let bar_x = 300.0 - w - 2.0;
        assert_eq!(rects[0], Rect::new((bar_x + 4.0) as i32, 6, 70, 28));
        assert_eq!(rects[2].right(), (bar_x + w - 4.0) as i32);
        assert!(rects.windows(2).all(|p| p[0].right() + 4 == p[1].x));
    }

    #[test]
    fn hover_and_active_colors() {
        let theme = Theme::new(false);
        let c = RecordingCanvas::default();
        paint_bar(
            &c,
            Some(&FONT),
            &theme,
            1.0,
            (300, 40),
            &items(),
            false,
            Some(0),
            Some(0),
        );
        let ops = c.ops();
        assert!(ops
            .iter()
            .any(|o| matches!(o, Op::Fill(_, _, col) if *col == theme.accent_strong)));
        // Nút "active" không hover: không tô nền, icon màu nhấn.
        assert!(ops
            .iter()
            .any(|o| matches!(o, Op::Icon(Icon::Fade, _, _, _, col) if *col == theme.accent)));
        // Nút đang nhấn: icon lệch xuống 1 px.
        let cam = ops.iter().find_map(|o| match o {
            Op::Icon(Icon::Camera, _, y, _, _) => Some(*y),
            _ => None,
        });
        assert_eq!(cam, Some(6.0 + 6.0 + 1.0));
    }
}
