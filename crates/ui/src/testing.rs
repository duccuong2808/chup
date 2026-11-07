//! Canvas ghi lại lệnh vẽ và font độ rộng cố định, để test bố cục và thứ tự vẽ mà không cần OS.

use std::cell::RefCell;

use chup_core::icons::Icon;
use image::RgbaImage;

use crate::{Canvas, Color, FontMetrics, RectF};

/// Mỗi ký tự rộng `char_w`, cao `line_h`; xuống dòng khi vượt `max_w`.
pub struct FixedFont {
    pub char_w: f32,
    pub line_h: f32,
}

impl FontMetrics for FixedFont {
    fn measure(&self, text: &str, max_w: f32) -> (f32, f32) {
        let n = text.chars().count() as f32;
        let w = n * self.char_w;
        let lines = if w > max_w && max_w > 0.0 {
            (w / max_w).ceil()
        } else {
            1.0
        };
        (
            w.min(max_w.max(self.char_w)).ceil(),
            (lines * self.line_h).ceil(),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Begin(Color),
    BeginOver,
    End,
    Clip(RectF),
    PopClip,
    Fill(RectF, f32, Color),
    Stroke(RectF, f32, f32, Color),
    Text(String, RectF, Color),
    Icon(Icon, f32, f32, f32, Color),
    Image(u32, RectF, f32, (f32, f32), f32),
}

#[derive(Default)]
pub struct RecordingCanvas {
    pub ops: RefCell<Vec<Op>>,
}

impl RecordingCanvas {
    pub fn ops(&self) -> Vec<Op> {
        self.ops.borrow().clone()
    }

    fn push(&self, op: Op) {
        self.ops.borrow_mut().push(op);
    }
}

/// Ảnh giả: chỉ nhớ kích thước (id = rộng).
pub struct FakeImage(pub u32, pub u32);

impl Canvas for RecordingCanvas {
    type Font = FixedFont;
    type Image = FakeImage;

    fn image(&self, img: &RgbaImage) -> Option<FakeImage> {
        Some(FakeImage(img.width(), img.height()))
    }
    fn image_size(&self, img: &FakeImage) -> (f32, f32) {
        (img.0 as f32, img.1 as f32)
    }
    fn begin(&self, clear: Color) {
        self.push(Op::Begin(clear));
    }
    fn begin_over(&self) {
        self.push(Op::BeginOver);
    }
    fn end(&self) {
        self.push(Op::End);
    }
    fn push_clip(&self, r: RectF) {
        self.push(Op::Clip(r));
    }
    fn pop_clip(&self) {
        self.push(Op::PopClip);
    }
    fn fill_round(&self, r: RectF, radius: f32, c: Color) {
        self.push(Op::Fill(r, radius, c));
    }
    fn stroke_round(&self, r: RectF, radius: f32, width: f32, c: Color) {
        self.push(Op::Stroke(r, radius, width, c));
    }
    fn text(&self, text: &str, _font: &FixedFont, r: RectF, c: Color) {
        self.push(Op::Text(text.to_string(), r, c));
    }
    fn icon(&self, icon: Icon, x: f32, y: f32, size: f32, c: Color) {
        self.push(Op::Icon(icon, x, y, size, c));
    }
    fn image_at(&self, img: &FakeImage, r: RectF, radius: f32, origin: (f32, f32), scale: f32) {
        self.push(Op::Image(img.0, r, radius, origin, scale));
    }
}
