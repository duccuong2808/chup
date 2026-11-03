//! Bố cục chữ OCR trên ảnh (SPEC 15.4, 17.2): các từ theo thứ tự đọc, gom thành hàng, kèm vị trí từng ký tự,
//! để Preview chọn chữ bằng chuột như Live Text (theo ký tự) và để ghép text khi copy.
//! Toạ độ là px của ảnh gốc (đã bỏ hệ số phóng của `prepare`).

use std::ops::Range;

use unicode_normalization::UnicodeNormalization;

use super::{Line, LineWord};
use crate::coords::Rect;

/// Dòng ngắn (≤ 2 ký tự) có confidence dưới ngưỡng này là rác (ô checkbox, viền...).
const JUNK_CONF: f32 = 50.0;

/// Vị trí con trỏ chữ: trước ký tự thứ `ch` của từ `word` (`ch` = số ký tự của từ là cuối từ).
/// Thứ tự so sánh = thứ tự đọc.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub word: usize,
    pub ch: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    /// Đã chuẩn hoá NFC, không có khoảng trắng hai đầu.
    pub text: String,
    pub rect: Rect,
    /// Toạ độ x các ranh giới ký tự: `edges[i]` là mép trái ký tự `i`, phần tử cuối là mép phải từ
    /// (`số ký tự + 1` phần tử, không giảm).
    pub edges: Vec<i32>,
}

impl Word {
    fn len(&self) -> usize {
        self.edges.len() - 1
    }

    /// Chuỗi ký tự `s..e`.
    fn slice(&self, s: usize, e: usize) -> String {
        self.text.chars().skip(s).take(e.saturating_sub(s)).collect()
    }

    /// Ranh giới ký tự gần `x` nhất.
    fn caret(&self, x: i32) -> usize {
        (0..self.edges.len()).min_by_key(|&i| (self.edges[i] - x).abs()).unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LineSpan {
    rect: Rect,
    words: Range<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextLayout {
    /// Mọi từ theo thứ tự đọc: hàng trên xuống, trong hàng trái sang phải.
    words: Vec<Word>,
    /// Các dòng engine trả về (đã sắp), mỗi dòng giữ một đoạn liên tiếp của `words`.
    lines: Vec<LineSpan>,
    /// Hàng = các dòng chồng nhau theo chiều dọc; mỗi hàng là một đoạn liên tiếp của `lines`.
    rows: Vec<Range<usize>>,
}

/// Một hàng đang gom: khoảng dọc (trên, dưới) và các dòng kèm rect đã đổi về ảnh gốc.
type RowAcc = ((i32, i32), Vec<(Rect, Line)>);

fn is_junk(l: &Line) -> bool {
    let t = l.text.trim();
    !t.chars().any(char::is_alphanumeric) || (t.chars().count() <= 2 && l.conf < JUNK_CONF)
}

/// Hai dòng cùng hàng khi phần chồng nhau theo chiều dọc > 50% dòng thấp hơn.
fn same_row(span: (i32, i32), r: &Rect) -> bool {
    let overlap = span.1.min(r.bottom()) - span.0.max(r.y);
    let min_h = (span.1 - span.0).min(r.h as i32).max(1);
    overlap * 2 > min_h
}

fn scale_v(v: i32, factor: f64) -> i32 {
    let f = if factor > 0.0 { factor } else { 1.0 };
    (v as f64 / f).round() as i32
}

/// `[x, y, w, h]` của ảnh đưa vào engine -> `Rect` của ảnh gốc (chia hệ số phóng).
fn unscale(b: [i32; 4], factor: f64) -> Rect {
    let k = |v: i32| scale_v(v, factor);
    Rect::new(k(b[0]), k(b[1]), k(b[2].max(1)).max(1) as u32, k(b[3].max(1)).max(1) as u32)
}

fn clean(s: &str) -> String {
    s.trim().nfc().collect()
}

/// Độ rộng tương đối của ký tự, để ước lượng vị trí ký tự khi engine chỉ có bbox của từ (Windows OCR).
fn char_weight(c: char) -> f64 {
    match c {
        'i' | 'l' | 'j' | 'I' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' | '`' | 'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 0.45,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '{' | '}' | '/' | '\\' | '-' | '"' | '1' => 0.65,
        'm' | 'w' | 'M' | 'W' | '@' | '%' => 1.5,
        c if c.is_uppercase() => 1.2,
        c if c.is_whitespace() => 0.5,
        _ => 1.0,
    }
}

/// Ranh giới ký tự của từ `text` trong `rect`. Có vị trí ký tự từ engine (`chars`, đúng số ký tự) thì dùng:
/// ranh giới giữa hai ký tự là điểm giữa khe; không thì chia bề rộng từ theo `char_weight`.
fn edges(text: &str, rect: Rect, chars: &[[i32; 2]], factor: f64) -> Vec<i32> {
    let n = text.chars().count();
    let (left, right) = (rect.x, rect.right());
    let mut e = Vec::with_capacity(n + 1);
    e.push(left);
    if chars.len() == n && n > 0 {
        for i in 1..n {
            let prev_r = scale_v(chars[i - 1][0] + chars[i - 1][1], factor);
            let next_l = scale_v(chars[i][0], factor);
            e.push(((prev_r + next_l) / 2).clamp(left, right));
        }
    } else {
        let w: Vec<f64> = text.chars().map(char_weight).collect();
        let total: f64 = w.iter().sum::<f64>().max(1e-6);
        let mut acc = 0.0;
        for wi in w.iter().take(n.saturating_sub(1)) {
            acc += wi;
            e.push(left + (acc / total * rect.w as f64).round() as i32);
        }
    }
    e.push(right);
    // Ký tự engine trả lệch thứ tự: giữ ranh giới không giảm.
    for i in 1..e.len() {
        e[i] = e[i].max(e[i - 1]);
    }
    e
}

/// Khoảng cách từ `v` tới đoạn `[lo, hi)` (0 nếu nằm trong).
fn gap(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo - v
    } else if v >= hi {
        v - hi + 1
    } else {
        0
    }
}

impl TextLayout {
    /// Dựng từ các dòng engine trả về. `factor` = cỡ ảnh đưa vào engine / cỡ ảnh gốc (2 khi đã phóng ×2).
    /// Bỏ dòng rác; dòng không có `words` thì cả dòng là một từ.
    pub fn build(lines: Vec<Line>, factor: f64) -> Self {
        let mut lines: Vec<(Rect, Line)> =
            lines.into_iter().filter(|l| !is_junk(l)).map(|l| (unscale(l.bbox, factor), l)).collect();
        lines.sort_by_key(|(r, _)| (r.y, r.x));
        let mut grouped: Vec<RowAcc> = vec![];
        for (r, l) in lines {
            match grouped.last_mut() {
                Some((span, row)) if same_row(*span, &r) => {
                    *span = (span.0.min(r.y), span.1.max(r.bottom()));
                    row.push((r, l));
                }
                _ => grouped.push(((r.y, r.bottom()), vec![(r, l)])),
            }
        }
        let mut out = TextLayout::default();
        for (_, mut row) in grouped {
            row.sort_by_key(|(r, _)| r.x);
            let first_line = out.lines.len();
            for (rect, line) in row {
                let start = out.words.len();
                for w in &line.words {
                    out.push_word(w, factor);
                }
                if out.words.len() == start {
                    let whole = LineWord { text: line.text.clone(), bbox: line.bbox, conf: line.conf, chars: vec![] };
                    out.push_word(&whole, factor);
                }
                if out.words.len() > start {
                    out.lines.push(LineSpan { rect, words: start..out.words.len() });
                }
            }
            if out.lines.len() > first_line {
                out.rows.push(first_line..out.lines.len());
            }
        }
        out
    }

    fn push_word(&mut self, w: &LineWord, factor: f64) {
        let text = clean(&w.text);
        if text.is_empty() {
            return;
        }
        let rect = unscale(w.bbox, factor);
        // `chars` ứng với text gốc của engine; khoảng trắng hai đầu bị bỏ thì số ký tự lệch, `edges` tự ước lượng.
        let edges = edges(&text, rect, &w.chars, factor);
        self.words.push(Word { text, rect, edges });
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn words(&self) -> &[Word] {
        &self.words
    }

    fn row_words(&self, row: &Range<usize>) -> Range<usize> {
        self.lines[row.start].words.start..self.lines[row.end - 1].words.end
    }

    fn row_rect(&self, row: &Range<usize>) -> Rect {
        self.lines[row.clone()].iter().fold(Rect::default(), |acc, l| acc.union(&l.rect))
    }

    fn row_of(&self, word: usize) -> Option<&Range<usize>> {
        self.rows.iter().find(|r| self.row_words(r).contains(&word))
    }

    fn word_end(&self, word: usize) -> Pos {
        Pos { word, ch: self.words[word].len() }
    }

    /// Đầu và cuối toàn bộ chữ (Ctrl+A).
    pub fn all(&self) -> Option<(Pos, Pos)> {
        let last = self.words.len().checked_sub(1)?;
        Some((Pos::default(), self.word_end(last)))
    }

    /// Cả từ chứa `p` (bấm đúp).
    pub fn word_range(&self, p: Pos) -> (Pos, Pos) {
        (Pos { word: p.word, ch: 0 }, self.word_end(p.word))
    }

    /// Cả hàng chứa `p` (bấm 3 lần).
    pub fn row_range(&self, p: Pos) -> (Pos, Pos) {
        match self.row_of(p.word).map(|r| self.row_words(r)) {
            Some(r) => (Pos { word: r.start, ch: 0 }, self.word_end(r.end - 1)),
            None => self.word_range(p),
        }
    }

    /// Chữ giữa hai vị trí (thứ tự bất kỳ): trong hàng nối bằng dấu cách, giữa các hàng xuống dòng.
    pub fn text(&self, a: Pos, b: Pos) -> String {
        let (lo, hi) = (a.min(b), a.max(b));
        let mut rows = vec![];
        for row in &self.rows {
            let r = self.row_words(row);
            let pieces: Vec<String> = (r.start.max(lo.word)..r.end.min(hi.word + 1))
                .filter_map(|i| {
                    let w = &self.words[i];
                    let s = if i == lo.word { lo.ch } else { 0 };
                    let e = if i == hi.word { hi.ch.min(w.len()) } else { w.len() };
                    (s < e).then(|| w.slice(s, e))
                })
                .collect();
            if !pieces.is_empty() {
                rows.push(pieces.join(" "));
            }
        }
        rows.join("\n")
    }

    /// Toàn bộ chữ (như nút [T] copy). Rỗng nếu không có chữ.
    pub fn all_text(&self) -> String {
        self.all().map(|(a, b)| self.text(a, b)).unwrap_or_default()
    }

    /// Vị trí dưới điểm `p`: `p` nằm trong một dòng (nới `pad` px) thì trả ranh giới ký tự gần nhất
    /// của từ gần nhất trong dòng đó.
    pub fn hit(&self, p: (i32, i32), pad: i32) -> Option<Pos> {
        let line = self
            .lines
            .iter()
            .filter(|l| l.rect.expand(pad).contains_point(p.0, p.1))
            .min_by_key(|l| gap(p.1, l.rect.y, l.rect.bottom()) + gap(p.0, l.rect.x, l.rect.right()))?;
        self.nearest_in(line.words.clone(), p.0)
    }

    /// Vị trí gần `p` nhất (dùng khi đang kéo chọn): hàng gần nhất theo chiều dọc, rồi từ và ký tự gần nhất.
    pub fn nearest(&self, p: (i32, i32)) -> Option<Pos> {
        let row = self.rows.iter().min_by_key(|row| {
            let r = self.row_rect(row);
            gap(p.1, r.y, r.bottom())
        })?;
        self.nearest_in(self.row_words(row), p.0)
    }

    fn nearest_in(&self, words: Range<usize>, x: i32) -> Option<Pos> {
        let word = words.min_by_key(|&i| {
            let r = self.words[i].rect;
            gap(x, r.x, r.right())
        })?;
        Some(Pos { word, ch: self.words[word].caret(x) })
    }

    /// Vùng tô cho đoạn `a..b`: mỗi hàng một rect, từ con trỏ đầu tới con trỏ cuối trong hàng đó.
    pub fn highlight(&self, a: Pos, b: Pos) -> Vec<Rect> {
        let (lo, hi) = (a.min(b), a.max(b));
        let mut out = vec![];
        for row in &self.rows {
            let r = self.row_words(row);
            let (s, e) = (r.start.max(lo.word), r.end.min(hi.word + 1));
            if s >= e {
                continue;
            }
            let x0 = if s == lo.word { self.words[s].edges[lo.ch.min(self.words[s].len())] } else { self.words[s].rect.x };
            let last = &self.words[e - 1];
            let x1 = if e - 1 == hi.word { last.edges[hi.ch.min(last.len())] } else { last.rect.right() };
            if x1 <= x0 {
                continue;
            }
            let (y0, y1) = self.words[s..e].iter().fold((i32::MAX, i32::MIN), |(t, b), w| (t.min(w.rect.y), b.max(w.rect.bottom())));
            out.push(Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, x: i32, y: i32, w: i32, h: i32, conf: f32) -> Line {
        Line { text: text.into(), bbox: [x, y, w, h], conf, words: vec![] }
    }

    /// Dòng có words và vị trí ký tự: mỗi ký tự rộng 10 px, các từ cách nhau 10 px.
    fn wline(text: &str, x: i32, y: i32, h: i32) -> Line {
        let mut words = vec![];
        let mut cx = x;
        for t in text.split(' ') {
            let n = t.chars().count() as i32;
            let chars = (0..n).map(|i| [cx + 10 * i, 10]).collect();
            words.push(LineWord { text: t.into(), bbox: [cx, y, 10 * n, h], conf: 90.0, chars });
            cx += 10 * n + 10;
        }
        Line { text: text.into(), bbox: [x, y, cx - 10 - x, h], conf: 90.0, words }
    }

    fn pos(word: usize, ch: usize) -> Pos {
        Pos { word, ch }
    }

    fn sample() -> TextLayout {
        // Hàng 0: "Xin chào" + "bạn" (hai dòng cùng hàng); hàng 1: "dòng hai"; hàng 2: "ba".
        TextLayout::build(
            vec![wline("dòng hai", 0, 40, 20), wline("bạn", 200, 2, 20), wline("Xin chào", 0, 0, 22), wline("ba", 10, 80, 20)],
            1.0,
        )
    }

    #[test]
    fn words_are_in_reading_order_and_rows_join() {
        let t = sample();
        let texts: Vec<_> = t.words().iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Xin", "chào", "bạn", "dòng", "hai", "ba"]);
        assert_eq!(t.all_text(), "Xin chào bạn\ndòng hai\nba");
    }

    #[test]
    fn text_between_carets_takes_parts_of_words() {
        let t = sample();
        assert_eq!(t.text(pos(1, 1), pos(3, 2)), "hào bạn\ndò", "giữa từ tới giữa từ, qua hàng");
        assert_eq!(t.text(pos(3, 2), pos(1, 1)), "hào bạn\ndò", "kéo ngược chiều");
        assert_eq!(t.text(pos(0, 1), pos(0, 3)), "in");
        assert_eq!(t.text(pos(0, 2), pos(0, 2)), "", "chưa chọn gì");
        assert_eq!(t.text(pos(2, 3), pos(3, 0)), "", "cuối hàng tới đầu hàng sau: không có ký tự");
        assert_eq!(t.text(pos(0, 3), pos(2, 3)), "chào bạn");
    }

    #[test]
    fn ranges_for_double_triple_click_and_select_all() {
        let t = sample();
        assert_eq!(t.word_range(pos(1, 2)), (pos(1, 0), pos(1, 4)));
        assert_eq!(t.row_range(pos(1, 2)), (pos(0, 0), pos(2, 3)));
        assert_eq!(t.row_range(pos(4, 0)), (pos(3, 0), pos(4, 3)));
        assert_eq!(t.all(), Some((pos(0, 0), pos(5, 2))));
        assert_eq!(TextLayout::default().all(), None);
    }

    #[test]
    fn lines_without_words_are_one_word_and_junk_is_dropped() {
        let t = TextLayout::build(
            vec![line("  cả dòng ", 0, 0, 100, 20, 90.0), line("| ~", 0, 30, 10, 10, 95.0), line("x1", 0, 60, 10, 10, 20.0)],
            1.0,
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t.all_text(), "cả dòng");
        assert_eq!(t.words()[0].edges.len(), 8, "7 ký tự: ước lượng ranh giới");
        assert!(TextLayout::build(vec![], 1.0).is_empty());
        assert_eq!(TextLayout::build(vec![], 1.0).all_text(), "");
    }

    #[test]
    fn bbox_and_chars_are_mapped_back_to_original_image() {
        let t = TextLayout::build(vec![wline("ab cd", 20, 40, 30)], 2.0);
        assert_eq!(t.words()[0].rect, Rect::new(10, 20, 10, 15));
        assert_eq!(t.words()[0].edges, [10, 15, 20]);
        assert_eq!(t.words()[1].rect, Rect::new(25, 20, 10, 15));
    }

    #[test]
    fn estimated_edges_follow_char_widths() {
        // Không có vị trí ký tự (Windows OCR): "mil" = m rộng, i và l hẹp.
        let l = Line {
            text: "mil".into(),
            bbox: [0, 0, 120, 20],
            conf: 100.0,
            words: vec![LineWord { text: "mil".into(), bbox: [0, 0, 120, 20], conf: 100.0, chars: vec![] }],
        };
        let t = TextLayout::build(vec![l], 1.0);
        let e = &t.words()[0].edges;
        assert_eq!(e.len(), 4);
        assert_eq!((e[0], e[3]), (0, 120));
        assert!(e[1] > 60 && e[2] - e[1] < e[1], "m chiếm nhiều hơn i: {e:?}");
    }

    #[test]
    fn words_are_nfc_and_char_count_matches_after_nfc() {
        let mut l = wline("Viet", 0, 0, 10);
        // NFD: 6 code point, 4 ký tự sau NFC; engine trả 4 ký tự.
        l.words[0].text = "Vie\u{0302}\u{0323}t".into();
        let t = TextLayout::build(vec![l], 1.0);
        assert_eq!(t.words()[0].text, "Vi\u{1EC7}t");
        assert_eq!(t.words()[0].edges, [0, 10, 20, 30, 40], "dùng vị trí ký tự của engine");
        assert_eq!(t.text(pos(0, 2), pos(0, 3)), "\u{1EC7}");
    }

    #[test]
    fn hit_finds_caret_in_line_or_nothing() {
        let t = sample();
        assert_eq!(t.hit((3, 10), 0), Some(pos(0, 0)), "nửa trái chữ X");
        assert_eq!(t.hit((7, 10), 0), Some(pos(0, 1)), "nửa phải chữ X");
        assert_eq!(t.hit((34, 10), 0), Some(pos(0, 3)), "khe giữa hai từ, gần Xin");
        assert_eq!(t.hit((37, 10), 0), Some(pos(1, 0)));
        assert_eq!(t.hit((150, 10), 0), None, "giữa hai dòng cùng hàng: không phải chữ");
        assert_eq!(t.hit((5, 30), 0), None, "khe giữa hai hàng");
        assert_eq!(t.hit((5, 24), 3), Some(pos(0, 0)), "nới pad");
        assert_eq!(t.hit((226, 10), 0), Some(pos(2, 3)));
    }

    #[test]
    fn nearest_picks_closest_row_then_caret() {
        let t = sample();
        assert_eq!(t.nearest((500, 10)), Some(pos(2, 3)), "bên phải hàng 0: cuối hàng");
        assert_eq!(t.nearest((-50, 35)), Some(pos(3, 0)), "gần hàng 1 hơn, bên trái");
        assert_eq!(t.nearest((60, 1000)), Some(pos(5, 2)), "dưới mọi hàng");
        assert_eq!(t.nearest((0, -100)), Some(pos(0, 0)));
        assert_eq!(TextLayout::default().nearest((0, 0)), None);
    }

    #[test]
    fn highlight_spans_carets_one_rect_per_row() {
        let t = sample();
        // "hào bạn" (chào từ ký tự 1: x 50; bạn tới cuối: x 230) và "dò" (x 0..20).
        assert_eq!(t.highlight(pos(1, 1), pos(3, 2)), vec![Rect::new(50, 0, 180, 22), Rect::new(0, 40, 20, 20)]);
        assert_eq!(t.highlight(pos(4, 1), pos(4, 2)), vec![Rect::new(60, 40, 10, 20)]);
        assert!(t.highlight(pos(0, 2), pos(0, 2)).is_empty());
        assert_eq!(t.highlight(pos(2, 3), pos(3, 1)), vec![Rect::new(0, 40, 10, 20)], "hàng trên không có ký tự nào");
    }
}
