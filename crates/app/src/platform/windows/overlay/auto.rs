//! Tự động chọn vùng trên overlay (SPEC 16): tô khung dưới con trỏ, đổi cấp, chọn bằng click.
//! Dùng chung cho Live và Freeze. Logic thuần ở `chup_core::autoselect`; worker UIA / khung từ ảnh ở
//! `super::super::autoselect`, kết quả về qua App (`auto_result`).

use std::rc::Rc;

use chup_core::{
    autoselect::{normalize, window_at, Frame, FrameKind, Picker, WinFrame},
    coords::Rect,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};

use super::{
    super::{
        app::Action,
        gfx::{Color, PillPainter, Theme},
        win::emit,
    },
    cursor_pos, Overlay, State,
};

/// Viền nét đứt: dày 2 DIP, đoạn 6 DIP.
const EDGE_DIP: f64 = 2.0;
const DASH_DIP: f64 = 6.0;

pub(super) struct Auto {
    wins: Rc<Vec<WinFrame>>,
    picker: Picker,
    /// Điểm (desktop) của lần rê gần nhất và cửa sổ dưới nó.
    at: Option<((i32, i32), WinFrame)>,
    /// Chuỗi khung đã chuẩn hoá (desktop), từ ngoài vào trong, của cửa sổ trong `at`.
    chain: Vec<Frame>,
    /// Khung đang tô (toạ độ overlay).
    shown: Option<Frame>,
    label: Option<PillPainter>,
    label_at: Option<Rect>,
    label_text: String,
    /// Viền và đoạn nét đứt (physical px), màu accent: tính 1 lần mỗi phiên.
    edge: i32,
    dash: i32,
    accent: u32,
}

impl Auto {
    pub(super) fn new(wins: Rc<Vec<WinFrame>>, scale: f64) -> Self {
        Self {
            edge: (EDGE_DIP * scale).round().max(1.0) as i32,
            dash: (DASH_DIP * scale).round().max(2.0) as i32,
            accent: Theme::current().accent.px(),
            wins,
            picker: Picker::default(),
            at: None,
            chain: vec![],
            shown: None,
            label: None,
            label_at: None,
            label_text: String::new(),
        }
    }

    pub(super) fn shown(&self) -> Option<Rect> {
        self.shown.map(|f| f.rect)
    }
}

/// Giữ phần đầu chuỗi còn chứa điểm (chuỗi lồng nhau nên đó là một tiền tố).
fn keep_containing(chain: &mut Vec<Frame>, p: (i32, i32)) {
    let k = chain.iter().take_while(|f| f.rect.contains_point(p.0, p.1)).count();
    chain.truncate(k);
}

impl Overlay {
    /// Bật tự động chọn vùng cho phiên này (gọi sau `show` / `show_frozen`).
    pub fn auto_begin(&self, wins: Rc<Vec<WinFrame>>) {
        self.st.borrow_mut().auto = Some(Auto::new(wins, self.mon.scale));
    }

    /// Khung đang tô, toạ độ overlay.
    pub(super) fn auto_frame(&self) -> Option<Rect> {
        self.st.borrow().auto.as_ref().and_then(Auto::shown)
    }

    /// Chuột ở `p` (toạ độ overlay). `allowed = false`: đang kéo, hoặc (Freeze) chuột trong vùng chọn / trên toolbar.
    /// Giữ `Shift` thì không tô (SPEC 16.2 bước 5).
    pub(super) fn auto_hover(&self, p: (i32, i32), allowed: bool) {
        let (mx, my) = (self.mon.rect.x, self.mon.rect.y);
        let dp = (p.0 + mx, p.1 + my);
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let mut query = None;
        {
            let mut st = self.st.borrow_mut();
            let Some(a) = st.auto.as_mut() else { return };
            let target = match (allowed && !shift).then(|| window_at(&a.wins, dp).copied()).flatten() {
                None => {
                    a.at = None;
                    None
                }
                Some(w) => {
                    if a.at.map(|(_, o)| o.id) == Some(w.id) {
                        keep_containing(&mut a.chain, dp);
                    } else {
                        a.chain.clear();
                    }
                    if a.chain.is_empty() {
                        a.chain = normalize(&[Frame::new(w.rect, FrameKind::Window)], dp, self.mon.rect);
                    }
                    let moved = a.at.map(|(q, _)| q) != Some(dp);
                    a.at = Some((dp, w));
                    if moved {
                        query = Some(w);
                    }
                    a.picker.pick(w.id, &a.chain).copied()
                }
            };
            self.auto_show(&mut st, target);
        }
        if let Some(win) = query {
            emit(Action::AutoHover { p: dp, win, mon: self.mon.rect });
        }
    }

    /// Như `auto_hover` tại vị trí con trỏ hiện tại (vừa mở overlay, nhấn / nhả Shift).
    pub fn auto_refresh(&self) {
        let (cx, cy) = cursor_pos();
        if !self.mon.rect.contains_point(cx, cy) {
            return;
        }
        let p = (cx - self.mon.rect.x, cy - self.mon.rect.y);
        let allowed = self.auto_allowed(p);
        self.auto_hover(p, allowed);
    }

    /// Kết quả của worker cho điểm `p` (desktop) trong cửa sổ `window`. App đã lọc theo thế hệ phiên.
    pub fn auto_result(&self, p: (i32, i32), window: isize, raw: &[Frame]) {
        let mut st = self.st.borrow_mut();
        let Some(a) = st.auto.as_mut() else { return };
        let Some((cur, w)) = a.at else { return };
        if w.id != window {
            return;
        }
        let mut chain = normalize(raw, p, self.mon.rect);
        keep_containing(&mut chain, cur);
        // Kết quả của đúng điểm hiện tại luôn dùng; của điểm cũ thì chỉ dùng nếu nó biết sâu hơn.
        if p != cur && chain.len() <= a.chain.len() {
            return;
        }
        a.chain = chain;
        let target = a.picker.pick(w.id, &a.chain).copied();
        self.auto_show(&mut st, target);
    }

    /// Cuộn chuột / ↑↓: `steps` dương = lên khung cha. `0` = chỉ vẽ lại theo vị trí con trỏ (nhấn / nhả Shift).
    pub fn auto_level(&self, steps: i32) {
        if steps == 0 {
            return self.auto_refresh();
        }
        let mut st = self.st.borrow_mut();
        let Some(a) = st.auto.as_mut() else { return };
        let Some((_, w)) = a.at else { return };
        a.picker.wheel(steps, a.chain.len());
        let target = a.picker.pick(w.id, &a.chain).copied();
        self.auto_show(&mut st, target);
    }

    /// Đổi khung đang tô thành `target` (desktop), vẽ và đẩy phần thay đổi.
    fn auto_show(&self, st: &mut State, target: Option<Frame>) {
        let (mx, my) = (self.mon.rect.x, self.mon.rect.y);
        let local = target.map(|f| Frame::new(f.rect.offset(-mx, -my), f.kind));
        let Some(a) = st.auto.as_mut() else { return };
        let pad = a.edge + 1;
        if a.shown == local {
            return;
        }
        let mut dirty = a.shown.map(|f| f.rect.expand(pad)).unwrap_or_default();
        if let Some(at) = a.label_at.take() {
            dirty = dirty.union(&at);
        }
        a.shown = local;
        if let Some(f) = local {
            dirty = dirty.union(&f.rect.expand(pad));
            a.label_at = self.place_auto_label(a, f);
            if let Some(at) = a.label_at {
                dirty = dirty.union(&at);
            }
        }
        if dirty.is_empty() {
            return;
        }
        super::paint(st, dirty);
        if let Some(s) = st.surface.as_ref() {
            s.present(self.hwnd, (mx, my), 255, Some(dirty));
        }
    }

    /// Nhãn "W × H · Loại": dưới khung, sát đáy thì lên trên, không còn chỗ thì nằm trong góc trên-trái khung.
    fn place_auto_label(&self, a: &mut Auto, f: Frame) -> Option<Rect> {
        let (w, h) = (self.mon.rect.w as i32, self.mon.rect.h as i32);
        let k = self.mon.scale as f32;
        if a.label.is_none() {
            a.label = PillPainter::new(12.0 * k, 6.0 * k, 2.0 * k, 4.0 * k, (self.s(320.0), self.s(30.0)));
        }
        a.label_text = f.label();
        let p = a.label.as_mut()?;
        let (lw, lh) = p.paint(&a.label_text, Color::rgba(0, 0, 0, 0.75), Color::rgba(255, 255, 255, 1.0));
        let gap = self.s(6.0);
        let r = f.rect;
        let y = if r.bottom() + gap + lh <= h {
            r.bottom() + gap
        } else if r.y - gap - lh >= 0 {
            r.y - gap - lh
        } else {
            r.y + gap
        };
        let x = (r.x + if y == r.y + gap { gap } else { 0 }).clamp(0, (w - lw).max(0));
        Some(Rect::new(x, y, lw as u32, lh as u32))
    }

    /// Chế độ hiện tại có cho tô khung ở `p` (toạ độ overlay) không.
    fn auto_allowed(&self, p: (i32, i32)) -> bool {
        if self.frozen_mode.get() {
            return self.frozen_auto_allowed(p);
        }
        let st = self.st.borrow();
        !st.dragging
    }
}

/// Khung đang tô (toạ độ overlay), để mỗi chế độ tự làm sáng phần bên trong **trước** khi vẽ vùng chọn, handle.
pub(super) fn highlight(st: &State) -> Option<Rect> {
    st.auto.as_ref().and_then(Auto::shown)
}

/// Vẽ viền nét đứt và nhãn của khung đang tô, trên cùng.
pub(super) fn paint_auto_edges(st: &mut State) {
    let State { surface, auto, .. } = st;
    let (Some(s), Some(a)) = (surface.as_mut(), auto.as_mut()) else { return };
    let Some(f) = a.shown else { return };
    s.outline_dashed_outside(f.rect, a.edge, a.dash, a.accent, 0xFFFF_FFFF);
    if let (Some(p), Some(at)) = (a.label.as_mut(), a.label_at) {
        s.blend_part(&mut p.surface, (at.w as i32, at.h as i32), at.x, at.y);
    }
}
