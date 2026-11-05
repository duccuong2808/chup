//! Màu và bảng màu (giống design token của bản web cũ).

/// Màu thẳng (chưa premultiply), 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Color = Color::rgba(255, 255, 255, 1.0);
    pub const CLEAR: Color = Color::rgba(0, 0, 0, 0.0);

    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a,
        }
    }

    pub const fn hex(v: u32) -> Self {
        Self::rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 1.0)
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Pixel premultiplied BGRA (u32 little-endian = 0xAARRGGBB).
    pub fn px(self) -> u32 {
        let a = (self.a.clamp(0.0, 1.0) * 255.0).round() as u32;
        let c = |v: f32| ((v.clamp(0.0, 1.0) * a as f32).round() as u32).min(a);
        (a << 24) | (c(self.r) << 16) | (c(self.g) << 8) | c(self.b)
    }
}

/// Bảng màu sáng / tối theo giao diện của OS.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    pub fn new(dark: bool) -> Self {
        let accent = Color::hex(0x1f6feb);
        let accent_strong = Color::hex(0x1858c4);
        let danger = Color::hex(0xd1242f);
        if dark {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn px_is_premultiplied_bgra() {
        assert_eq!(Color::WHITE.px(), 0xFFFF_FFFF);
        assert_eq!(Color::CLEAR.px(), 0);
        // Đỏ 50%: alpha 128, kênh đỏ nhân alpha.
        assert_eq!(Color::rgba(255, 0, 0, 0.5).px(), 0x8080_0000);
        assert_eq!(Color::hex(0x1f6feb).px(), 0xFF1F_6FEB);
    }

    #[test]
    fn themes_differ_only_in_surfaces() {
        let (l, d) = (Theme::new(false), Theme::new(true));
        assert_eq!(l.accent, d.accent);
        assert_ne!(l.surface, d.surface);
    }
}
