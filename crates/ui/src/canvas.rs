//! Trait vẽ hình và chữ. Mỗi OS cài bằng API vẽ native (Windows: Direct2D software + DirectWrite;
//! macOS: CoreGraphics + CoreText), vẽ vào surface BGRA premultiplied. 1 đơn vị = 1 physical px.

use chup_core::icons::Icon;
use image::RgbaImage;

use crate::{Color, RectF};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Leading,
    Center,
}

/// Font cần dùng (vỏ OS tạo font native theo đây): cỡ physical px, đậm vừa, căn ngang, xuống dòng.
/// Chữ giao diện: Segoe UI (Windows), font hệ thống (macOS).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontSpec {
    pub px: f32,
    pub semibold: bool,
    pub align: Align,
    pub wrap: bool,
}

impl FontSpec {
    pub const fn new(px: f32, align: Align, wrap: bool) -> Self {
        Self {
            px,
            semibold: false,
            align,
            wrap,
        }
    }
}

/// Đo chữ của một font (kích thước theo physical px). Font do vỏ OS tạo.
pub trait FontMetrics {
    /// (rộng, cao) của chữ khi giới hạn bề rộng `max_w`, đã làm tròn lên.
    fn measure(&self, text: &str, max_w: f32) -> (f32, f32);
}

/// Đích vẽ gắn với một surface. Phương thức nhận `&self` (API native giữ trạng thái bên trong).
/// Chữ căn giữa theo chiều dọc trong `r` (và căn ngang theo `Align` của font).
pub trait Canvas {
    type Font: FontMetrics;
    /// Ảnh đã tải lên đích vẽ (Direct2D bitmap, CGImage).
    type Image;

    fn image(&self, img: &RgbaImage) -> Option<Self::Image>;
    fn image_size(&self, img: &Self::Image) -> (f32, f32);

    /// Bắt đầu vẽ, xoá cả surface về `clear`.
    fn begin(&self, clear: Color);
    /// Bắt đầu vẽ, giữ nguyên pixel đang có (vẽ chồng lên).
    fn begin_over(&self);
    fn end(&self);
    /// Chỉ vẽ trong `r` cho tới `pop_clip`.
    fn push_clip(&self, r: RectF);
    fn pop_clip(&self);

    fn fill_round(&self, r: RectF, radius: f32, c: Color);
    fn stroke_round(&self, r: RectF, radius: f32, width: f32, c: Color);
    fn text(&self, text: &str, font: &Self::Font, r: RectF, c: Color);
    /// Icon 24×24 (nét dày 2, đầu tròn) vào ô vuông cạnh `size` tại (x, y).
    fn icon(&self, icon: Icon, x: f32, y: f32, size: f32, c: Color);
    /// Tô `r` (bo góc `radius`) bằng ảnh đặt góc trên-trái tại `origin`, phóng `scale` lần; ảnh ngoài `r` bị cắt.
    fn image_at(&self, img: &Self::Image, r: RectF, radius: f32, origin: (f32, f32), scale: f32);

    /// Bóng đổ giả: vài lớp bo góc mờ dần quanh hình (rẻ, giống nhau trên mọi OS).
    fn shadow(&self, r: RectF, radius: f32, spread: f32, dy: f32, c: Color) {
        let steps = spread.max(1.0).round() as i32;
        for i in (1..=steps).rev() {
            let k = i as f32;
            let a = c.a * (1.0 - k / (steps as f32 + 1.0)) / steps as f32 * 2.0;
            let rr = RectF {
                left: r.left - k,
                top: r.top - k + dy,
                right: r.right + k,
                bottom: r.bottom + k + dy,
            };
            self.fill_round(rr, radius + k, c.with_alpha(a));
        }
    }

    /// Tô `r` (bo góc) bằng ảnh, cắt giữa kiểu `object-fit: cover`.
    fn image_cover(&self, img: &Self::Image, r: RectF, radius: f32) {
        let (bw, bh) = self.image_size(img);
        if bw <= 0.0 || bh <= 0.0 {
            return;
        }
        let (dw, dh) = (r.w(), r.h());
        let k = (dw / bw).max(dh / bh);
        let tx = r.left + (dw - bw * k) / 2.0;
        let ty = r.top + (dh - bh * k) / 2.0;
        self.image_at(img, r, radius, (tx, ty), k);
    }
}

/// Kích thước "viên thuốc" (chữ trên nền bo tròn) cho `text`, đã làm tròn lên.
pub fn pill_size<F: FontMetrics>(font: &F, text: &str, pad_x: f32, pad_y: f32) -> (f32, f32) {
    let (tw, th) = font.measure(text, 10_000.0);
    ((tw + pad_x * 2.0).ceil(), (th + pad_y * 2.0).ceil())
}

#[allow(clippy::too_many_arguments)]
/// Vẽ "viên thuốc" cỡ (w, h) ở góc trên-trái (giữa `begin` và `end`). Font nên căn giữa.
pub fn paint_pill<C: Canvas>(
    c: &C,
    font: &C::Font,
    text: &str,
    w: f32,
    h: f32,
    radius: f32,
    bg: Color,
    fg: Color,
) {
    let r = crate::rectf(0.0, 0.0, w, h);
    c.fill_round(r, radius.min(h / 2.0), bg);
    c.text(text, font, r, fg);
}
