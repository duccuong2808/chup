//! Phần giao diện dùng chung cho mọi OS (SPEC 6, thiết kế `docs/plans/2026-11-05-macos-design.md` mục 1).
//!
//! - Không biết cửa sổ, sự kiện hay API vẽ của OS. Vẽ hình và chữ qua trait [`Canvas`]; vẽ pixel trực tiếp
//!   (overlay, frame) qua [`Pixels`], một view lên bộ nhớ BGRA premultiplied của surface do vỏ OS cấp.
//! - Mọi toạ độ là physical px; kích thước khai báo bằng DIP rồi nhân scale `k` của màn hình.
//! - Vỏ OS giữ: tạo / hiện / ẩn cửa sổ, sự kiện chuột / phím, đẩy surface lên màn hình, và cài đặt `Canvas`.

pub mod canvas;
pub mod color;
pub mod frame;
pub mod geom;
pub mod overlay;
pub mod pixels;
pub mod preview;
pub mod stack;
pub mod toolbar;

#[cfg(test)]
mod testing;

pub use canvas::{Align, Canvas, FontMetrics, FontSpec};
pub use color::{Color, Theme};
pub use geom::{rectf, RectF};
pub use pixels::{Pixels, PixelsRef};
