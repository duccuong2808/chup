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
                ID2D1Geometry, ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE,
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

pub use chup_ui::{rectf, Align, Color, FontSpec, RectF, Theme};
use chup_ui::{FontMetrics, Pixels, PixelsRef};

/// Bảng màu sáng/tối theo Windows.
pub fn theme() -> Theme {
    Theme::new(super::util::dark_mode())
}

fn d2d(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

fn d2r(r: RectF) -> D2D_RECT_F {
    D2D_RECT_F { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
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

    /// View pixel để vẽ trực tiếp (overlay, frame).
    pub fn px(&mut self) -> Pixels<'_> {
        let (w, h) = (self.w, self.h);
        Pixels::new(self.pixels(), w, h)
    }

    /// Chép vùng `r` từ `src` cùng kích thước.
    pub fn copy_rect_from(&mut self, src: &mut Surface, r: Rect) {
        if (src.w, src.h) != (self.w, self.h) {
            return;
        }
        let s = src.pixels();
        self.px().copy_rect_from(s, r);
    }

    pub fn fill(&mut self, r: Rect, px: u32) {
        self.px().fill(r, px);
    }

    /// Viền nét đứt dày `t` nằm bên ngoài `r` (xem `chup_ui::Pixels::outline_dashed_outside`).
    pub fn outline_dashed_outside(&mut self, r: Rect, t: i32, dash: i32, a: u32, b: u32) {
        self.px().outline_dashed_outside(r, t, dash, a, b);
    }

    /// Chép `src` (premultiplied) lên vị trí (x, y) bằng phép "over", chỉ trong `clip`.
    pub fn blend_from(&mut self, src: &mut Surface, x: i32, y: i32, clip: Rect) {
        let (w, h) = (src.w, src.h);
        self.blend_part(src, (w, h), x, y, clip)
    }

    /// Như `blend_from` nhưng chỉ lấy góc trên-trái cỡ `size` của `src` (xem `chup_ui::Pixels::blend_part`).
    pub fn blend_part(&mut self, src: &mut Surface, size: (i32, i32), x: i32, y: i32, clip: Rect) {
        let (sw, sh) = (src.w, src.h);
        let data = src.pixels();
        self.px().blend_part(PixelsRef { data, w: sw, h: sh }, size, (x, y), clip);
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
    use chup_ui::FontMetrics as _;
    let t0 = std::time::Instant::now();
    if let Some(f) = Font::new(13.0, false, Align::Leading, false) {
        let _ = f.measure("Chụp", 100.0);
    }
    log::debug!("làm nóng Direct2D/DirectWrite mất {:?}", t0.elapsed());
}

/// Font chữ (DirectWrite). Kích thước theo physical px.
pub struct Font {
    fmt: IDWriteTextFormat,
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

    pub fn from_spec(s: FontSpec) -> Option<Font> {
        Self::new(s.px, s.semibold, s.align, s.wrap)
    }

    fn layout(&self, text: &str, max_w: f32, max_h: f32) -> Option<IDWriteTextLayout> {
        let t: Vec<u16> = text.encode_utf16().collect();
        with_factories(|f| unsafe { f.dw.CreateTextLayout(&t, &self.fmt, max_w, max_h).ok() }).flatten()
    }

}

impl FontMetrics for Font {
    /// (rộng, cao) của chữ khi giới hạn bề rộng `max_w`.
    fn measure(&self, text: &str, max_w: f32) -> (f32, f32) {
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

    fn brush(&self, c: Color) -> Option<ID2D1SolidColorBrush> {
        unsafe { self.rt.CreateSolidColorBrush(&d2d(c), None).ok() }
    }
}

impl chup_ui::Canvas for Canvas {
    type Font = Font;
    type Image = ID2D1Bitmap;

    /// Bitmap Direct2D từ ảnh RGBA thẳng (đổi sang BGRA premultiplied).
    fn image(&self, img: &image::RgbaImage) -> Option<ID2D1Bitmap> {
        let (w, h) = img.dimensions();
        let buf = chup_ui::pixels::premultiply(img);
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

    fn image_size(&self, img: &ID2D1Bitmap) -> (f32, f32) {
        let D2D_SIZE_F { width, height } = unsafe { img.GetSize() };
        (width, height)
    }

    fn begin(&self, clear: Color) {
        unsafe {
            self.rt.BeginDraw();
            self.rt.SetTransform(&Matrix3x2::identity());
            self.rt.Clear(Some(&d2d(clear)));
        }
    }

    fn begin_over(&self) {
        unsafe {
            self.rt.BeginDraw();
            self.rt.SetTransform(&Matrix3x2::identity());
        }
    }

    fn push_clip(&self, r: RectF) {
        unsafe { self.rt.PushAxisAlignedClip(&d2r(r), D2D1_ANTIALIAS_MODE_ALIASED) };
    }

    fn pop_clip(&self) {
        unsafe { self.rt.PopAxisAlignedClip() };
    }

    fn end(&self) {
        if let Err(e) = unsafe { self.rt.EndDraw(None, None) } {
            log::warn!("Direct2D EndDraw: {e}");
        }
    }

    fn fill_round(&self, r: RectF, radius: f32, c: Color) {
        if let Some(b) = self.brush(c) {
            let rr = D2D1_ROUNDED_RECT { rect: d2r(r), radiusX: radius, radiusY: radius };
            unsafe { self.rt.FillRoundedRectangle(&rr, &b) };
        }
    }

    fn stroke_round(&self, r: RectF, radius: f32, width: f32, c: Color) {
        if let Some(b) = self.brush(c) {
            let rr = D2D1_ROUNDED_RECT { rect: d2r(r), radiusX: radius, radiusY: radius };
            unsafe { self.rt.DrawRoundedRectangle(&rr, &b, width, None) };
        }
    }

    fn text(&self, text: &str, font: &Font, r: RectF, c: Color) {
        let Some(b) = self.brush(c) else { return };
        let t: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.rt.DrawText(&t, &font.fmt, &d2r(r), &b, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL)
        };
    }

    fn icon(&self, icon: Icon, x: f32, y: f32, size: f32, c: Color) {
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
                        let rr = D2D1_ROUNDED_RECT { rect: d2r(rectf(x, y, w, h)), radiusX: rx, radiusY: rx };
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

    fn image_at(&self, bmp: &ID2D1Bitmap, r: RectF, radius: f32, origin: (f32, f32), scale: f32) {
        let (tx, ty, k) = (origin.0, origin.1, scale);
        unsafe {
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
            let rr = D2D1_ROUNDED_RECT { rect: d2r(r), radiusX: radius, radiusY: radius };
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
    use chup_ui::Canvas as _;
    let font = Font::new(font_px, false, Align::Center, false)?;
    let (w, h) = chup_ui::canvas::pill_size(&font, text, pad_x, pad_y);
    let s = Surface::new(w as i32, h as i32)?;
    let c = Canvas::new(&s)?;
    c.begin(Color::CLEAR);
    chup_ui::canvas::paint_pill(&c, &font, text, w, h, radius, bg, fg);
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
