//! Tự động chọn vùng (SPEC 16): chuỗi khung dưới con trỏ, đổi cấp, luật click / kéo.
//! Thiết kế: docs/plans/2026-11-03-autoselect-design.md. Module thuần, unit test được.
//!
//! Mọi `Rect` là physical px trên toàn desktop. Khung lấy từ cửa sổ (DWM), phần tử UI Automation, hoặc nhận từ ảnh.

use crate::coords::Rect;

/// Loại khung, để hiện nhãn "W × H · Loại".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    Window,
    Button,
    Edit,
    Text,
    Image,
    Link,
    List,
    ListItem,
    Menu,
    MenuItem,
    Tab,
    ToolBar,
    TitleBar,
    Document,
    Table,
    Group,
    Pane,
    Other,
    /// Nhận từ ảnh (nguồn 3).
    Detected,
}

impl FrameKind {
    /// Từ mã `UIA_*ControlTypeId` (50000...).
    pub fn from_uia(control_type: i32) -> Self {
        use FrameKind::*;
        match control_type {
            50000 | 50002 | 50013 | 50031 => Button,
            50003 | 50004 => Edit,
            50020 => Text,
            50006 => Image,
            50005 => Link,
            50008 | 50023 | 50028 => List,
            50007 | 50024 | 50029 => ListItem,
            50009 | 50010 => Menu,
            50011 => MenuItem,
            50018 | 50019 => Tab,
            50017 | 50021 | 50040 => ToolBar,
            50037 => TitleBar,
            50030 => Document,
            50036 => Table,
            50026 => Group,
            50025 | 50033 => Pane,
            50032 => Window,
            _ => Other,
        }
    }

    /// Từ role của Accessibility macOS (`AXButton`, `AXTextField`...).
    pub fn from_ax(role: &str) -> Self {
        use FrameKind::*;
        match role {
            "AXButton" | "AXPopUpButton" | "AXMenuButton" | "AXCheckBox" | "AXRadioButton" | "AXDisclosureTriangle" => {
                Button
            }
            "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField" => Edit,
            "AXStaticText" | "AXHeading" => Text,
            "AXImage" => Image,
            "AXLink" => Link,
            "AXList" | "AXOutline" | "AXBrowser" => List,
            "AXRow" | "AXCell" | "AXListItem" => ListItem,
            "AXMenu" | "AXMenuBar" => Menu,
            "AXMenuItem" | "AXMenuBarItem" => MenuItem,
            "AXTabGroup" | "AXTab" => Tab,
            "AXToolbar" => ToolBar,
            "AXWebArea" => Document,
            "AXTable" | "AXGrid" => Table,
            "AXGroup" | "AXRadioGroup" | "AXSplitGroup" => Group,
            "AXScrollArea" | "AXSplitter" | "AXLayoutArea" => Pane,
            "AXWindow" | "AXSheet" | "AXDrawer" => Window,
            _ => Other,
        }
    }

    pub fn label(self) -> &'static str {
        use FrameKind::*;
        let t = crate::i18n::t();
        match self {
            Window => t.kind_window,
            Button => t.kind_button,
            Edit => t.kind_edit,
            Text => t.kind_text,
            Image => t.kind_image,
            Link => t.kind_link,
            List => t.kind_list,
            ListItem => t.kind_list_item,
            Menu => t.kind_menu,
            MenuItem => t.kind_menu_item,
            Tab => t.kind_tab,
            ToolBar => t.kind_toolbar,
            TitleBar => t.kind_title_bar,
            Document => t.kind_document,
            Table => t.kind_table,
            Group => t.kind_group,
            Pane | Other => t.kind_pane,
            Detected => t.kind_detected,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub rect: Rect,
    pub kind: FrameKind,
}

impl Frame {
    pub const fn new(rect: Rect, kind: FrameKind) -> Self {
        Self { rect, kind }
    }

    /// Nhãn hiện trên overlay: "W × H · Loại".
    pub fn label(&self) -> String {
        format!("{} × {} · {}", self.rect.w, self.rect.h, self.kind.label())
    }
}

/// Cửa sổ top-level chụp lúc mở overlay. Danh sách theo z-order: trên cùng trước.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WinFrame {
    /// HWND (Windows), số cửa sổ `kCGWindowNumber` (macOS).
    pub id: isize,
    /// Khung DWM, không gồm viền resize vô hình.
    pub rect: Rect,
}

/// Cửa sổ trên cùng chứa điểm.
pub fn window_at(wins: &[WinFrame], p: (i32, i32)) -> Option<&WinFrame> {
    wins.iter().find(|w| w.rect.contains_point(p.0, p.1))
}

/// Khung nhỏ hơn cạnh này thì bỏ (SPEC 16.3).
pub const MIN_SIDE: u32 = 8;
/// Khung chênh khung ngay ngoài nó không quá chừng này mỗi cạnh thì coi là trùng (SPEC 16.3).
pub const SAME_PX: i32 = 2;
/// Nhả chuột cách chỗ nhấn không quá chừng này (mỗi trục) là click (SPEC 16.2).
pub const CLICK_PX: i32 = 4;

fn same(a: &Rect, b: &Rect) -> bool {
    (a.x - b.x).abs() <= SAME_PX
        && (a.y - b.y).abs() <= SAME_PX
        && (a.right() - b.right()).abs() <= SAME_PX
        && (a.bottom() - b.bottom()).abs() <= SAME_PX
}

/// Chuẩn hoá chuỗi khung (từ ngoài vào trong, phần tử đầu là cửa sổ) cho điểm `p`:
/// - cắt theo `clip` (màn hình đang có con trỏ) và theo khung ngoài nó (phần tử cuộn ra ngoài vùng nhìn thấy);
/// - bỏ khung không chứa `p`, nhỏ hơn `MIN_SIDE`, hoặc trùng khung ngay ngoài nó (giữ khung ngoài).
pub fn normalize(raw: &[Frame], p: (i32, i32), clip: Rect) -> Vec<Frame> {
    let mut out: Vec<Frame> = Vec::with_capacity(raw.len());
    for f in raw {
        let bound = out.last().map(|o| o.rect).unwrap_or(clip);
        let Some(r) = f.rect.intersect(&bound) else { continue };
        if !r.contains_point(p.0, p.1) || r.w < MIN_SIDE || r.h < MIN_SIDE {
            continue;
        }
        if out.last().is_some_and(|o| same(&o.rect, &r)) {
            continue;
        }
        out.push(Frame::new(r, f.kind));
    }
    out
}

/// Chọn cấp khung (SPEC 16.2 bước 3).
/// - Khung đã chọn bằng ↑↓ / cuộn (`anchor`) được giữ nguyên khi con trỏ còn trong nó, dù chuỗi ở điểm mới sâu
///   hay nông hơn (rê qua chữ, ảnh bên trong một thẻ).
/// - Ra khỏi khung đó: `up` = số cấp đã lên tính từ khung trong cùng, giữ nguyên khi di chuột trong cùng cửa sổ
///   (rê sang nút bên cạnh vẫn là thanh công cụ); chuỗi ngắn hơn `up` (kết quả UIA chưa về) thì kẹp khi đọc.
/// - Sang cửa sổ khác: về khung trong cùng.
#[derive(Clone, Copy, Debug, Default)]
pub struct Picker {
    window: Option<isize>,
    up: usize,
    anchor: Option<Rect>,
}

impl Picker {
    pub fn pick<'a>(&mut self, window: isize, chain: &'a [Frame]) -> Option<&'a Frame> {
        if self.window != Some(window) {
            self.window = Some(window);
            self.up = 0;
            self.anchor = None;
        }
        chain.get(self.index(chain)?)
    }

    /// Vị trí khung đang chọn trong `chain` (ngoài -> trong).
    fn index(&self, chain: &[Frame]) -> Option<usize> {
        let last = chain.len().checked_sub(1)?;
        let anchored = self.anchor.and_then(|a| chain.iter().position(|f| f.rect == a));
        Some(anchored.unwrap_or(last - self.up.min(last)))
    }

    /// `steps` dương = lên khung cha, âm = xuống khung con, tính từ khung đang chọn trong `chain`. Kẹp trong chuỗi.
    pub fn wheel(&mut self, steps: i32, chain: &[Frame]) {
        let Some(cur) = self.index(chain) else { return };
        let last = chain.len() - 1;
        let next = (cur as i64 - steps as i64).clamp(0, last as i64) as usize;
        self.up = last - next;
        // Khung trong cùng thì đi theo con trỏ như bình thường, không giữ.
        self.anchor = (next != last).then(|| chain[next].rect);
    }

    /// Phiên chọn vùng mới.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Nhấn ở `down`, nhả ở `up`: là click (chọn khung đang tô) hay kéo (chọn vùng tự do).
pub fn is_click(down: (i32, i32), up: (i32, i32)) -> bool {
    (up.0 - down.0).abs() <= CLICK_PX && (up.1 - down.1).abs() <= CLICK_PX
}

#[cfg(test)]
mod tests {
    #[test]
    fn ax_roles_map_to_kinds() {
        assert_eq!(FrameKind::from_ax("AXButton"), FrameKind::Button);
        assert_eq!(FrameKind::from_ax("AXTextField"), FrameKind::Edit);
        assert_eq!(FrameKind::from_ax("AXWebArea"), FrameKind::Document);
        assert_eq!(FrameKind::from_ax("AXMenuItem"), FrameKind::MenuItem);
        assert_eq!(FrameKind::from_ax("AXUnknownThing"), FrameKind::Other);
    }

    use super::*;

    fn f(x: i32, y: i32, w: u32, h: u32, kind: FrameKind) -> Frame {
        Frame::new(Rect::new(x, y, w, h), kind)
    }

    const MON: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn kind_from_uia_and_label() {
        assert_eq!(FrameKind::from_uia(50000), FrameKind::Button);
        assert_eq!(FrameKind::from_uia(50004), FrameKind::Edit);
        assert_eq!(FrameKind::from_uia(50006), FrameKind::Image);
        assert_eq!(FrameKind::from_uia(50030), FrameKind::Document);
        assert_eq!(FrameKind::from_uia(50033), FrameKind::Pane);
        assert_eq!(FrameKind::from_uia(12345), FrameKind::Other);
        assert_eq!(FrameKind::Button.label(), "Nút");
        assert_eq!(FrameKind::Detected.label(), "Vùng ảnh");
        assert_eq!(f(0, 0, 320, 40, FrameKind::ToolBar).label(), "320 × 40 · Thanh công cụ");
    }

    #[test]
    fn window_at_follows_z_order() {
        let wins = [
            WinFrame { id: 1, rect: Rect::new(100, 100, 200, 200) },
            WinFrame { id: 2, rect: Rect::new(0, 0, 1000, 800) },
        ];
        assert_eq!(window_at(&wins, (150, 150)).map(|w| w.id), Some(1));
        assert_eq!(window_at(&wins, (50, 50)).map(|w| w.id), Some(2));
        assert_eq!(window_at(&wins, (1500, 50)), None);
    }

    #[test]
    fn normalize_clips_to_monitor_and_parent() {
        // Cửa sổ vắt sang màn hình bên phải; phần tử con cuộn ra ngoài khung cửa sổ.
        let raw = [
            f(1800, 100, 400, 600, FrameKind::Window),
            f(1810, 50, 300, 900, FrameKind::Document),
        ];
        let out = normalize(&raw, (1850, 300), MON);
        assert_eq!(out[0].rect, Rect::new(1800, 100, 120, 600));
        assert_eq!(out[1].rect, Rect::new(1810, 100, 110, 600));
    }

    #[test]
    fn normalize_drops_small_same_and_not_containing() {
        let raw = [
            f(0, 0, 1000, 800, FrameKind::Window),
            // trùng cửa sổ trong 2 px: bỏ, giữ "Cửa sổ"
            f(1, 2, 998, 797, FrameKind::Pane),
            // không chứa điểm (Chromium có phần tử ẩn ở toạ độ âm)
            f(-300, 100, 288, 900, FrameKind::Group),
            f(100, 100, 400, 300, FrameKind::ToolBar),
            // nhỏ hơn 8 px
            f(120, 120, 7, 40, FrameKind::Button),
            f(110, 110, 50, 40, FrameKind::Button),
        ];
        let out = normalize(&raw, (120, 130), MON);
        let kinds: Vec<FrameKind> = out.iter().map(|x| x.kind).collect();
        assert_eq!(kinds, [FrameKind::Window, FrameKind::ToolBar, FrameKind::Button]);
    }

    #[test]
    fn normalize_keeps_3px_difference() {
        let raw = [f(0, 0, 1000, 800, FrameKind::Window), f(3, 0, 997, 800, FrameKind::Pane)];
        assert_eq!(normalize(&raw, (500, 500), MON).len(), 2);
    }

    #[test]
    fn normalize_point_outside_clip_gives_empty() {
        let raw = [f(1800, 0, 400, 400, FrameKind::Window)];
        assert!(normalize(&raw, (1950, 100), MON).is_empty());
    }

    #[test]
    fn picker_up_down_clamped() {
        let chain = [
            f(0, 0, 1000, 800, FrameKind::Window),
            f(10, 10, 500, 60, FrameKind::ToolBar),
            f(20, 20, 40, 40, FrameKind::Button),
        ];
        let mut p = Picker::default();
        assert_eq!(p.pick(7, &chain).unwrap().kind, FrameKind::Button);
        p.wheel(1, &chain);
        assert_eq!(p.pick(7, &chain).unwrap().kind, FrameKind::ToolBar);
        p.wheel(5, &chain);
        assert_eq!(p.pick(7, &chain).unwrap().kind, FrameKind::Window);
        p.wheel(-1, &chain);
        assert_eq!(p.pick(7, &chain).unwrap().kind, FrameKind::ToolBar);
        p.wheel(-9, &chain);
        assert_eq!(p.pick(7, &chain).unwrap().kind, FrameKind::Button);
    }

    #[test]
    fn picker_keeps_level_in_same_window_resets_on_other() {
        let chain = [
            f(0, 0, 1000, 800, FrameKind::Window),
            f(10, 10, 500, 60, FrameKind::ToolBar),
            f(20, 20, 40, 40, FrameKind::Button),
        ];
        let next = [
            f(0, 0, 1000, 800, FrameKind::Window),
            f(10, 10, 500, 60, FrameKind::ToolBar),
            f(70, 20, 40, 40, FrameKind::Button),
        ];
        let mut p = Picker::default();
        p.pick(7, &chain);
        p.wheel(1, &chain);
        // Rê sang nút bên cạnh trong cùng cửa sổ: vẫn là thanh công cụ.
        assert_eq!(p.pick(7, &next).unwrap().kind, FrameKind::ToolBar);
        // Kết quả UIA chưa về (chỉ có cửa sổ): kẹp, nhưng không quên cấp đã lên.
        assert_eq!(p.pick(7, &next[..1]).unwrap().kind, FrameKind::Window);
        assert_eq!(p.pick(7, &next).unwrap().kind, FrameKind::ToolBar);
        // Sang cửa sổ khác: về khung trong cùng.
        assert_eq!(p.pick(8, &next).unwrap().kind, FrameKind::Button);
    }

    #[test]
    fn picker_empty_chain() {
        let mut p = Picker::default();
        assert!(p.pick(1, &[]).is_none());
        p.wheel(1, &[]);
    }

    #[test]
    fn picker_holds_chosen_frame_while_pointer_inside() {
        // Cửa sổ > cột > thẻ > ảnh. Chọn thẻ (↑ một lần từ ảnh), rê sang chữ trong thẻ (chuỗi nông hơn), rồi sang
        // ảnh khác trong thẻ (chuỗi sâu hơn): vẫn là thẻ. Ra khỏi thẻ: quay về "lên 1 cấp từ trong cùng".
        let win = f(0, 0, 1000, 800, FrameKind::Window);
        let column = f(20, 20, 600, 700, FrameKind::Detected);
        let card = f(40, 40, 400, 300, FrameKind::Detected);
        let photo = f(60, 60, 100, 80, FrameKind::Detected);
        let mut p = Picker::default();
        let on_photo = [win, column, card, photo];
        assert_eq!(p.pick(7, &on_photo), Some(&photo));
        p.wheel(1, &on_photo);
        assert_eq!(p.pick(7, &on_photo), Some(&card));
        let on_text = [win, column, card];
        assert_eq!(p.pick(7, &on_text), Some(&card), "chuỗi nông hơn: không nhảy lên cột");
        let other_photo = f(200, 60, 100, 80, FrameKind::Detected);
        let on_other_photo = [win, column, card, other_photo];
        assert_eq!(p.pick(7, &on_other_photo), Some(&card));
        p.wheel(1, &on_other_photo);
        assert_eq!(p.pick(7, &on_other_photo), Some(&column), "↑ tiếp từ khung đang giữ");
        p.wheel(-2, &on_other_photo);
        assert_eq!(p.pick(7, &on_other_photo), Some(&other_photo), "về trong cùng: đi theo con trỏ");
        assert_eq!(p.pick(7, &on_photo), Some(&photo));
    }

    #[test]
    fn click_threshold_is_4px_per_axis() {
        assert!(is_click((100, 100), (100, 100)));
        assert!(is_click((100, 100), (104, 96)));
        assert!(!is_click((100, 100), (105, 100)));
        assert!(!is_click((100, 100), (100, 95)));
    }
}
