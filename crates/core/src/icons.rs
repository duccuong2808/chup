//! Icon dạng vector, hệ toạ độ 24×24 (giống bộ icon SVG cũ), nét 2, đầu nét tròn.
//! Lớp platform đổi `Shape` sang API vẽ của OS (Direct2D, Core Graphics...).

/// Lệnh path với toạ độ tuyệt đối.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    Move(f32, f32),
    Line(f32, f32),
    /// Cung elip theo SVG: bán kính, góc xoay (độ), cờ cung lớn, cờ chiều kim đồng hồ, điểm cuối.
    Arc { rx: f32, ry: f32, rot: f32, large: bool, sweep: bool, x: f32, y: f32 },
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Path chỉ vẽ nét.
    Stroke(Vec<Cmd>),
    /// Path tô kín (vẫn vẽ nét cùng màu).
    Fill(Vec<Cmd>),
    Circle { cx: f32, cy: f32, r: f32 },
    Rect { x: f32, y: f32, w: f32, h: f32, rx: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Camera,
    Fade,
    Close,
    Copy,
    Folder,
    ChevronRight,
    ChevronLeft,
    Check,
    Pin,
    /// OCR: copy chữ.
    Text,
    /// Đang chạy (vòng 3/4), thay nút đang bận.
    Busy,
    /// Chia sẻ qua link (SPEC 19): mũi tên lên khỏi khay.
    Upload,
}

impl Icon {
    pub fn shapes(self) -> Vec<Shape> {
        use Shape::*;
        let p = |d: &str| parse_path(d).expect("icon path hợp lệ");
        match self {
            Icon::Camera => vec![Stroke(p("M4 8h3l2-3h6l2 3h3v11H4z")), Circle { cx: 12.0, cy: 13.0, r: 3.5 }],
            Icon::Fade => vec![Circle { cx: 12.0, cy: 12.0, r: 8.0 }, Fill(p("M12 4a8 8 0 0 1 0 16z"))],
            Icon::Close => vec![Stroke(p("M6 6l12 12M18 6L6 18"))],
            Icon::Copy => vec![
                Rect { x: 9.0, y: 9.0, w: 11.0, h: 11.0, rx: 2.0 },
                Stroke(p("M5 15V5a1 1 0 0 1 1-1h10")),
            ],
            Icon::Folder => vec![Stroke(p(
                "M3 6a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z",
            ))],
            Icon::ChevronRight => vec![Stroke(p("M9 6l6 6-6 6"))],
            Icon::ChevronLeft => vec![Stroke(p("M15 6l-6 6 6 6"))],
            Icon::Check => vec![Stroke(p("M5 12l5 5 9-10"))],
            Icon::Pin => vec![Stroke(p("M9 3h6M10 3v6l-3 4h10l-3-4V3M12 13v8"))],
            Icon::Text => vec![Stroke(p("M5 7V5h14v2M12 5v14M9 19h6"))],
            Icon::Busy => vec![Stroke(p("M12 4a8 8 0 1 0 8 8"))],
            Icon::Upload => vec![Stroke(p("M12 15V4M7 9l5-5 5 5M5 14v5h14v-5"))],
        }
    }
}

/// Parse tập con của SVG path: M m L l H h V v A a Z z. Lặp tham số ngầm được hỗ trợ
/// (sau M là các L). Trả về toạ độ tuyệt đối.
pub fn parse_path(d: &str) -> Result<Vec<Cmd>, String> {
    let mut toks = Tokens::new(d);
    let mut out = vec![];
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    let mut cmd: Option<char> = None;

    loop {
        let c = match toks.peek_cmd() {
            Some(c) => {
                toks.bump();
                c
            }
            None if toks.at_end() => break,
            None => match cmd {
                // Lặp tham số ngầm: sau M/m là L/l.
                Some('M') => 'L',
                Some('m') => 'l',
                Some(c) if c != 'Z' && c != 'z' => c,
                _ => return Err(format!("thiếu lệnh tại vị trí {}", toks.pos)),
            },
        };
        let rel = c.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match c.to_ascii_uppercase() {
            'M' => {
                let (x, y) = (toks.num()? + ox, toks.num()? + oy);
                out.push(Cmd::Move(x, y));
                (cx, cy, sx, sy) = (x, y, x, y);
            }
            'L' => {
                let (x, y) = (toks.num()? + ox, toks.num()? + oy);
                out.push(Cmd::Line(x, y));
                (cx, cy) = (x, y);
            }
            'H' => {
                let x = toks.num()? + ox;
                out.push(Cmd::Line(x, cy));
                cx = x;
            }
            'V' => {
                let y = toks.num()? + oy;
                out.push(Cmd::Line(cx, y));
                cy = y;
            }
            'A' => {
                let (rx, ry, rot) = (toks.num()?, toks.num()?, toks.num()?);
                let (large, sweep) = (toks.flag()?, toks.flag()?);
                let (x, y) = (toks.num()? + ox, toks.num()? + oy);
                out.push(Cmd::Arc { rx, ry, rot, large, sweep, x, y });
                (cx, cy) = (x, y);
            }
            'Z' => {
                out.push(Cmd::Close);
                (cx, cy) = (sx, sy);
            }
            other => return Err(format!("lệnh path không hỗ trợ: {other}")),
        }
        cmd = Some(c);
    }
    Ok(out)
}

struct Tokens<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> Tokens<'a> {
    fn new(d: &'a str) -> Self {
        Self { s: d.as_bytes(), pos: 0 }
    }

    fn skip_sep(&mut self) {
        while self.pos < self.s.len() && matches!(self.s[self.pos], b' ' | b',' | b'\n' | b'\t') {
            self.pos += 1;
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip_sep();
        self.pos >= self.s.len()
    }

    fn peek_cmd(&mut self) -> Option<char> {
        self.skip_sep();
        let c = *self.s.get(self.pos)? as char;
        c.is_ascii_alphabetic().then_some(c)
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn num(&mut self) -> Result<f32, String> {
        self.skip_sep();
        let start = self.pos;
        let mut seen_dot = false;
        if matches!(self.s.get(self.pos), Some(b'-' | b'+')) {
            self.pos += 1;
        }
        while let Some(&b) = self.s.get(self.pos) {
            match b {
                b'0'..=b'9' => self.pos += 1,
                b'.' if !seen_dot => {
                    seen_dot = true;
                    self.pos += 1
                }
                _ => break,
            }
        }
        std::str::from_utf8(&self.s[start..self.pos])
            .ok()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| format!("số không hợp lệ tại vị trí {start}"))
    }

    fn flag(&mut self) -> Result<bool, String> {
        self.skip_sep();
        match self.s.get(self.pos) {
            Some(b'0') => {
                self.pos += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.pos += 1;
                Ok(true)
            }
            _ => Err(format!("cờ cung không hợp lệ tại vị trí {}", self.pos)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Cmd::*;

    #[test]
    fn relative_and_implicit_commands() {
        assert_eq!(
            parse_path("M9 6l6 6-6 6").unwrap(),
            vec![Move(9.0, 6.0), Line(15.0, 12.0), Line(9.0, 18.0)]
        );
        assert_eq!(
            parse_path("M4 8h3v11H4z").unwrap(),
            vec![Move(4.0, 8.0), Line(7.0, 8.0), Line(7.0, 19.0), Line(4.0, 19.0), Close]
        );
    }

    #[test]
    fn arcs_resolve_to_absolute() {
        let cmds = parse_path("M12 4a8 8 0 0 1 0 16z").unwrap();
        assert_eq!(
            cmds[1],
            Arc { rx: 8.0, ry: 8.0, rot: 0.0, large: false, sweep: true, x: 12.0, y: 20.0 }
        );
        assert_eq!(cmds[2], Close);
    }

    #[test]
    fn every_icon_parses() {
        for i in [
            Icon::Camera,
            Icon::Fade,
            Icon::Close,
            Icon::Copy,
            Icon::Folder,
            Icon::ChevronRight,
            Icon::ChevronLeft,
            Icon::Check,
            Icon::Pin,
            Icon::Text,
            Icon::Busy,
            Icon::Upload,
        ] {
            assert!(!i.shapes().is_empty());
        }
        assert!(parse_path("M1 1 Q2 2 3 3").is_err());
    }
}
