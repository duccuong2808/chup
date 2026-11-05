//! Cửa sổ Preview (SPEC 17): vẽ 2 lớp và vùng bấm. Logic phóng to / kéo / đổi cỡ ở `chup_core::preview`, chọn chữ
//! ở `chup_core::ocr`.
//!
//! - `paint_base`: bóng đổ + ảnh đã scale + nền + viền, vẽ lại khi cỡ, mức phóng to hoặc chỗ kéo ảnh đổi.
//! - `paint_top`: vùng chọn chữ, toolbar (khi hover) và nhãn zoom/opacity, vẽ chồng lên bản chép của `base`.
//!
//! Cửa sổ thật rộng hơn `view.rect` một lề `margin` mỗi phía để vẽ bóng đổ. Toạ độ "cục bộ" là toạ độ so với góc
//! trên-trái của `view.rect`; toạ độ surface = cục bộ + lề.

use chup_core::{
    coords::Rect,
    icons::Icon,
    ocr::{Pos, TextLayout},
    preview::View,
};

use crate::{rectf, Align, Canvas, Color, FontSpec, RectF, Theme};

// Kích thước (DIP).
pub const MIN_W: f32 = 160.0;
pub const MIN_H: f32 = 48.0;
pub const RADIUS: f32 = 8.0;
pub const BTN: f32 = 28.0;
pub const BTN_GAP: f32 = 4.0;
pub const BAR_PAD: f32 = 8.0;
/// Bề rộng dải mép cửa sổ để kéo đổi cỡ.
pub const EDGE: f32 = 6.0;
/// Nới vùng nhận chuột quanh dòng chữ (DIP).
pub const TEXT_PAD: f32 = 4.0;
/// Độ đục của màu tô vùng chọn chữ.
pub const SEL_ALPHA: f32 = 0.35;
/// Bóng đổ quanh khung: độ loang và độ lệch xuống (DIP). Lề cửa sổ = tổng hai số.
pub const SHADOW_SPREAD: f32 = 8.0;
pub const SHADOW_DY: f32 = 2.0;
/// Nhãn zoom / opacity hiện bao lâu (ms).
pub const PILL_MS: u32 = 1200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    /// OCR: copy chữ trong ảnh (SPEC 15.2). Đang OCR thì vẽ vòng "bận", bấm lần nữa là huỷ.
    Ocr,
    Pin,
    Copy,
    Folder,
    Close,
}

pub const BUTTONS: [(Btn, Icon); 5] = [
    (Btn::Ocr, Icon::Text),
    (Btn::Pin, Icon::Pin),
    (Btn::Copy, Icon::Copy),
    (Btn::Folder, Icon::Folder),
    (Btn::Close, Icon::Close),
];

/// Font của nhãn zoom / opacity.
pub fn pill_font(k: f32) -> FontSpec {
    FontSpec::new(12.0 * k, Align::Center, false)
}

/// Cỡ cửa sổ nhỏ nhất (physical px) ở scale `k`.
pub fn min_size(k: f64) -> (u32, u32) {
    (
        (MIN_W as f64 * k).round() as u32,
        (MIN_H as f64 * k).round() as u32,
    )
}

/// Lề bóng đổ quanh `view.rect` (physical px).
pub fn margin(k: f32) -> i32 {
    ((SHADOW_SPREAD + SHADOW_DY) * k).ceil() as i32
}

/// Vùng bấm của toolbar góc trên-phải (toạ độ cục bộ) cho khung rộng `w`. Luôn tính, kể cả khi không vẽ.
pub fn bar_hits(w: f32, k: f32) -> Vec<(Btn, Rect)> {
    let b = BTN * k;
    let x0 = bar_x0(w, k);
    BUTTONS
        .iter()
        .enumerate()
        .map(|(i, (btn, _))| {
            let x = x0 + i as f32 * (b + BTN_GAP * k);
            (
                *btn,
                Rect::new(x as i32, (BAR_PAD * k) as i32, b as u32, b as u32),
            )
        })
        .collect()
}

fn bar_x0(w: f32, k: f32) -> f32 {
    let (b, n) = (BTN * k, BUTTONS.len() as f32);
    w - BAR_PAD * k - n * b - (n - 1.0) * BTN_GAP * k
}

/// Vẽ lớp nền (giữa `begin` và `end`): bóng đổ, ảnh theo `v` (cỡ ảnh `dims`), viền. `grab`: lấp khung bằng alpha
/// 1/255 để góc bo tròn vẫn nhận chuột (layered window của Windows bỏ qua alpha 0).
#[allow(clippy::too_many_arguments)]
pub fn paint_base<C: Canvas>(
    c: &C,
    theme: &Theme,
    v: View,
    dims: (u32, u32),
    img: &C::Image,
    m: i32,
    k: f32,
    grab: bool,
) {
    let rad = RADIUS * k;
    let (mf, vw, vh) = (m as f32, v.rect.w as f32, v.rect.h as f32);
    let full = rectf(mf, mf, vw, vh);
    c.shadow(full, rad, SHADOW_SPREAD * k, SHADOW_DY * k, theme.shadow);
    if grab {
        c.fill_round(full, rad, Color::rgba(0, 0, 0, 1.0 / 255.0));
    }
    let fr = v.frame(dims);
    let fills = fr.w == v.rect.w && fr.h == v.rect.h;
    if !fills {
        // Ảnh nhỏ hơn cỡ tối thiểu: nằm giữa trên nền tối.
        c.fill_round(full, rad, Color::rgba(24, 24, 27, 1.0));
    }
    let img_r = rectf(fr.x as f32 + mf, fr.y as f32 + mf, fr.w as f32, fr.h as f32);
    let (ox, oy) = v.origin(dims);
    c.image_at(
        img,
        img_r,
        if fills { rad } else { 0.0 },
        (ox as f32 + mf, oy as f32 + mf),
        v.scale() as f32,
    );
    c.stroke_round(
        rectf(mf + 0.5, mf + 0.5, vw - 1.0, vh - 1.0),
        rad,
        1.0,
        theme.line,
    );
}

/// Vùng chọn chữ `sel` -> các rect tô (toạ độ surface), cắt theo khung ảnh (đang phóng to thì phần chữ ngoài khung
/// không vẽ).
pub fn sel_rects(
    text: &TextLayout,
    sel: (Pos, Pos),
    v: View,
    dims: (u32, u32),
    m: i32,
    k: f32,
) -> Vec<RectF> {
    let mf = m as f32;
    let fr = v.frame(dims);
    let (fl, ft) = (fr.x as f32 + mf, fr.y as f32 + mf);
    let (fr_, fb) = (fl + fr.w as f32, ft + fr.h as f32);
    let (ox, oy) = v.origin(dims);
    let (ox, oy, z) = (ox as f32 + mf, oy as f32 + mf, v.scale() as f32);
    let pad = k;
    text.highlight(sel.0, sel.1)
        .iter()
        .filter_map(|r| {
            let l = (ox + r.x as f32 * z - pad).max(fl);
            let t = (oy + r.y as f32 * z - pad).max(ft);
            let rt = (ox + (r.x + r.w as i32) as f32 * z + pad).min(fr_);
            let bt = (oy + (r.y + r.h as i32) as f32 * z + pad).min(fb);
            (rt > l && bt > t).then(|| rectf(l, t, rt - l, bt - t))
        })
        .collect()
}

/// Khung bao vùng bôi đen (nới 2 px cho bo góc, khử răng cưa), để chỉ vẽ lại phần đổi.
pub fn sel_box(rects: &[RectF]) -> Option<Rect> {
    rects.iter().fold(None::<Rect>, |acc, r| {
        let (x, y) = (r.left.floor() as i32 - 2, r.top.floor() as i32 - 2);
        let ri = Rect::new(
            x,
            y,
            (r.right.ceil() as i32 + 2 - x) as u32,
            (r.bottom.ceil() as i32 + 2 - y) as u32,
        );
        Some(acc.map_or(ri, |a| a.union(&ri)))
    })
}

/// Lớp trên của Preview.
pub struct Top<'a, F> {
    pub k: f32,
    /// Lề bóng đổ.
    pub m: i32,
    /// Cỡ khung (`view.rect`).
    pub size: (f32, f32),
    pub sel: &'a [RectF],
    /// Chuột trong cửa sổ: vẽ toolbar.
    pub bar: bool,
    pub hover: Option<Btn>,
    /// Đang OCR ảnh này: nút [T] vẽ vòng "bận".
    pub busy: bool,
    pub topmost: bool,
    /// Nhãn zoom / opacity và font của nó.
    pub pill: Option<(&'a str, &'a F)>,
}

/// Vẽ lớp trên (giữa `begin_over` và `end`).
pub fn paint_top<C: Canvas>(c: &C, theme: &Theme, t: &Top<C::Font>) {
    let (k, mf) = (t.k, t.m as f32);
    let (w, h) = t.size;
    let sel_color = theme.accent.with_alpha(SEL_ALPHA);
    for r in t.sel {
        c.fill_round(*r, 2.0 * k, sel_color);
    }
    if t.bar {
        let b = BTN * k;
        let x0 = bar_x0(w, k);
        for (i, (btn, icon)) in BUTTONS.iter().enumerate() {
            let r = rectf(
                mf + x0 + i as f32 * (b + BTN_GAP * k),
                mf + BAR_PAD * k,
                b,
                b,
            );
            let hot = t.hover == Some(*btn);
            let busy = *btn == Btn::Ocr && t.busy;
            let icon = if busy { Icon::Busy } else { *icon };
            let bg = if (*btn == Btn::Pin && t.topmost) || busy {
                theme.accent.with_alpha(if hot { 1.0 } else { 0.9 })
            } else {
                Color::rgba(0, 0, 0, if hot { 0.85 } else { 0.6 })
            };
            c.fill_round(r, 6.0 * k, bg);
            let is = 16.0 * k;
            c.icon(
                icon,
                r.left + (b - is) / 2.0,
                r.top + (b - is) / 2.0,
                is,
                Color::WHITE,
            );
        }
    }
    if let Some((text, f)) = t.pill {
        let (tw, th) = crate::FontMetrics::measure(f, text, 10_000.0);
        let (pw, ph) = (tw + 20.0 * k, th + 8.0 * k);
        let r = rectf(mf + (w - pw) / 2.0, mf + h - 12.0 * k - ph, pw, ph);
        c.fill_round(r, ph / 2.0, Color::rgba(0, 0, 0, 0.7));
        c.text(text, f, r, Color::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeImage, FixedFont, Op, RecordingCanvas};

    #[test]
    fn bar_hits_are_right_aligned() {
        let hits = bar_hits(400.0, 1.0);
        assert_eq!(hits.len(), 5);
        assert_eq!(hits[4], (Btn::Close, Rect::new(400 - 8 - 28, 8, 28, 28)));
        assert_eq!(hits[0].1.x, 400 - 8 - 5 * 28 - 4 * 4);
    }

    #[test]
    fn margin_and_min_size_scale() {
        assert_eq!(margin(1.0), 10);
        assert_eq!(margin(1.5), 15);
        assert_eq!(min_size(2.0), (320, 96));
    }

    #[test]
    fn base_draws_image_at_view_origin() {
        let v = View::new(Rect::new(0, 0, 200, 100), 1.0);
        let c = RecordingCanvas::default();
        paint_base(
            &c,
            &Theme::new(false),
            v,
            (200, 100),
            &FakeImage(200, 100),
            10,
            1.0,
            true,
        );
        let img = c.ops().into_iter().find_map(|o| match o {
            Op::Image(_, r, rad, origin, scale) => Some((r, rad, origin, scale)),
            _ => None,
        });
        assert_eq!(
            img,
            Some((rectf(10.0, 10.0, 200.0, 100.0), 8.0, (10.0, 10.0), 1.0))
        );
    }

    #[test]
    fn top_marks_pin_and_busy() {
        let theme = Theme::new(true);
        let f = FixedFont {
            char_w: 7.0,
            line_h: 14.0,
        };
        let c = RecordingCanvas::default();
        let t = Top {
            k: 1.0,
            m: 10,
            size: (300.0, 200.0),
            sel: &[],
            bar: true,
            hover: None,
            busy: true,
            topmost: true,
            pill: Some(("120%", &f)),
        };
        paint_top(&c, &theme, &t);
        let ops = c.ops();
        assert!(ops.iter().any(|o| matches!(o, Op::Icon(Icon::Busy, ..))));
        let accent_fills = ops
            .iter()
            .filter(|o| matches!(o, Op::Fill(_, _, col) if *col == theme.accent.with_alpha(0.9)))
            .count();
        assert_eq!(accent_fills, 2);
        assert!(ops
            .iter()
            .any(|o| matches!(o, Op::Text(s, ..) if s == "120%")));
    }

    #[test]
    fn sel_box_unions_with_padding() {
        let b = sel_box(&[rectf(10.5, 10.0, 5.0, 5.0), rectf(30.0, 20.0, 5.0, 5.0)]);
        assert_eq!(b, Some(Rect::new(8, 8, 29, 19)));
        assert_eq!(sel_box(&[]), None);
    }
}
