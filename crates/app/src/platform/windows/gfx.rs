//! Vẽ cho layered window.
//!
//! - `Surface`: DIB 32-bit premultiplied BGRA + memory DC, đẩy lên cửa sổ bằng
//!   `UpdateLayeredWindowIndirect` (chỉ vùng bẩn). Chỉ tồn tại khi cửa sổ đang hiện.
//! - Overlay và frame vẽ bằng thao tác pixel trực tiếp (tô hình chữ nhật) vì phủ cả màn hình:
//!   rẻ hơn Direct2D nhiều ở 4K.
//! - Toolbar, stack và chữ dùng Direct2D **software** (không tạo device GPU, đỡ RAM) + DirectWrite.

use std::cell::RefCell;

use chup_core::{
    coords::Rect,
    icons::{Cmd, Icon, Shape},
};
use windows::{
    core::{w, Interface},
    Win32::{
        Foundation::{COLORREF, HWND, POINT, RECT, SIZE},
        Graphics::{
            Direct2D::{
                Common::{
                    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW,
                    D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_F,
                    D2D_SIZE_U,
                },
                D2D1CreateFactory, ID2D1Bitmap, ID2D1BitmapBrush, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory,
                ID2D1Geometry, ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE,
                D2D1_ARC_SIZE_SMALL, D2D1_BITMAP_BRUSH_PROPERTIES, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                D2D1_BITMAP_PROPERTIES, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID, D2D1_DRAW_TEXT_OPTIONS_NONE,
                D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_OPTIONS, D2D1_FACTORY_TYPE_SINGLE_THREADED,
                D2D1_FEATURE_LEVEL_DEFAULT, D2D1_LINE_JOIN_ROUND, D2D1_RENDER_TARGET_PROPERTIES,
                D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
                D2D1_STROKE_STYLE_PROPERTIES, D2D1_SWEEP_DIRECTION_CLOCKWISE,
                D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
            },
            DirectWrite::{
                DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_FACTORY_TYPE_SHARED,
                DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS,
                DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::{
                CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, SelectObject, AC_SRC_ALPHA,
                AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC,
                HGDIOBJ,
            },
        },
        UI::WindowsAndMessaging::{UpdateLayeredWindowIndirect, ULW_ALPHA, UPDATELAYEREDWINDOWINFO},
    },
};
use windows_numerics::{Matrix3x2, Vector2};

use super::util::win_rect;

// ---------- màu ----------

/// Màu thẳng (chưa premultiply), 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
    }
    pub const fn hex(v: u32) -> Self {
        Self::rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 1.0)
    }
    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }
    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.r, g: self.g, b: self.b, a: self.a }
    }
    /// Pixel premultiplied BGRA (u32 little-endian = 0xAARRGGBB).
    pub fn px(self) -> u32 {
        let a = (self.a.clamp(0.0, 1.0) * 255.0).round() as u32;
        let c = |v: f32| ((v.clamp(0.0, 1.0) * a as f32).round() as u32).min(a);
        (a << 24) | (c(self.r) << 16) | (c(self.g) << 8) | c(self.b)
    }
}

/// Bảng màu (giống design token của bản web cũ). Sáng/tối theo Windows.
#[derive(Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    pub accent_strong: Color,
    pub danger: Color,
    pub surface: Color,
    pub surface2: Color,
    pub text: Color,
    pub line: Color,
    pub shadow: Color,
}

impl Theme {
    pub fn current() -> Self {
        let accent = Color::hex(0x1f6feb);
        let accent_strong = Color::hex(0x1858c4);
        let danger = Color::hex(0xd1242f);
        if super::util::dark_mode() {
            Theme {
                accent,
                accent_strong,
                danger,
                surface: Color::rgba(32, 34, 38, 0.96),
                surface2: Color::hex(0x2a2d33),
                text: Color::hex(0xe8eaed),
                line: Color::rgba(255, 255, 255, 0.12),
                shadow: Color::rgba(0, 0, 0, 0.5),
            }
        } else {
            Theme {
                accent,
                accent_strong,
                danger,
                surface: Color::rgba(255, 255, 255, 0.96),
                surface2: Color::hex(0xf3f4f6),
                text: Color::hex(0x1b1f24),
                line: Color::rgba(27, 31, 36, 0.12),
                shadow: Color::rgba(0, 0, 0, 0.18),
            }
        }
    }
}

// ---------- Surface ----------

pub struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    pub w: i32,
    pub h: i32,
}

impl Surface {
    pub fn new(w: i32, h: i32) -> Option<Self> {
        let (w, h) = (w.max(1), h.max(1));
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = match CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(b) if !bits.is_null() => b,
                _ => {
                    let _ = DeleteDC(dc);
                    return None;
                }
            };
            let old = SelectObject(dc, bmp.into());
            Some(Self { dc, bmp, old, bits: bits as *mut u32, w, h })
        }
    }

    pub fn dc(&self) -> HDC {
        self.dc
    }

    pub fn pixels(&mut self) -> &mut [u32] {
        unsafe {
            let _ = GdiFlush();
            std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize)
        }
    }

    /// Tô `r` (toạ độ trong surface) bằng 1 pixel premultiplied, cắt theo biên.
    pub fn fill(&mut self, r: Rect, px: u32) {
        let Some(r) = r.intersect(&Rect::new(0, 0, self.w as u32, self.h as u32)) else { return };
        let w = self.w as usize;
        let buf = self.pixels();
        for y in r.y..r.bottom() {
            let row = y as usize * w;
            buf[row + r.x as usize..row + r.right() as usize].fill(px);
        }
    }

    /// Viền hình chữ nhật dày `t` nằm **bên ngoài** `r`.
    pub fn outline_outside(&mut self, r: Rect, t: i32, px: u32) {
        let o = r.expand(t);
        self.fill(Rect::new(o.x, o.y, o.w, t as u32), px);
        self.fill(Rect::new(o.x, r.bottom(), o.w, t as u32), px);
        self.fill(Rect::new(o.x, r.y, t as u32, r.h), px);
        self.fill(Rect::new(r.right(), r.y, t as u32, r.h), px);
    }

    /// Viền nét đứt dày `t` nằm bên ngoài `r`: các đoạn dài `dash` xen kẽ màu `a` và `b`
    /// (2 màu tương phản để nhìn rõ trên mọi nền).
    pub fn outline_dashed_outside(&mut self, r: Rect, t: i32, dash: i32, a: u32, b: u32) {
        let o = r.expand(t);
        let dash = dash.max(1);
        let (mut x, mut i) = (o.x, 0);
        while x < o.right() {
            let len = dash.min(o.right() - x) as u32;
            let c = if i % 2 == 0 { a } else { b };
            self.fill(Rect::new(x, o.y, len, t as u32), c);
            self.fill(Rect::new(x, r.bottom(), len, t as u32), c);
            x += dash;
            i += 1;
        }
        let (mut y, mut i) = (r.y, 1);
        while y < r.bottom() {
            let len = dash.min(r.bottom() - y) as u32;
            let c = if i % 2 == 0 { a } else { b };
            self.fill(Rect::new(o.x, y, t as u32, len), c);
            self.fill(Rect::new(r.right(), y, t as u32, len), c);
            y += dash;
            i += 1;
        }
    }

    /// Chép `src` (premultiplied) lên vị trí (x, y) bằng phép "over".
    pub fn blend_from(&mut self, src: &mut Surface, x: i32, y: i32) {
        let (w, h) = (src.w, src.h);
        self.blend_part(src, (w, h), x, y)
    }

    /// Như `blend_from` nhưng chỉ lấy góc trên-trái cỡ `size` của `src`.
    pub fn blend_part(&mut self, src: &mut Surface, size: (i32, i32), x: i32, y: i32) {
        let dst_bounds = Rect::new(0, 0, self.w as u32, self.h as u32);
        let (pw, ph) = (size.0.min(src.w), size.1.min(src.h));
        let Some(area) = Rect::new(x, y, pw as u32, ph as u32).intersect(&dst_bounds) else { return };
        let (sw, dw) = (src.w as usize, self.w as usize);
        let s = src.pixels().to_vec();
        let d = self.pixels();
        for yy in area.y..area.bottom() {
            for xx in area.x..area.right() {
                let sp = s[(yy - y) as usize * sw + (xx - x) as usize];
                let sa = sp >> 24;
                if sa == 0 {
                    continue;
                }
                let di = yy as usize * dw + xx as usize;
                d[di] = if sa == 255 { sp } else { over(sp, d[di]) };
            }
        }
    }

    /// Đẩy lên layered window. `dirty` (toạ độ surface) = chỉ cập nhật phần đó.
    pub fn present(&self, hwnd: HWND, pos: (i32, i32), alpha: u8, dirty: Option<Rect>) {
        self.present_ex(hwnd, pos, (self.w, self.h), alpha, dirty)
    }

    /// Đẩy phần góc trên-trái cỡ `size` (≤ surface) lên cửa sổ; cửa sổ nhận đúng `size`.
    pub fn present_part(&self, hwnd: HWND, pos: (i32, i32), size: (i32, i32), alpha: u8) {
        let size = (size.0.clamp(1, self.w), size.1.clamp(1, self.h));
        self.present_ex(hwnd, pos, size, alpha, None)
    }

    fn present_ex(&self, hwnd: HWND, pos: (i32, i32), size: (i32, i32), alpha: u8, dirty: Option<Rect>) {
        let dst = POINT { x: pos.0, y: pos.1 };
        let size = SIZE { cx: size.0, cy: size.1 };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let dirty_rect: Option<RECT> =
            dirty.and_then(|d| d.intersect(&Rect::new(0, 0, self.w as u32, self.h as u32))).map(|d| win_rect(&d));
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            hdcDst: HDC::default(),
            pptDst: &dst,
            psize: &size,
            hdcSrc: self.dc,
            pptSrc: &src,
            crKey: COLORREF(0),
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: dirty_rect.as_ref().map_or(std::ptr::null(), |r| r as *const RECT),
        };
        unsafe {
            let _ = GdiFlush();
            let _ = UpdateLayeredWindowIndirect(hwnd, &info);
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// src over dst, cả hai premultiplied.
fn over(s: u32, d: u32) -> u32 {
    let inv = 255 - (s >> 24);
    let ch = |shift: u32| {
        let sc = (s >> shift) & 0xFF;
        let dc = (d >> shift) & 0xFF;
        (sc + (dc * inv + 127) / 255).min(255) << shift
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

// ---------- Direct2D ----------

struct Factories {
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    round: ID2D1StrokeStyle,
}

thread_local! {
    static FACTORIES: RefCell<Option<Factories>> = const { RefCell::new(None) };
}

fn with_factories<T>(f: impl FnOnce(&Factories) -> T) -> Option<T> {
    FACTORIES.with(|cell| {
        if cell.borrow().is_none() {
            let made = (|| -> windows::core::Result<Factories> {
                let d2d: ID2D1Factory = unsafe {
                    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&D2D1_FACTORY_OPTIONS::default()))?
                };
                let dw: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
                let round = unsafe {
                    d2d.CreateStrokeStyle(
                        &D2D1_STROKE_STYLE_PROPERTIES {
                            startCap: D2D1_CAP_STYLE_ROUND,
                            endCap: D2D1_CAP_STYLE_ROUND,
                            dashCap: D2D1_CAP_STYLE_ROUND,
                            lineJoin: D2D1_LINE_JOIN_ROUND,
                            miterLimit: 10.0,
                            dashStyle: D2D1_DASH_STYLE_SOLID,
                            dashOffset: 0.0,
                        },
                        None,
                    )?
                };
                Ok(Factories { d2d, dw, round })
            })();
            match made {
                Ok(f) => *cell.borrow_mut() = Some(f),
                Err(e) => {
                    log::error!("Không tạo được Direct2D/DirectWrite: {e}");
                    return None;
                }
            }
        }
        cell.borrow().as_ref().map(f)
    })
}

/// Tạo sẵn factory Direct2D/DirectWrite và nạp font, để lần hiện overlay đầu tiên không chậm.
pub fn warm_up() {
    let t0 = std::time::Instant::now();
    if let Some(f) = Font::new(13.0, false, Align::Leading, false) {
        let _ = f.measure("Chụp", 100.0);
    }
    log::debug!("làm nóng Direct2D/DirectWrite mất {:?}", t0.elapsed());
}

pub fn rectf(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

/// Font chữ (DirectWrite). Kích thước theo physical px.
pub struct Font {
    fmt: IDWriteTextFormat,
}

#[derive(Clone, Copy)]
pub enum Align {
    Leading,
    Center,
}

impl Font {
    pub fn new(px: f32, semibold: bool, align: Align, wrap: bool) -> Option<Font> {
        with_factories(|f| unsafe {
            let weight: DWRITE_FONT_WEIGHT = if semibold { DWRITE_FONT_WEIGHT_SEMI_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
            let fmt = f
                .dw
                .CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    px,
                    w!("vi-VN"),
                )
                .ok()?;
            let _ = fmt.SetWordWrapping(if wrap { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP });
            let _ = fmt.SetTextAlignment(match align {
                Align::Leading => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            });
            let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            Some(Font { fmt })
        })
        .flatten()
    }

    fn layout(&self, text: &str, max_w: f32, max_h: f32) -> Option<IDWriteTextLayout> {
        let t: Vec<u16> = text.encode_utf16().collect();
        with_factories(|f| unsafe { f.dw.CreateTextLayout(&t, &self.fmt, max_w, max_h).ok() }).flatten()
    }

    /// (rộng, cao) của chữ khi giới hạn bề rộng `max_w`.
    pub fn measure(&self, text: &str, max_w: f32) -> (f32, f32) {
        let Some(l) = self.layout(text, max_w, 10_000.0) else { return (0.0, 0.0) };
        let mut m = DWRITE_TEXT_METRICS::default();
        if unsafe { l.GetMetrics(&mut m) }.is_err() {
            return (0.0, 0.0);
        }
        (m.widthIncludingTrailingWhitespace.ceil(), m.height.ceil())
    }
}

/// Render target Direct2D vẽ thẳng vào 1 `Surface`.
pub struct Canvas {
    rt: ID2D1DCRenderTarget,
}

impl Canvas {
    pub fn new(s: &Surface) -> Option<Canvas> {
        with_factories(|f| unsafe {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                // 96 DPI: 1 đơn vị Direct2D = 1 physical px. Scale tự nhân ở lớp gọi.
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = f.d2d.CreateDCRenderTarget(&props).ok()?;
            rt.BindDC(s.dc(), &RECT { left: 0, top: 0, right: s.w, bottom: s.h }).ok()?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            Some(Canvas { rt })
        })
        .flatten()
    }

    /// Vẽ sang surface khác. Tài nguyên đã tạo (bitmap ảnh) vẫn dùng được: tạo 1 lần, vẽ lên nhiều surface.
    pub fn bind(&self, s: &Surface) -> bool {
        unsafe { self.rt.BindDC(s.dc(), &RECT { left: 0, top: 0, right: s.w, bottom: s.h }).is_ok() }
    }

    pub fn begin(&self, clear: Color) {
        unsafe {
            self.rt.BeginDraw();
            self.rt.SetTransform(&Matrix3x2::identity());
            self.rt.Clear(Some(&clear.d2d()));
        }
    }

    /// Như `begin` nhưng giữ nguyên pixel đang có (vẽ chồng lên).
    pub fn begin_over(&self) {
        unsafe {
            self.rt.BeginDraw();
            self.rt.SetTransform(&Matrix3x2::identity());
        }
    }

    pub fn end(&self) {
        if let Err(e) = unsafe { self.rt.EndDraw(None, None) } {
            log::warn!("Direct2D EndDraw: {e}");
        }
    }

    fn brush(&self, c: Color) -> Option<ID2D1SolidColorBrush> {
        unsafe { self.rt.CreateSolidColorBrush(&c.d2d(), None).ok() }
    }

    pub fn fill_round(&self, r: D2D_RECT_F, radius: f32, c: Color) {
        if let Some(b) = self.brush(c) {
            let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
            unsafe { self.rt.FillRoundedRectangle(&rr, &b) };
        }
    }

    pub fn stroke_round(&self, r: D2D_RECT_F, radius: f32, width: f32, c: Color) {
        if let Some(b) = self.brush(c) {
            let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
            unsafe { self.rt.DrawRoundedRectangle(&rr, &b, width, None) };
        }
    }

    /// Bóng đổ giả: vài lớp viền mờ dần quanh hình (Direct2D software không có blur rẻ).
    pub fn shadow(&self, r: D2D_RECT_F, radius: f32, spread: f32, dy: f32, c: Color) {
        let steps = spread.max(1.0).round() as i32;
        for i in (1..=steps).rev() {
            let k = i as f32;
            let a = c.a * (1.0 - k / (steps as f32 + 1.0)) / steps as f32 * 2.0;
            let rr = D2D_RECT_F { left: r.left - k, top: r.top - k + dy, right: r.right + k, bottom: r.bottom + k + dy };
            self.fill_round(rr, radius + k, c.with_alpha(a));
        }
    }

    pub fn text(&self, text: &str, font: &Font, r: D2D_RECT_F, c: Color) {
        let Some(b) = self.brush(c) else { return };
        let t: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.rt.DrawText(
                &t,
                &font.fmt,
                &r,
                &b,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            )
        };
    }

    /// Vẽ icon 24×24 vào ô vuông cạnh `size` tại (x, y).
    pub fn icon(&self, icon: Icon, x: f32, y: f32, size: f32, c: Color) {
        let Some(b) = self.brush(c) else { return };
        let k = size / 24.0;
        let m = Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x, M32: y };
        with_factories(|f| unsafe {
            self.rt.SetTransform(&m);
            for shape in icon.shapes() {
                match shape {
                    Shape::Circle { cx, cy, r } => {
                        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: r, radiusY: r };
                        self.rt.DrawEllipse(&e, &b, 2.0, &f.round);
                    }
                    Shape::Rect { x, y, w, h, rx } => {
                        let rr = D2D1_ROUNDED_RECT { rect: rectf(x, y, w, h), radiusX: rx, radiusY: rx };
                        self.rt.DrawRoundedRectangle(&rr, &b, 2.0, &f.round);
                    }
                    Shape::Stroke(cmds) => {
                        if let Some(g) = path_geometry(&f.d2d, &cmds, false) {
                            self.rt.DrawGeometry(&g, &b, 2.0, &f.round);
                        }
                    }
                    Shape::Fill(cmds) => {
                        if let Some(g) = path_geometry(&f.d2d, &cmds, true) {
                            self.rt.FillGeometry(&g, &b, None);
                            self.rt.DrawGeometry(&g, &b, 2.0, &f.round);
                        }
                    }
                }
            }
            self.rt.SetTransform(&Matrix3x2::identity());
        });
    }

    /// Bitmap Direct2D từ ảnh RGBA thẳng (đổi sang BGRA premultiplied).
    pub fn bitmap(&self, img: &image::RgbaImage) -> Option<ID2D1Bitmap> {
        let (w, h) = img.dimensions();
        let mut buf = Vec::with_capacity((w * h) as usize);
        for p in img.pixels() {
            let [r, g, b, a] = p.0;
            let pm = |c: u8| (c as u32 * a as u32 + 127) / 255;
            buf.push(((a as u32) << 24) | (pm(r) << 16) | (pm(g) << 8) | pm(b));
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        unsafe {
            self.rt
                .CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(buf.as_ptr() as *const _), w * 4, &props)
                .ok()
        }
    }

    /// Tô `r` (bo góc) bằng ảnh, cắt giữa kiểu `object-fit: cover`.
    pub fn image_cover(&self, bmp: &ID2D1Bitmap, r: D2D_RECT_F, radius: f32) {
        unsafe {
            let D2D_SIZE_F { width: bw, height: bh } = bmp.GetSize();
            if bw <= 0.0 || bh <= 0.0 {
                return;
            }
            let (dw, dh) = (r.right - r.left, r.bottom - r.top);
            let k = (dw / bw).max(dh / bh);
            let tx = r.left + (dw - bw * k) / 2.0;
            let ty = r.top + (dh - bh * k) / 2.0;
            let props = D2D1_BITMAP_BRUSH_PROPERTIES {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            };
            let Ok(brush): windows::core::Result<ID2D1BitmapBrush> = self.rt.CreateBitmapBrush(bmp, Some(&props), None)
            else {
                return;
            };
            brush.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: tx, M32: ty });
            let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
            let b: ID2D1Brush = brush.cast().unwrap();
            self.rt.FillRoundedRectangle(&rr, &b);
        }
    }
}

fn path_geometry(f: &ID2D1Factory, cmds: &[Cmd], filled: bool) -> Option<ID2D1Geometry> {
    unsafe {
        let g = f.CreatePathGeometry().ok()?;
        let sink = g.Open().ok()?;
        let begin = if filled { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW };
        let mut open = false;
        for c in cmds {
            match *c {
                Cmd::Move(x, y) => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    sink.BeginFigure(Vector2 { X: x, Y: y }, begin);
                    open = true;
                }
                Cmd::Line(x, y) => sink.AddLine(Vector2 { X: x, Y: y }),
                Cmd::Arc { rx, ry, rot, large, sweep, x, y } => sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: Vector2 { X: x, Y: y },
                    size: D2D_SIZE_F { width: rx, height: ry },
                    rotationAngle: rot,
                    sweepDirection: if sweep {
                        D2D1_SWEEP_DIRECTION_CLOCKWISE
                    } else {
                        D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                    },
                    arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                }),
                Cmd::Close => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                }
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close().ok()?;
        g.cast().ok()
    }
}

/// Vẽ chữ trong 1 "viên thuốc" (nền bo tròn) thành surface riêng, để ghép vào overlay.
pub fn pill(text: &str, font_px: f32, pad_x: f32, pad_y: f32, radius: f32, bg: Color, fg: Color) -> Option<Surface> {
    let font = Font::new(font_px, false, Align::Center, false)?;
    let (tw, th) = font.measure(text, 10_000.0);
    let (w, h) = ((tw + pad_x * 2.0).ceil(), (th + pad_y * 2.0).ceil());
    let s = Surface::new(w as i32, h as i32)?;
    let c = Canvas::new(&s)?;
    c.begin(Color::rgba(0, 0, 0, 0.0));
    c.fill_round(rectf(0.0, 0.0, w, h), radius.min(h / 2.0), bg);
    c.text(text, &font, rectf(0.0, 0.0, w, h), fg);
    c.end();
    drop(c);
    Some(s)
}

/// Vẽ lại "viên thuốc" nhiều lần vào cùng 1 bộ đệm (nhãn W × H khi kéo chuột):
/// không phải tạo font, DIB và render target mỗi lần chuột di.
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
        let (tw, th) = self.font.measure(text, 10_000.0);
        let w = ((tw + self.pad.0 * 2.0).ceil() as i32).min(self.surface.w);
        let h = ((th + self.pad.1 * 2.0).ceil() as i32).min(self.surface.h);
        self.canvas.begin(Color::rgba(0, 0, 0, 0.0));
        let r = rectf(0.0, 0.0, w as f32, h as f32);
        self.canvas.fill_round(r, self.radius.min(h as f32 / 2.0), bg);
        self.canvas.text(text, &self.font, r, fg);
        self.canvas.end();
        (w, h)
    }
}
