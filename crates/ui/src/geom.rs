//! Hình chữ nhật số thực (toạ độ vẽ, physical px).

use chup_core::coords::Rect;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub fn rectf(x: f32, y: f32, w: f32, h: f32) -> RectF {
    RectF {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

impl RectF {
    pub fn w(&self) -> f32 {
        self.right - self.left
    }

    pub fn h(&self) -> f32 {
        self.bottom - self.top
    }

    /// Rect nguyên (cắt phần lẻ, như hit-test cũ của vỏ Windows).
    pub fn to_rect(&self) -> Rect {
        Rect::new(
            self.left as i32,
            self.top as i32,
            self.w() as u32,
            self.h() as u32,
        )
    }

    pub fn from_rect(r: Rect) -> RectF {
        rectf(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
    }
}
