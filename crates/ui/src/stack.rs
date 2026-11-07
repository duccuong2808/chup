//! Stack thumbnail (SPEC 5.4): bố cục, vẽ, hit-test. Cửa sổ co giãn đúng theo nội dung, neo ở góc dưới của
//! work area (`chup_core::coords::place_stack`).

use chup_core::{
    capture::Shot,
    coords::Rect,
    i18n,
    icons::Icon,
    model::{Toast, ToastKind},
};

use crate::{rectf, Align, Canvas, Color, FontMetrics, FontSpec, RectF, Theme};

// Bố cục (DIP), giống bản web cũ.
pub const WIDTH: f32 = 184.0;
pub const PAD_X: f32 = 12.0;
pub const PAD_Y: f32 = 6.0;
pub const GAP: f32 = 8.0;
pub const CARD_W: f32 = 160.0;
pub const TAB_W: f32 = 28.0;
pub const TAB_H: f32 = 56.0;
pub const TAB_BOTTOM: f32 = 6.0;
pub const HEADER_H: f32 = 24.0;
/// Khoảng cách tới mép work area.
pub const MARGIN_DIP: f32 = 12.0;
/// Rê ra khỏi stack bao lâu thì ẩn header (ms).
pub const LEAVE_MS: u32 = 400;
/// Kéo thẻ quá bao xa (DIP) thì thành kéo file.
pub const DRAG_DIP: f64 = 4.0;
/// Toast còn bao lâu (ms).
pub const TOAST_MS: u32 = 2500;
pub const TOAST_ERROR_MS: u32 = 7000;
/// Giữ tối đa bao nhiêu toast.
pub const MAX_TOASTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Card(usize),
    Copy(usize),
    Folder(usize),
    Remove(usize),
    Collapse,
    Clear,
    Tab,
    Toast(u32),
}

#[derive(Clone, Debug)]
pub struct ToastItem {
    pub id: u32,
    pub toast: Toast,
}

pub fn toast_ms(t: &Toast) -> u32 {
    if t.kind == ToastKind::Error {
        TOAST_ERROR_MS
    } else {
        TOAST_MS
    }
}

/// Trạng thái stack cần để bố cục và vẽ.
pub struct StackState<'a> {
    pub shots: &'a [Shot],
    pub toasts: &'a [ToastItem],
    pub collapsed: bool,
    /// Chuột đang ở trên stack (hiện header).
    pub hovering: bool,
    pub hover: Option<Hit>,
    pub edge_right: bool,
    /// Scale màn hình.
    pub k: f32,
}

/// Font của stack: chữ toast (xuống dòng), số nhỏ (cỡ ảnh, số trên tab), chữ nút.
pub struct StackFonts<F> {
    pub body: Option<F>,
    pub small: Option<F>,
    pub btn: Option<F>,
}

/// Spec của `StackFonts` theo thứ tự body, small, btn.
pub fn font_specs(k: f32) -> [FontSpec; 3] {
    [
        FontSpec::new(13.0 * k, Align::Leading, true),
        FontSpec::new(11.0 * k, Align::Center, false),
        FontSpec::new(12.0 * k, Align::Center, false),
    ]
}

pub fn thumb_h(s: &Shot) -> f32 {
    (CARD_W * s.height as f32 / s.width.max(1) as f32)
        .round()
        .clamp(40.0, 120.0)
}

/// Phần tử trong "wrap" (trên xuống) và chiều cao (physical).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item {
    Header,
    Toast(usize, f32),
    Card(usize, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub items: Vec<Item>,
    pub wrap_visible: bool,
    pub show_tab: bool,
    /// Cỡ cửa sổ (physical).
    pub w: i32,
    pub h: i32,
}

fn item_h(it: &Item, k: f32) -> f32 {
    match it {
        Item::Header => HEADER_H * k,
        Item::Toast(_, h) | Item::Card(_, h) => *h,
    }
}

/// Bố cục; `None` = không có gì để hiện (ẩn cửa sổ).
pub fn layout<F: FontMetrics>(s: &StackState, fonts: &StackFonts<F>) -> Option<Layout> {
    let k = s.k;
    let has = !s.shots.is_empty();
    let show_list = has && !s.collapsed;
    let show_tab = has && s.collapsed;
    let show_header = s.hovering && show_list;
    let wrap_visible = show_list || !s.toasts.is_empty();
    if !wrap_visible && !show_tab {
        return None;
    }
    let mut items = vec![];
    if show_header {
        items.push(Item::Header);
    }
    for (i, t) in s.toasts.iter().enumerate() {
        let th = fonts
            .body
            .as_ref()
            .map(|f| f.measure(&t.toast.message, (CARD_W - 23.0) * k).1)
            .unwrap_or(16.0 * k);
        items.push(Item::Toast(i, th + 16.0 * k));
    }
    if show_list {
        for (i, shot) in s.shots.iter().enumerate() {
            items.push(Item::Card(i, thumb_h(shot) * k));
        }
    }
    let wrap_h = if wrap_visible {
        PAD_Y * 2.0 * k
            + items.iter().map(|it| item_h(it, k)).sum::<f32>()
            + GAP * k * (items.len().saturating_sub(1)) as f32
    } else {
        0.0
    };
    let tab_h = if show_tab {
        (TAB_H + TAB_BOTTOM) * k
    } else {
        0.0
    };
    let w = if wrap_visible { WIDTH * k } else { TAB_W * k }.ceil() as i32;
    let h = (wrap_h + tab_h).ceil() as i32;
    Some(Layout {
        items,
        wrap_visible,
        show_tab,
        w,
        h,
    })
}

/// Vẽ stack theo `l` (giữa `begin` và `end`). `image(id)` = thumbnail đã tải lên của shot. Trả về vùng bấm; nút trên
/// thẻ đứng trước thẻ nên được tìm thấy trước.
pub fn paint<'i, C: Canvas>(
    c: &C,
    theme: &Theme,
    fonts: &StackFonts<C::Font>,
    s: &StackState,
    l: &Layout,
    image: impl Fn(&str) -> Option<&'i C::Image>,
) -> Vec<(Hit, Rect)>
where
    C::Image: 'i,
{
    let (k, hover, edge_right) = (s.k, s.hover, s.edge_right);
    let to_rect = |r: RectF| r.to_rect();
    let mut hits: Vec<(Hit, Rect)> = vec![];
    let mut y = PAD_Y * k;
    let x = PAD_X * k;
    let cw = CARD_W * k;
    for it in &l.items {
        let h = item_h(it, k);
        match it {
            Item::Header => {
                let b = 24.0 * k;
                let collapse = rectf(x, y, b, b);
                let label = i18n::t().stack_clear_all;
                let clear_w = fonts
                    .btn
                    .as_ref()
                    .map(|f| f.measure(label, 1000.0).0)
                    .unwrap_or(40.0)
                    + 16.0 * k;
                let clear = rectf(x + cw - clear_w, y, clear_w, b);
                for (r, hit) in [(collapse, Hit::Collapse), (clear, Hit::Clear)] {
                    c.shadow(r, 6.0 * k, 2.0 * k, 1.0 * k, theme.shadow);
                    let bg = if hover == Some(hit) {
                        theme.surface2
                    } else {
                        theme.surface
                    };
                    c.fill_round(r, 6.0 * k, bg);
                    c.stroke_round(r, 6.0 * k, 1.0, theme.line);
                    hits.push((hit, to_rect(r)));
                }
                let chevron = if edge_right {
                    Icon::ChevronRight
                } else {
                    Icon::ChevronLeft
                };
                c.icon(
                    chevron,
                    collapse.left + 5.0 * k,
                    collapse.top + 5.0 * k,
                    14.0 * k,
                    theme.text,
                );
                if let Some(f) = fonts.btn.as_ref() {
                    c.text(label, f, clear, theme.text);
                }
            }
            Item::Toast(i, _) => {
                let t = &s.toasts[*i];
                let r = rectf(x, y, cw, h);
                c.shadow(r, 6.0 * k, 3.0 * k, 2.0 * k, theme.shadow);
                c.fill_round(r, 6.0 * k, theme.surface);
                c.stroke_round(r, 6.0 * k, 1.0, theme.line);
                let bar = if t.toast.kind == ToastKind::Error {
                    theme.danger
                } else {
                    theme.accent
                };
                c.fill_round(rectf(x, y, 3.0 * k, h), 1.5 * k, bar);
                if let Some(f) = fonts.body.as_ref() {
                    c.text(
                        &t.toast.message,
                        f,
                        rectf(x + 13.0 * k, y + 8.0 * k, cw - 23.0 * k, h - 16.0 * k),
                        theme.text,
                    );
                }
                hits.push((Hit::Toast(t.id), to_rect(r)));
            }
            Item::Card(i, _) => {
                let shot = &s.shots[*i];
                let r = rectf(x, y, cw, h);
                let rad = 6.0 * k;
                c.shadow(r, rad, 3.0 * k, 2.0 * k, theme.shadow);
                c.fill_round(r, rad, theme.surface2);
                if let Some(b) = image(&shot.id) {
                    c.image_cover(b, r, rad);
                }
                c.stroke_round(r, rad, 1.0, theme.line);
                let card_hovered = matches!(hover, Some(Hit::Card(j) | Hit::Copy(j) | Hit::Folder(j) | Hit::Remove(j)) if j == *i);
                let b = 24.0 * k;
                let btns = [
                    (Hit::Copy(*i), Icon::Copy),
                    (Hit::Folder(*i), Icon::Folder),
                    (Hit::Remove(*i), Icon::Close),
                ];
                for (n, (hit, icon)) in btns.iter().enumerate() {
                    let bx = r.right - 4.0 * k - b * (3 - n) as f32 - 2.0 * k * (2 - n) as f32;
                    let br = rectf(bx, r.top + 4.0 * k, b, b);
                    if card_hovered {
                        let a = if hover == Some(*hit) { 0.85 } else { 0.6 };
                        c.fill_round(br, 6.0 * k, Color::rgba(0, 0, 0, a));
                        c.icon(
                            *icon,
                            br.left + 5.0 * k,
                            br.top + 5.0 * k,
                            14.0 * k,
                            Color::WHITE,
                        );
                    }
                    hits.push((*hit, to_rect(br)));
                }
                if card_hovered {
                    if let Some(f) = fonts.small.as_ref() {
                        let text = format!("{} × {}", shot.width, shot.height);
                        let (tw, th) = f.measure(&text, 1000.0);
                        let mr = rectf(
                            r.left + 4.0 * k,
                            r.bottom - 4.0 * k - th - 2.0 * k,
                            tw + 10.0 * k,
                            th + 2.0 * k,
                        );
                        c.fill_round(mr, 4.0 * k, Color::rgba(0, 0, 0, 0.55));
                        c.text(&text, f, mr, Color::WHITE);
                    }
                }
                hits.push((Hit::Card(*i), to_rect(r)));
            }
        }
        y += h + GAP * k;
    }

    if l.show_tab {
        let tx = if edge_right {
            l.w as f32 - TAB_W * k
        } else {
            0.0
        };
        let ty = l.h as f32 - (TAB_H + TAB_BOTTOM) * k;
        let r = rectf(tx, ty, TAB_W * k, TAB_H * k);
        c.shadow(r, 6.0 * k, 3.0 * k, 2.0 * k, theme.shadow);
        let bg = if hover == Some(Hit::Tab) {
            theme.surface2
        } else {
            theme.surface
        };
        c.fill_round(r, 6.0 * k, bg);
        c.stroke_round(r, 6.0 * k, 1.0, theme.line);
        let chevron = if edge_right {
            Icon::ChevronLeft
        } else {
            Icon::ChevronRight
        };
        c.icon(chevron, tx + 7.0 * k, ty + 12.0 * k, 14.0 * k, theme.text);
        if let Some(f) = fonts.small.as_ref() {
            c.text(
                &s.shots.len().to_string(),
                f,
                rectf(tx, ty + 28.0 * k, TAB_W * k, 16.0 * k),
                theme.text,
            );
        }
        hits.push((Hit::Tab, to_rect(r)));
    }
    hits
}

/// Vùng bấm dưới (x, y) (toạ độ trong cửa sổ).
pub fn hit_at(hits: &[(Hit, Rect)], x: i32, y: i32) -> Option<Hit> {
    hits.iter()
        .find(|(_, r)| r.contains_point(x, y))
        .map(|(h, _)| *h)
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use image::RgbaImage;

    use super::*;
    use crate::testing::{FakeImage, FixedFont, RecordingCanvas};

    fn shot(id: &str, w: u32, h: u32) -> Shot {
        Shot {
            id: id.into(),
            path: PathBuf::from(format!("{id}.png")),
            thumb_path: PathBuf::new(),
            thumb: Arc::new(RgbaImage::new(1, 1)),
            width: w,
            height: h,
            created_at: chrono::Local::now(),
            scale: 1.0,
        }
    }

    fn fonts() -> StackFonts<FixedFont> {
        let f = || {
            Some(FixedFont {
                char_w: 7.0,
                line_h: 16.0,
            })
        };
        StackFonts {
            body: f(),
            small: f(),
            btn: f(),
        }
    }

    fn state<'a>(shots: &'a [Shot], toasts: &'a [ToastItem]) -> StackState<'a> {
        StackState {
            shots,
            toasts,
            collapsed: false,
            hovering: false,
            hover: None,
            edge_right: true,
            k: 1.0,
        }
    }

    #[test]
    fn empty_stack_hides() {
        assert_eq!(layout(&state(&[], &[]), &fonts()), None);
    }

    #[test]
    fn cards_stack_with_gap_and_padding() {
        // 1600×900 -> 90 px; 100×1000 -> chặn ở 120.
        let shots = [shot("a", 1600, 900), shot("b", 100, 1000)];
        let l = layout(&state(&shots, &[]), &fonts()).unwrap();
        assert_eq!(l.items, vec![Item::Card(0, 90.0), Item::Card(1, 120.0)]);
        assert_eq!((l.w, l.h), (184, (6.0 * 2.0 + 90.0 + 120.0 + 8.0) as i32));
    }

    #[test]
    fn header_only_when_hovering_and_tab_when_collapsed() {
        let shots = [shot("a", 160, 90)];
        let mut s = state(&shots, &[]);
        s.hovering = true;
        assert_eq!(layout(&s, &fonts()).unwrap().items[0], Item::Header);
        s.collapsed = true;
        let l = layout(&s, &fonts()).unwrap();
        assert!(l.show_tab && !l.wrap_visible);
        assert_eq!((l.w, l.h), (28, 62));
    }

    #[test]
    fn hits_put_buttons_before_card() {
        let shots = [shot("a", 160, 90)];
        let mut s = state(&shots, &[]);
        s.hover = Some(Hit::Card(0));
        let l = layout(&s, &fonts()).unwrap();
        let c = RecordingCanvas::default();
        let img = FakeImage(160, 90);
        let hits = paint(&c, &Theme::new(false), &fonts(), &s, &l, |_| Some(&img));
        let order: Vec<Hit> = hits.iter().map(|(h, _)| *h).collect();
        assert_eq!(
            order,
            vec![Hit::Copy(0), Hit::Folder(0), Hit::Remove(0), Hit::Card(0)]
        );
        // Nút Xoá sát góc trên-phải thẻ.
        let remove = hits[2].1;
        assert_eq!((remove.right(), remove.y), (12 + 160 - 4, 6 + 4));
        assert_eq!(
            hit_at(&hits, remove.x + 1, remove.y + 1),
            Some(Hit::Remove(0))
        );
        assert_eq!(hit_at(&hits, 20, 60), Some(Hit::Card(0)));
    }

    #[test]
    fn toast_height_follows_text() {
        let toasts = [ToastItem {
            id: 1,
            toast: Toast::error("x".repeat(40)),
        }];
        let l = layout(&state(&[], &toasts), &fonts()).unwrap();
        // 40 ký tự × 7 = 280 px trên dòng rộng 137 -> 3 dòng × 16 + 16.
        assert_eq!(l.items, vec![Item::Toast(0, 64.0)]);
    }
}
