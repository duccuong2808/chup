//! Vẽ trên macOS.
//!
//! - `Surface`: buffer BGRA premultiplied (top-down, đúng layout của `CGBitmapContext` với
//!   `PremultipliedFirst | ByteOrder32Little`) kèm bitmap context trên chính buffer đó. Pixel op của `chup_ui` và
//!   CoreGraphics dùng chung một bộ nhớ. Chỉ tồn tại khi cửa sổ đang hiện.
//! - `Canvas`: cài `chup_ui::Canvas` bằng CoreGraphics + CoreText. Lật trục y một lần (`begin`) để toạ độ tính từ
//!   góc trên-trái như Windows. 1 đơn vị = 1 physical px.
//! - `Font`: CoreText, font hệ thống. Ngắt dòng và căn giữa theo chiều dọc như DirectWrite.

use std::{cell::RefCell, ffi::c_void};

use chup_core::{
    coords::Rect,
    icons::{Icon, Shape},
};
pub use chup_ui::{rectf, Align, Color, FontSpec, RectF, Theme};
use chup_ui::{path::Seg, FontMetrics, Pixels, PixelsRef};
use image::RgbaImage;
use objc2_core_foundation::{
    CFAttributedString, CFBoolean, CFData, CFDictionary, CFIndex, CFRange, CFRetained, CFString, CFType,
    CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapInfo, CGImageByteOrderInfo, CGColorRenderingIntent, CGColorSpace, CGContext,
    CGDataProvider, CGImage, CGImageAlphaInfo, CGInterpolationQuality, CGLineCap, CGLineJoin, CGMutablePath, CGPath,
};
use objc2_core_text::{
    kCTFontAttributeName, kCTForegroundColorFromContextAttributeName, CTFont, CTFontUIFontType, CTLine, CTTypesetter,
};

/// Bảng màu sáng/tối theo macOS.
pub fn theme() -> Theme {
    Theme::new(super::util::dark_mode())
}

fn bitmap_info() -> u32 {
    CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0
}

fn srgb() -> Option<CFRetained<CGColorSpace>> {
    CGColorSpace::with_name(Some(unsafe { objc2_core_graphics::kCGColorSpaceSRGB }))
}

fn cg_rect(r: RectF) -> CGRect {
    CGRect::new(CGPoint::new(r.left as CGFloat, r.top as CGFloat), CGSize::new(r.w().max(0.0) as CGFloat, r.h().max(0.0) as CGFloat))
}

/// CGImage từ buffer BGRA premultiplied (chép dữ liệu: buffer gốc vẽ tiếp được mà không ảnh hưởng ảnh).
/// `stride` = số pixel mỗi hàng của buffer; lấy góc trên-trái cỡ (w, h).
pub fn image_from_bgra(data: &[u32], stride: usize, w: usize, h: usize) -> Option<CFRetained<CGImage>> {
    if w == 0 || h == 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &data[y * stride..y * stride + w];
        // SAFETY: u32 -> 4 byte little-endian, đúng thứ tự BGRA.
        bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(row.as_ptr() as *const u8, w * 4) });
    }
    let cf = CFData::from_bytes(&bytes);
    let provider = CGDataProvider::with_cf_data(Some(&cf))?;
    let cs = srgb()?;
    unsafe {
        CGImage::new(
            w,
            h,
            8,
            32,
            w * 4,
            Some(&cs),
            CGBitmapInfo(bitmap_info()),
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

// ---------- Surface ----------

pub struct Surface {
    buf: Vec<u32>,
    ctx: CFRetained<CGContext>,
    pub w: i32,
    pub h: i32,
}

impl Surface {
    pub fn new(w: i32, h: i32) -> Option<Self> {
        let (w, h) = (w.max(1), h.max(1));
        let mut buf = Vec::new();
        buf.try_reserve_exact((w as usize) * (h as usize)).ok()?;
        buf.resize((w * h) as usize, 0u32);
        let cs = srgb()?;
        // SAFETY: buffer sống cùng Surface và không đổi chỗ (không bao giờ resize sau khi tạo).
        let ctx = unsafe {
            CGBitmapContextCreate(buf.as_mut_ptr() as *mut c_void, w as usize, h as usize, 8, w as usize * 4, Some(&cs), bitmap_info())
        }?;
        Some(Self { buf, ctx, w, h })
    }

    pub fn pixels(&mut self) -> &mut [u32] {
        &mut self.buf
    }

    pub fn data(&self) -> &[u32] {
        &self.buf
    }

    /// View pixel để vẽ trực tiếp (overlay, frame).
    pub fn px(&mut self) -> Pixels<'_> {
        let (w, h) = (self.w, self.h);
        Pixels::new(&mut self.buf, w, h)
    }

    pub fn as_ref(&self) -> PixelsRef<'_> {
        PixelsRef { data: &self.buf, w: self.w, h: self.h }
    }

    pub fn fill(&mut self, r: Rect, px: u32) {
        self.px().fill(r, px);
    }

    /// Chép `src` lên (x, y) bằng phép "over", chỉ trong `clip`.
    pub fn blend_from(&mut self, src: &Surface, x: i32, y: i32, clip: Rect) {
        self.blend_part(src, (src.w, src.h), x, y, clip)
    }

    pub fn blend_part(&mut self, src: &Surface, size: (i32, i32), x: i32, y: i32, clip: Rect) {
        self.px().blend_part(src.as_ref(), size, (x, y), clip);
    }

    /// Ảnh của góc trên-trái cỡ `size` (để đưa lên layer của cửa sổ).
    pub fn image(&self, size: (i32, i32)) -> Option<CFRetained<CGImage>> {
        let (w, h) = (size.0.clamp(1, self.w) as usize, size.1.clamp(1, self.h) as usize);
        image_from_bgra(&self.buf, self.w as usize, w, h)
    }
}

/// Ảnh RGBA thẳng của cả surface (bỏ premultiply).
pub fn to_rgba_image(s: &Surface) -> Option<RgbaImage> {
    let mut out = Vec::with_capacity(s.buf.len() * 4);
    for &p in &s.buf {
        let a = p >> 24;
        let ch = |shift: u32| {
            let c = (p >> shift) & 0xFF;
            (c * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8)
        };
        out.extend_from_slice(&[ch(16), ch(8), ch(0), a as u8]);
    }
    RgbaImage::from_raw(s.w as u32, s.h as u32, out)
}

// ---------- Font ----------

pub struct Font {
    font: CFRetained<CTFont>,
    align: Align,
    wrap: bool,
    ascent: f64,
    line_h: f64,
}

impl Font {
    pub fn new(px: f32, semibold: bool, align: Align, wrap: bool) -> Option<Font> {
        let kind = if semibold { CTFontUIFontType::EmphasizedSystem } else { CTFontUIFontType::System };
        let font = unsafe { CTFont::new_ui_font_for_language(kind, px as CGFloat, None) }?;
        let (ascent, descent, leading) = unsafe { (font.ascent(), font.descent(), font.leading()) };
        Some(Font { font, align, wrap, ascent, line_h: (ascent + descent + leading).ceil() })
    }

    pub fn from_spec(s: FontSpec) -> Option<Font> {
        Self::new(s.px, s.semibold, s.align, s.wrap)
    }

    fn attributed(&self, text: &str) -> Option<CFRetained<CFAttributedString>> {
        let s = CFString::from_str(text);
        let keys: [&CFString; 2] = unsafe { [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName] };
        let values: [&CFType; 2] = [&self.font, CFBoolean::new(true)];
        let dict = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
        unsafe { CFAttributedString::new(None, Some(&s), Some(dict.as_opaque())) }
    }

    /// Các dòng của `text` (ngắt theo `max_w` nếu font xuống dòng) và bề rộng từng dòng.
    fn lines(&self, text: &str, max_w: f32) -> Vec<(CFRetained<CTLine>, f64)> {
        let Some(attr) = self.attributed(text) else { return vec![] };
        let width = |l: &CTLine| unsafe { l.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        if !self.wrap {
            let l = unsafe { CTLine::with_attributed_string(&attr) };
            let w = width(&l);
            return vec![(l, w)];
        }
        let ts = unsafe { CTTypesetter::with_attributed_string(&attr) };
        let len = attr.length();
        let mut out = vec![];
        let mut start: CFIndex = 0;
        while start < len {
            let mut n = unsafe { ts.suggest_line_break(start, max_w.max(1.0) as f64) };
            if n <= 0 {
                n = 1;
            }
            let l = unsafe { ts.line(CFRange { location: start, length: n }) };
            // Không tính khoảng trắng cuối dòng khi xuống dòng (giống DirectWrite khi ngắt).
            let w = width(&l) - unsafe { l.trailing_whitespace_width() };
            out.push((l, w));
            start += n;
        }
        out
    }
}

impl FontMetrics for Font {
    fn measure(&self, text: &str, max_w: f32) -> (f32, f32) {
        let lines = self.lines(text, max_w);
        let w = lines.iter().map(|(_, w)| *w).fold(0.0, f64::max);
        (w.ceil() as f32, (self.line_h * lines.len().max(1) as f64).ceil() as f32)
    }
}

/// Tạo sẵn font hệ thống để lần hiện overlay đầu tiên không chậm.
pub fn warm_up() {
    let t0 = std::time::Instant::now();
    if let Some(f) = Font::new(13.0, false, Align::Leading, false) {
        let _ = f.measure("Chụp", 100.0);
    }
    log::debug!("làm nóng CoreText mất {:?}", t0.elapsed());
}

// ---------- Canvas ----------

/// Đích vẽ CoreGraphics gắn với 1 `Surface`.
pub struct Canvas {
    ctx: RefCell<CFRetained<CGContext>>,
    h: std::cell::Cell<f64>,
}

impl Canvas {
    pub fn new(s: &Surface) -> Option<Canvas> {
        Some(Canvas { ctx: RefCell::new(s.ctx.clone()), h: std::cell::Cell::new(s.h as f64) })
    }

    fn c(&self) -> std::cell::Ref<'_, CFRetained<CGContext>> {
        self.ctx.borrow()
    }

    fn fill_color(ctx: &CGContext, c: Color) {
        CGContext::set_rgb_fill_color(Some(ctx), c.r as f64, c.g as f64, c.b as f64, c.a as f64);
    }

    fn stroke_color(ctx: &CGContext, c: Color) {
        CGContext::set_rgb_stroke_color(Some(ctx), c.r as f64, c.g as f64, c.b as f64, c.a as f64);
    }

    /// Hình chữ nhật bo góc (bán kính được kẹp: CoreGraphics không nhận bán kính quá nửa cạnh).
    fn round_path(r: RectF, radius: f32) -> Option<CFRetained<CGPath>> {
        let rect = cg_rect(r);
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return None;
        }
        let rad = (radius as f64).max(0.0).min(rect.size.width / 2.0).min(rect.size.height / 2.0);
        Some(unsafe { CGPath::with_rounded_rect(rect, rad, rad, std::ptr::null()) })
    }

    fn begin_flipped(&self) {
        let c = self.c();
        CGContext::save_g_state(Some(&c));
        CGContext::translate_ctm(Some(&c), 0.0, self.h.get());
        CGContext::scale_ctm(Some(&c), 1.0, -1.0);
        // Chữ: lật lại để không bị ngược.
        CGContext::set_text_matrix(Some(&c), CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: -1.0, tx: 0.0, ty: 0.0 });
        CGContext::set_should_antialias(Some(&c), true);
    }
}

fn add_segments(path: &CGMutablePath, segs: &[Seg]) {
    for s in segs {
        unsafe {
            match *s {
                Seg::Move(x, y) => CGMutablePath::move_to_point(Some(path), std::ptr::null(), x as f64, y as f64),
                Seg::Line(x, y) => CGMutablePath::add_line_to_point(Some(path), std::ptr::null(), x as f64, y as f64),
                Seg::Cubic(a, b, p) => CGMutablePath::add_curve_to_point(
                    Some(path),
                    std::ptr::null(),
                    a.0 as f64,
                    a.1 as f64,
                    b.0 as f64,
                    b.1 as f64,
                    p.0 as f64,
                    p.1 as f64,
                ),
                Seg::Close => CGMutablePath::close_subpath(Some(path)),
            }
        }
    }
}

impl chup_ui::Canvas for Canvas {
    type Font = Font;
    type Image = CFRetained<CGImage>;

    fn image(&self, img: &RgbaImage) -> Option<CFRetained<CGImage>> {
        let (w, h) = img.dimensions();
        let buf = chup_ui::pixels::premultiply(img);
        image_from_bgra(&buf, w as usize, w as usize, h as usize)
    }

    fn image_size(&self, img: &CFRetained<CGImage>) -> (f32, f32) {
        (CGImage::width(Some(img)) as f32, CGImage::height(Some(img)) as f32)
    }

    fn begin(&self, clear: Color) {
        {
            let c = self.c();
            let w = objc2_core_graphics::CGBitmapContextGetWidth(Some(&c)) as f64;
            CGContext::clear_rect(Some(&c), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w, self.h.get())));
            if clear.a > 0.0 {
                Self::fill_color(&c, clear);
                CGContext::fill_rect(Some(&c), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w, self.h.get())));
            }
        }
        self.begin_flipped();
    }

    fn begin_over(&self) {
        self.begin_flipped();
    }

    fn end(&self) {
        let c = self.c();
        CGContext::restore_g_state(Some(&c));
        CGContext::flush(Some(&c));
    }

    fn push_clip(&self, r: RectF) {
        let c = self.c();
        CGContext::save_g_state(Some(&c));
        CGContext::clip_to_rect(Some(&c), cg_rect(r));
    }

    fn pop_clip(&self) {
        CGContext::restore_g_state(Some(&self.c()));
    }

    fn fill_round(&self, r: RectF, radius: f32, col: Color) {
        let Some(p) = Self::round_path(r, radius) else { return };
        let c = self.c();
        Self::fill_color(&c, col);
        CGContext::add_path(Some(&c), Some(&p));
        CGContext::fill_path(Some(&c));
    }

    fn stroke_round(&self, r: RectF, radius: f32, width: f32, col: Color) {
        let Some(p) = Self::round_path(r, radius) else { return };
        let c = self.c();
        Self::stroke_color(&c, col);
        CGContext::set_line_width(Some(&c), width as f64);
        CGContext::add_path(Some(&c), Some(&p));
        CGContext::stroke_path(Some(&c));
    }

    fn text(&self, text: &str, font: &Font, r: RectF, col: Color) {
        let lines = font.lines(text, r.w());
        if lines.is_empty() {
            return;
        }
        let c = self.c();
        Self::fill_color(&c, col);
        let total = font.line_h * lines.len() as f64;
        // Căn giữa theo chiều dọc như DWRITE_PARAGRAPH_ALIGNMENT_CENTER.
        let mut y = r.top as f64 + (r.h() as f64 - total) / 2.0;
        for (line, w) in &lines {
            let x = match font.align {
                Align::Leading => r.left as f64,
                Align::Center => r.left as f64 + (r.w() as f64 - w) / 2.0,
            };
            CGContext::set_text_position(Some(&c), x, y + font.ascent);
            unsafe { line.draw(&c) };
            y += font.line_h;
        }
    }

    fn icon(&self, icon: Icon, x: f32, y: f32, size: f32, col: Color) {
        let c = self.c();
        let k = size as f64 / 24.0;
        CGContext::save_g_state(Some(&c));
        CGContext::translate_ctm(Some(&c), x as f64, y as f64);
        CGContext::scale_ctm(Some(&c), k, k);
        Self::stroke_color(&c, col);
        Self::fill_color(&c, col);
        CGContext::set_line_width(Some(&c), 2.0);
        CGContext::set_line_cap(Some(&c), CGLineCap::Round);
        CGContext::set_line_join(Some(&c), CGLineJoin::Round);
        for shape in icon.shapes() {
            match shape {
                Shape::Circle { cx, cy, r } => {
                    let rect = CGRect::new(CGPoint::new((cx - r) as f64, (cy - r) as f64), CGSize::new(2.0 * r as f64, 2.0 * r as f64));
                    CGContext::stroke_ellipse_in_rect(Some(&c), rect);
                }
                Shape::Rect { x, y, w, h, rx } => {
                    if let Some(p) = Self::round_path(rectf(x, y, w, h), rx) {
                        CGContext::add_path(Some(&c), Some(&p));
                        CGContext::stroke_path(Some(&c));
                    }
                }
                Shape::Stroke(cmds) => {
                    let p = CGMutablePath::new();
                    add_segments(&p, &chup_ui::path::segments(&cmds));
                    CGContext::add_path(Some(&c), Some(&p));
                    CGContext::stroke_path(Some(&c));
                }
                Shape::Fill(cmds) => {
                    let p = CGMutablePath::new();
                    add_segments(&p, &chup_ui::path::segments(&cmds));
                    CGContext::add_path(Some(&c), Some(&p));
                    CGContext::draw_path(Some(&c), objc2_core_graphics::CGPathDrawingMode::FillStroke);
                }
            }
        }
        CGContext::restore_g_state(Some(&c));
    }

    fn image_at(&self, img: &CFRetained<CGImage>, r: RectF, radius: f32, origin: (f32, f32), scale: f32) {
        let Some(clip) = Self::round_path(r, radius) else { return };
        let (iw, ih) = self.image_size(img);
        let (dw, dh) = ((iw * scale) as f64, (ih * scale) as f64);
        let c = self.c();
        CGContext::save_g_state(Some(&c));
        CGContext::add_path(Some(&c), Some(&clip));
        CGContext::clip(Some(&c));
        CGContext::set_interpolation_quality(Some(&c), CGInterpolationQuality::High);
        // Đang lật trục y: lật lại cục bộ để ảnh không bị ngược.
        CGContext::translate_ctm(Some(&c), origin.0 as f64, origin.1 as f64 + dh);
        CGContext::scale_ctm(Some(&c), 1.0, -1.0);
        CGContext::draw_image(Some(&c), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(dw, dh)), Some(img));
        CGContext::restore_g_state(Some(&c));
    }
}

/// Vẽ chữ trong "viên thuốc" thành surface riêng (dòng gợi ý của overlay).
pub fn pill(text: &str, font_px: f32, pad_x: f32, pad_y: f32, radius: f32, bg: Color, fg: Color) -> Option<Surface> {
    use chup_ui::Canvas as _;
    let font = Font::new(font_px, false, Align::Center, false)?;
    let (w, h) = chup_ui::canvas::pill_size(&font, text, pad_x, pad_y);
    let s = Surface::new(w as i32, h as i32)?;
    let c = Canvas::new(&s)?;
    c.begin(Color::CLEAR);
    chup_ui::canvas::paint_pill(&c, &font, text, w, h, radius, bg, fg);
    c.end();
    Some(s)
}

/// Vẽ lại "viên thuốc" nhiều lần vào cùng 1 bộ đệm (nhãn W × H khi kéo chuột).
pub struct PillPainter {
    font: Font,
    pub surface: Surface,
    canvas: Canvas,
    pad: (f32, f32),
    radius: f32,
}

impl PillPainter {
    pub fn new(font_px: f32, pad_x: f32, pad_y: f32, radius: f32, max: (i32, i32)) -> Option<Self> {
        let font = Font::new(font_px, false, Align::Center, false)?;
        let surface = Surface::new(max.0, max.1)?;
        let canvas = Canvas::new(&surface)?;
        Some(Self { font, surface, canvas, pad: (pad_x, pad_y), radius })
    }

    /// Vẽ `text`, trả về kích thước dùng được (góc trên-trái của surface).
    pub fn paint(&mut self, text: &str, bg: Color, fg: Color) -> (i32, i32) {
        use chup_ui::Canvas as _;
        let (pw, ph) = chup_ui::canvas::pill_size(&self.font, text, self.pad.0, self.pad.1);
        let w = (pw as i32).min(self.surface.w);
        let h = (ph as i32).min(self.surface.h);
        self.canvas.begin(Color::CLEAR);
        chup_ui::canvas::paint_pill(&self.canvas, &self.font, text, w as f32, h as f32, self.radius, bg, fg);
        self.canvas.end();
        (w, h)
    }
}
