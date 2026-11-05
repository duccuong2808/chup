//! Cửa sổ Cài đặt (SPEC 5.6) bằng control gốc của AppKit, cùng các trang và cùng `ConfigPatch` như bản Windows.
//! Control gửi action về `SettingsTarget` (theo `tag`); thay đổi đi qua App (`Action::ConfigPatch`), App gọi
//! `refresh` để hiện lại giá trị đã lưu. Ô ghi phím tắt dùng local event monitor.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    ptr::NonNull,
    rc::{Rc, Weak},
};

use block2::RcBlock;
use chup_core::{
    config::{AfterCapture, Config, Edge, OcrLanguage, PreviewPosition, StackMonitor},
    hotkey_name::{record, vk_from_mac, Mods as HkMods, Recorded},
    i18n::{fill, t, Strings, UiLanguage},
};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::NSAppearanceCustomization;
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSControlStateValueOff, NSControlStateValueOn, NSEvent,
    NSEventMask, NSEventModifierFlags, NSFont, NSLayoutAttribute, NSOpenPanel, NSPopUpButton, NSStackView,
    NSTabView, NSTabViewItem, NSTextField, NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSEdgeInsets, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use serde_json::json;

use super::{app::Action, win::emit};

const PROJECT_URL: &str = "https://github.com/duccuong2808/chup";
const UI_LANGS: [(UiLanguage, Option<&str>); 3] =
    [(UiLanguage::System, None), (UiLanguage::Vi, Some("Tiếng Việt")), (UiLanguage::En, Some("English"))];
const OPACITIES: [u8; 9] = [20, 30, 40, 50, 60, 70, 80, 90, 100];
const SIZES: [u8; 9] = [20, 30, 40, 50, 60, 70, 80, 90, 100];

pub struct SettingsView {
    pub config: Config,
    pub hotkey_errors: BTreeMap<String, String>,
    pub preview: String,
}

// Tag của control.
const ID_PICK: isize = 1;
const ID_OPEN: isize = 2;
const ID_PATTERN: isize = 3;
const ID_AUTOSTART: isize = 4;
const ID_UI_LANG: isize = 5;
const ID_HK: isize = 10; // 10..13: 4 ô phím tắt
const ID_HK_CLEAR: isize = 20; // 20..23
const ID_FREEZE_ON: isize = 30;
const ID_COPY: isize = 40;
const ID_AFTER: isize = 41;
const ID_HIDEUI: isize = 42;
const ID_OCR_LANG: isize = 50;
const ID_OCR_AUTO: isize = 51;
const ID_EDGE: isize = 60;
const ID_MON: isize = 61;
const ID_PV_TOP: isize = 62;
const ID_PV_OP: isize = 63;
const ID_PV_POS: isize = 64;
const ID_PV_SIZE: isize = 65;
const ID_WEB: isize = 70;
const ID_LOG: isize = 71;

/// Tên phím tắt trong config và chữ của nó.
const HOTKEYS: [(&str, fn(&Strings) -> &'static str); 4] = [
    ("selectRegion", |t| t.st_hk_select),
    ("capture", |t| t.st_hk_capture),
    ("freeze", |t| t.st_hk_freeze),
    ("ocr", |t| t.st_hk_ocr),
];

pub struct TargetIvars {
    owner: RefCell<Weak<Settings>>,
}

define_class!(
    // SAFETY: NSObject không có yêu cầu riêng khi kế thừa; không cài Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ChupSettingsTarget"]
    #[ivars = TargetIvars]
    pub struct SettingsTarget;

    impl SettingsTarget {
        #[unsafe(method(onControl:))]
        fn on_control(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let owner = self.ivars().owner.borrow().upgrade();
            if let Some(s) = owner {
                s.on_control(tag, sender);
            }
        }
    }

    unsafe impl NSObjectProtocol for SettingsTarget {}

    unsafe impl NSWindowDelegate for SettingsTarget {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) {
            let owner = self.ivars().owner.borrow().upgrade();
            if let Some(s) = owner {
                s.stop_recording();
            }
            emit(Action::SettingsClosed);
        }
    }
);

#[derive(Default)]
struct Controls {
    dir: Option<Retained<NSTextField>>,
    pattern: Option<Retained<NSTextField>>,
    pattern_hint: Option<Retained<NSTextField>>,
    autostart: Option<Retained<NSButton>>,
    ui_lang: Option<Retained<NSPopUpButton>>,
    hk: Vec<Retained<NSButton>>,
    hk_err: Vec<Retained<NSTextField>>,
    freeze_on: Option<Retained<NSButton>>,
    copy: Option<Retained<NSButton>>,
    after: Option<Retained<NSPopUpButton>>,
    hideui: Option<Retained<NSButton>>,
    ocr_lang: Option<Retained<NSPopUpButton>>,
    ocr_auto: Option<Retained<NSButton>>,
    edge: Option<Retained<NSPopUpButton>>,
    mon: Option<Retained<NSPopUpButton>>,
    pv_top: Option<Retained<NSButton>>,
    pv_op: Option<Retained<NSPopUpButton>>,
    pv_pos: Option<Retained<NSPopUpButton>>,
    pv_size: Option<Retained<NSPopUpButton>>,
    status: Option<Retained<NSTextField>>,
}

pub struct Settings {
    mtm: MainThreadMarker,
    window: Retained<NSWindow>,
    target: Retained<SettingsTarget>,
    c: RefCell<Controls>,
    view: RefCell<Option<SettingsView>>,
    /// Ô phím tắt đang ghi (0..3) và event monitor của nó.
    recording: Cell<Option<usize>>,
    monitor: RefCell<Option<Retained<AnyObject>>>,
    tabs: RefCell<Option<Retained<NSTabView>>>,
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn set_on(b: &Option<Retained<NSButton>>, on: bool) {
    if let Some(b) = b {
        b.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
    }
}

fn is_on(sender: &AnyObject) -> bool {
    let state: isize = unsafe { msg_send![sender, state] };
    state == NSControlStateValueOn
}

fn select(p: &Option<Retained<NSPopUpButton>>, i: usize) {
    if let Some(p) = p {
        p.selectItemAtIndex(i as isize);
    }
}

fn selected(sender: &AnyObject) -> usize {
    let i: isize = unsafe { msg_send![sender, indexOfSelectedItem] };
    i.max(0) as usize
}

/// Phím tắt kiểu macOS (`⌃⌘A`), chuỗi rỗng = chưa gán.
fn show_key(accel: &str) -> String {
    if accel.trim().is_empty() {
        return t().unused_hotkey.to_string();
    }
    let mut mods = String::new();
    let mut key = "";
    for part in accel.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.push('⌃'),
            "alt" | "option" => mods.push('⌥'),
            "shift" => mods.push('⇧'),
            "super" | "cmd" | "command" => mods.push('⌘'),
            _ => key = part,
        }
    }
    format!("{mods}{key}")
}

impl Settings {
    pub fn create(mtm: MainThreadMarker) -> Rc<Settings> {
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable;
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(620.0, 520.0));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        let target = SettingsTarget::alloc(mtm).set_ivars(TargetIvars { owner: RefCell::new(Weak::new()) });
        let target: Retained<SettingsTarget> = unsafe { msg_send![super(target), init] };
        window.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        let s = Rc::new(Settings {
            mtm,
            window,
            target,
            c: RefCell::new(Controls::default()),
            view: RefCell::new(None),
            recording: Cell::new(None),
            monitor: RefCell::new(None),
            tabs: RefCell::new(None),
        });
        *s.target.ivars().owner.borrow_mut() = Rc::downgrade(&s);
        s.build();
        s.window.center();
        s
    }

    pub fn show(&self) {
        NSApplication::sharedApplication(self.mtm).activate();
        self.window.makeKeyAndOrderFront(None);
    }

    /// Ngôn ngữ đổi: dựng lại toàn bộ nội dung.
    pub fn retext(&self) {
        self.build();
        if let Some(v) = self.view.borrow_mut().take() {
            self.refresh(v);
        }
    }

    pub fn set_status(&self, text: &str, error: bool) {
        if let Some(s) = &self.c.borrow().status {
            s.setStringValue(&ns(text));
            let color = if error { objc2_app_kit::NSColor::systemRedColor() } else { objc2_app_kit::NSColor::secondaryLabelColor() };
            s.setTextColor(Some(&color));
        }
    }

    // ---------- dựng giao diện ----------

    fn target(&self) -> &AnyObject {
        self.target.as_ref()
    }

    fn label(&self, text: &str) -> Retained<NSTextField> {
        NSTextField::labelWithString(&ns(text), self.mtm)
    }

    fn hint(&self, text: &str) -> Retained<NSTextField> {
        let l = NSTextField::wrappingLabelWithString(&ns(text), self.mtm);
        l.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
        l.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
        l.setPreferredMaxLayoutWidth(520.0);
        l
    }

    fn section(&self, text: &str) -> Retained<NSTextField> {
        let l = self.label(text);
        l.setFont(Some(&NSFont::boldSystemFontOfSize(NSFont::systemFontSize())));
        l
    }

    fn check(&self, text: &str, tag: isize) -> Retained<NSButton> {
        let b = unsafe { NSButton::checkboxWithTitle_target_action(&ns(text), Some(self.target()), Some(sel!(onControl:)), self.mtm) };
        b.setTag(tag);
        b
    }

    fn button(&self, text: &str, tag: isize) -> Retained<NSButton> {
        let b = unsafe { NSButton::buttonWithTitle_target_action(&ns(text), Some(self.target()), Some(sel!(onControl:)), self.mtm) };
        b.setTag(tag);
        b
    }

    fn popup(&self, items: &[String], tag: isize) -> Retained<NSPopUpButton> {
        let p = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(self.mtm), NSRect::ZERO, false);
        let titles: Vec<Retained<NSString>> = items.iter().map(|s| ns(s)).collect();
        p.addItemsWithTitles(&NSArray::from_retained_slice(&titles));
        p.setTag(tag);
        unsafe {
            p.setTarget(Some(self.target()));
            p.setAction(Some(sel!(onControl:)));
        }
        p
    }

    fn stack(&self, views: &[&NSView], vertical: bool) -> Retained<NSStackView> {
        let s = NSStackView::new(self.mtm);
        for v in views {
            s.addArrangedSubview(v);
        }
        if vertical {
            s.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
            s.setAlignment(NSLayoutAttribute::Leading);
            s.setSpacing(8.0);
        } else {
            s.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
            s.setAlignment(NSLayoutAttribute::CenterY);
            s.setSpacing(8.0);
        }
        s
    }

    /// Hàng "nhãn: control".
    fn row(&self, label: &str, control: &NSView) -> Retained<NSStackView> {
        let l = self.label(label);
        self.stack(&[&l, control], false)
    }

    fn page(&self, tabs: &NSTabView, title: &str, views: &[&NSView]) {
        let content = self.stack(views, true);
        content.setEdgeInsets(NSEdgeInsets { top: 16.0, left: 20.0, bottom: 16.0, right: 20.0 });
        let item = NSTabViewItem::new();
        item.setLabel(&ns(title));
        // Đặt nội dung sát mép trên của tab.
        let holder = NSView::new(self.mtm);
        holder.addSubview(&content);
        content.setTranslatesAutoresizingMaskIntoConstraints(false);
        unsafe {
            let top = content.topAnchor().constraintEqualToAnchor(&holder.topAnchor());
            let left = content.leadingAnchor().constraintEqualToAnchor(&holder.leadingAnchor());
            let right = content.trailingAnchor().constraintLessThanOrEqualToAnchor(&holder.trailingAnchor());
            for c in [top, left, right] {
                c.setActive(true);
            }
        }
        item.setView(Some(&holder));
        tabs.addTabViewItem(&item);
    }

    fn build(&self) {
        let t = t();
        let mut c = Controls::default();
        self.window.setTitle(&ns(t.app_settings_title));
        let tabs = NSTabView::new(self.mtm);

        // Chung.
        let dir = self.label("");
        dir.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
        dir.setPreferredMaxLayoutWidth(520.0);
        let pick = self.button(t.st_pick_folder, ID_PICK);
        let open = self.button(t.st_open_folder, ID_OPEN);
        let pattern = NSTextField::textFieldWithString(&ns(""), self.mtm);
        pattern.setTag(ID_PATTERN);
        unsafe {
            pattern.setTarget(Some(self.target()));
            pattern.setAction(Some(sel!(onControl:)));
        }
        pattern.setFrameSize(NSSize::new(300.0, 22.0));
        let pattern_hint = self.hint("");
        let autostart = self.check(t.mac_st_autostart, ID_AUTOSTART);
        let langs: Vec<String> = UI_LANGS.iter().map(|(_, n)| n.unwrap_or(t.mac_st_ui_language_system).to_string()).collect();
        let ui_lang = self.popup(&langs, ID_UI_LANG);
        let sec_save = self.section(t.st_sec_save);
        let buttons = self.stack(&[&pick, &open], false);
        let lbl_dir = self.label(t.st_save_dir);
        let row_pattern = self.row(t.st_file_name, &pattern);
        let sec_sys = self.section(t.st_sec_system);
        let row_lang = self.row(t.st_ui_language, &ui_lang);
        self.page(
            &tabs,
            t.page_general,
            &[&sec_save, &lbl_dir, &dir, &buttons, &row_pattern, &pattern_hint, &sec_sys, &autostart, &row_lang],
        );
        c.dir = Some(dir);
        c.pattern = Some(pattern);
        c.pattern_hint = Some(pattern_hint);
        c.autostart = Some(autostart);
        c.ui_lang = Some(ui_lang);

        // Phím tắt.
        let hint = self.hint(t.mac_st_hotkeys_hint);
        let mut rows: Vec<Retained<NSStackView>> = vec![];
        for (i, (_, text)) in HOTKEYS.iter().enumerate() {
            let b = self.button("", ID_HK + i as isize);
            b.setFrameSize(NSSize::new(160.0, 24.0));
            let clear = self.button(t.mac_st_clear, ID_HK_CLEAR + i as isize);
            let err = self.hint("");
            err.setTextColor(Some(&objc2_app_kit::NSColor::systemRedColor()));
            let l = self.label(text(t));
            rows.push(self.stack(&[&l, &b, &clear, &err], false));
            c.hk.push(b);
            c.hk_err.push(err);
        }
        let freeze_on = self.check(t.st_hk_freeze_on, ID_FREEZE_ON);
        let mut views: Vec<&NSView> = vec![&hint];
        for r in &rows {
            views.push(r);
        }
        views.push(&freeze_on);
        self.page(&tabs, t.page_hotkeys, &views);
        c.freeze_on = Some(freeze_on);

        // Chụp.
        let sec_after = self.section(t.st_sec_after);
        let copy = self.check(t.st_copy, ID_COPY);
        let after = self.popup(&[t.st_after_close.into(), t.st_after_keep.into()], ID_AFTER);
        let row_after = self.row(t.st_after_normal, &after);
        let after_hint = self.hint(t.st_after_hint);
        let sec_during = self.section(t.st_sec_during);
        let hideui = self.check(t.st_hide_ui, ID_HIDEUI);
        let hide_note = self.hint(t.mac_st_hide_ui_note);
        let sec_auto = self.section(t.st_sec_auto);
        let auto_note = self.hint(t.mac_st_auto_later);
        self.page(
            &tabs,
            t.page_capture,
            &[&sec_after, &copy, &row_after, &after_hint, &sec_during, &hideui, &hide_note, &sec_auto, &auto_note],
        );
        c.copy = Some(copy);
        c.after = Some(after);
        c.hideui = Some(hideui);

        // OCR.
        let sec_ocr = self.section(t.st_sec_ocr);
        let ocr_lang = self.popup(&[t.mac_st_ocr_en.into(), t.st_ocr_vi.into()], ID_OCR_LANG);
        let row_ocr = self.row(t.st_ocr_language, &ocr_lang);
        let ocr_note = self.hint(t.mac_st_ocr_note);
        let ocr_auto = self.check(t.st_ocr_auto, ID_OCR_AUTO);
        let ocr_auto_hint = self.hint(t.st_ocr_auto_hint);
        self.page(&tabs, t.page_ocr, &[&sec_ocr, &row_ocr, &ocr_note, &ocr_auto, &ocr_auto_hint]);
        c.ocr_lang = Some(ocr_lang);
        c.ocr_auto = Some(ocr_auto);

        // Stack & Preview.
        let sec_stack = self.section(t.st_sec_stack);
        let edge = self.popup(&[t.st_edge_right.into(), t.st_edge_left.into()], ID_EDGE);
        let row_edge = self.row(t.st_edge, &edge);
        let mon = self.popup(&[t.st_monitor_primary.into(), t.st_monitor_cursor.into()], ID_MON);
        let row_mon = self.row(t.st_monitor, &mon);
        let sec_pv = self.section(t.st_sec_preview);
        let pv_top = self.check(t.st_pv_top, ID_PV_TOP);
        let ops: Vec<String> = OPACITIES.iter().map(|o| format!("{o}%")).collect();
        let pv_op = self.popup(&ops, ID_PV_OP);
        let row_op = self.row(t.st_pv_opacity, &pv_op);
        let op_hint = self.hint(t.st_pv_opacity_hint);
        let pv_pos =
            self.popup(&[t.st_pv_pos_last.into(), t.st_pv_pos_stack.into(), t.st_pv_pos_center.into()], ID_PV_POS);
        let row_pos = self.row(t.st_pv_position, &pv_pos);
        let sizes: Vec<String> = SIZES.iter().map(|n| fill(t.st_pv_size_max, &[n])).collect();
        let pv_size = self.popup(&sizes, ID_PV_SIZE);
        let row_size = self.row(t.st_pv_size, &pv_size);
        let size_hint = self.hint(t.st_pv_size_hint);
        self.page(
            &tabs,
            t.page_stack_preview,
            &[&sec_stack, &row_edge, &row_mon, &sec_pv, &pv_top, &row_op, &op_hint, &row_pos, &row_size, &size_hint],
        );
        c.edge = Some(edge);
        c.mon = Some(mon);
        c.pv_top = Some(pv_top);
        c.pv_op = Some(pv_op);
        c.pv_pos = Some(pv_pos);
        c.pv_size = Some(pv_size);

        // Giới thiệu.
        let title = self.section("Chup");
        let desc = self.hint(t.mac_st_about_desc);
        let mut ver = fill(t.st_about_version, &[&env!("CARGO_PKG_VERSION")]);
        if !env!("CHUP_COMMIT").is_empty() {
            ver += &format!(" · {}", env!("CHUP_COMMIT"));
        }
        ver += &format!(" · {}", fill(t.st_about_built, &[&env!("CHUP_BUILD_TIME")]));
        let version = self.hint(&ver);
        let web = self.button(t.st_about_web, ID_WEB);
        let log = self.button(t.st_about_log, ID_LOG);
        let links = self.stack(&[&web, &log], false);
        self.page(&tabs, t.page_about, &[&title, &desc, &version, &links]);

        let status = self.hint("");
        c.status = Some(status.clone());
        let root = self.stack(&[&tabs, &status], true);
        root.setEdgeInsets(NSEdgeInsets { top: 12.0, left: 12.0, bottom: 12.0, right: 12.0 });
        tabs.setTranslatesAutoresizingMaskIntoConstraints(false);
        unsafe {
            tabs.widthAnchor().constraintEqualToConstant(596.0).setActive(true);
            tabs.heightAnchor().constraintEqualToConstant(460.0).setActive(true);
        }
        self.window.setContentView(Some(&root));
        *self.c.borrow_mut() = c;
        *self.tabs.borrow_mut() = Some(tabs);
    }

    // ---------- hiện giá trị ----------

    pub fn refresh(&self, v: SettingsView) {
        let t = t();
        {
            let c = self.c.borrow();
            let cfg = &v.config;
            if let Some(d) = &c.dir {
                d.setStringValue(&ns(&cfg.save_dir.display().to_string()));
            }
            if let Some(p) = &c.pattern {
                if self.window.firstResponder().is_none_or(|r| !std::ptr::eq(Retained::as_ptr(&r).cast::<u8>(), Retained::as_ptr(p).cast())) {
                    p.setStringValue(&ns(&cfg.file_name_pattern));
                }
            }
            if let Some(h) = &c.pattern_hint {
                h.setStringValue(&ns(&fill(t.st_file_name_hint, &[&v.preview])));
            }
            set_on(&c.autostart, cfg.launch_at_startup);
            select(&c.ui_lang, UI_LANGS.iter().position(|(l, _)| *l == cfg.ui_language).unwrap_or(0));
            let hk = &cfg.hotkeys;
            let keys = [&hk.select_region, &hk.capture, &hk.freeze, &hk.ocr];
            for (i, ((name, _), key)) in HOTKEYS.iter().zip(keys).enumerate() {
                if self.recording.get() != Some(i) {
                    if let Some(b) = c.hk.get(i) {
                        b.setTitle(&ns(&show_key(key)));
                    }
                }
                if let Some(e) = c.hk_err.get(i) {
                    e.setStringValue(&ns(v.hotkey_errors.get(*name).map(String::as_str).unwrap_or("")));
                }
            }
            set_on(&c.freeze_on, hk.freeze_enabled);
            set_on(&c.copy, cfg.copy_to_clipboard);
            select(&c.after, (cfg.after_capture_normal == AfterCapture::Keep) as usize);
            set_on(&c.hideui, cfg.hide_ui_during_capture);
            select(&c.ocr_lang, (cfg.ocr.language == OcrLanguage::Vi) as usize);
            set_on(&c.ocr_auto, cfg.ocr.auto_preview);
            select(&c.edge, (cfg.stack.edge == Edge::Left) as usize);
            select(&c.mon, (cfg.stack.monitor == StackMonitor::Cursor) as usize);
            set_on(&c.pv_top, cfg.preview.always_on_top);
            select(&c.pv_op, OPACITIES.iter().position(|o| *o >= cfg.preview.opacity).unwrap_or(OPACITIES.len() - 1));
            let pos = match cfg.preview.position {
                PreviewPosition::Remember => 0,
                PreviewPosition::Stack => 1,
                PreviewPosition::Center => 2,
            };
            select(&c.pv_pos, pos);
            select(&c.pv_size, SIZES.iter().position(|s| *s >= cfg.preview.size).unwrap_or(SIZES.len() - 1));
        }
        *self.view.borrow_mut() = Some(v);
    }

    /// Tự kiểm tra: xuất ảnh từng tab ra `dir/settings-<i>.png`.
    pub fn snapshot_tabs(&self, dir: &std::path::Path) {
        let Some(tabs) = self.tabs.borrow().clone() else { return };
        let Some(view) = self.window.contentView() else { return };
        // Giao diện sáng, nền đục: chữ trắng của giao diện tối không nhìn thấy trên nền PNG trong suốt.
        let aqua = objc2_app_kit::NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua });
        self.window.setAppearance(aqua.as_deref());
        view.setWantsLayer(true);
        if let Some(l) = view.layer() {
            l.setBackgroundColor(Some(&objc2_app_kit::NSColor::windowBackgroundColor().CGColor()));
        }
        for i in 0..tabs.numberOfTabViewItems() {
            tabs.selectTabViewItemAtIndex(i);
            view.layoutSubtreeIfNeeded();
            view.displayIfNeeded();
            let bounds = view.bounds();
            let Some(rep) = view.bitmapImageRepForCachingDisplayInRect(bounds) else { continue };
            view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
            let props = objc2_foundation::NSDictionary::new();
            let data = unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &props) };
            if let Some(d) = data {
                let p = dir.join(format!("settings-{i}.png"));
                let _ = std::fs::write(&p, d.to_vec());
                log::info!("selftest: {}", p.display());
            }
        }
        tabs.selectTabViewItemAtIndex(0);
        self.window.setAppearance(None);
    }

    pub fn close(&self) {
        self.window.close();
    }

    // ---------- thao tác ----------

    fn patch(&self, v: serde_json::Value) {
        emit(Action::ConfigPatch(v));
    }

    fn on_control(&self, tag: isize, sender: &AnyObject) {
        match tag {
            ID_PICK => self.pick_folder(),
            ID_OPEN => emit(Action::OpenSaveDir),
            ID_PATTERN => {
                let s: Retained<NSString> = unsafe { msg_send![sender, stringValue] };
                self.patch(json!({ "fileNamePattern": s.to_string() }));
            }
            ID_AUTOSTART => self.patch(json!({ "launchAtStartup": is_on(sender) })),
            ID_UI_LANG => {
                let lang = UI_LANGS[selected(sender).min(UI_LANGS.len() - 1)].0;
                self.patch(json!({ "uiLanguage": lang }));
            }
            t if (ID_HK..ID_HK + 4).contains(&t) => self.start_recording((t - ID_HK) as usize),
            t if (ID_HK_CLEAR..ID_HK_CLEAR + 4).contains(&t) => {
                self.stop_recording();
                let name = HOTKEYS[(t - ID_HK_CLEAR) as usize].0;
                self.patch(json!({ "hotkeys": { name: "" } }));
            }
            ID_FREEZE_ON => self.patch(json!({ "hotkeys": { "freezeEnabled": is_on(sender) } })),
            ID_COPY => self.patch(json!({ "copyToClipboard": is_on(sender) })),
            ID_AFTER => self.patch(json!({ "afterCaptureNormal": if selected(sender) == 1 { "keep" } else { "close" } })),
            ID_HIDEUI => self.patch(json!({ "hideUiDuringCapture": is_on(sender) })),
            ID_OCR_LANG => self.patch(json!({ "ocr": { "language": if selected(sender) == 1 { "vi" } else { "en" } } })),
            ID_OCR_AUTO => self.patch(json!({ "ocr": { "autoPreview": is_on(sender) } })),
            ID_EDGE => self.patch(json!({ "stack": { "edge": if selected(sender) == 1 { "left" } else { "right" } } })),
            ID_MON => {
                self.patch(json!({ "stack": { "monitor": if selected(sender) == 1 { "cursor" } else { "primary" } } }))
            }
            ID_PV_TOP => self.patch(json!({ "preview": { "alwaysOnTop": is_on(sender) } })),
            ID_PV_OP => self.patch(json!({ "preview": { "opacity": OPACITIES[selected(sender).min(OPACITIES.len() - 1)] } })),
            ID_PV_POS => {
                let v = ["remember", "stack", "center"][selected(sender).min(2)];
                self.patch(json!({ "preview": { "position": v } }));
            }
            ID_PV_SIZE => self.patch(json!({ "preview": { "size": SIZES[selected(sender).min(SIZES.len() - 1)] } })),
            ID_WEB => emit(Action::Open(PROJECT_URL.into())),
            ID_LOG => emit(Action::Open(super::util::dirs().log_dir.join("chup.log"))),
            _ => {}
        }
    }

    fn pick_folder(&self) {
        let panel = NSOpenPanel::openPanel(self.mtm);
        panel.setCanChooseDirectories(true);
        panel.setCanChooseFiles(false);
        panel.setAllowsMultipleSelection(false);
        panel.setCanCreateDirectories(true);
        panel.setTitle(Some(&ns(t().st_pick_folder_title)));
        if panel.runModal() != objc2_app_kit::NSModalResponseOK {
            return;
        }
        let path = panel.URLs().firstObject().and_then(|u| u.path());
        if let Some(p) = path {
            self.patch(json!({ "saveDir": p.to_string() }));
        }
    }

    /// Bắt đầu ghi phím cho ô `i`: tắt tạm hotkey toàn cục để tổ hợp đang có không chạy.
    fn start_recording(self: &Settings, i: usize) {
        self.stop_recording();
        emit(Action::PauseHotkeys);
        self.recording.set(Some(i));
        if let Some(b) = self.c.borrow().hk.get(i) {
            b.setTitle(&ns(t().st_recording));
        }
        let owner = self.target.ivars().owner.borrow().clone();
        let block = RcBlock::new(move |e: NonNull<NSEvent>| -> *mut NSEvent {
            let e = unsafe { e.as_ref() };
            match owner.upgrade() {
                Some(s) if s.recording.get().is_some() => {
                    s.on_record_key(e);
                    std::ptr::null_mut()
                }
                _ => e as *const NSEvent as *mut NSEvent,
            }
        });
        let mask = NSEventMask::KeyDown | NSEventMask::FlagsChanged;
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
        *self.monitor.borrow_mut() = monitor;
    }

    fn stop_recording(&self) {
        if let Some(m) = self.monitor.borrow_mut().take() {
            unsafe { NSEvent::removeMonitor(&m) };
        }
        if self.recording.take().is_some() {
            emit(Action::ResumeHotkeys);
        }
    }

    fn on_record_key(&self, e: &NSEvent) {
        let Some(i) = self.recording.get() else { return };
        let f = e.modifierFlags();
        let mods = HkMods {
            ctrl: f.contains(NSEventModifierFlags::Control),
            alt: f.contains(NSEventModifierFlags::Option),
            shift: f.contains(NSEventModifierFlags::Shift),
            win: f.contains(NSEventModifierFlags::Command),
        };
        let code = e.keyCode();
        let is_key = e.r#type() == objc2_app_kit::NSEventType::KeyDown;
        if is_key && code == 0x35 {
            // Esc: huỷ, giữ phím cũ.
            self.stop_recording();
            if let Some(v) = self.view.borrow_mut().take() {
                self.refresh(v);
            }
            return;
        }
        if is_key && !mods.any() && (code == 0x33 || code == 0x75) {
            // Delete: tắt phím tắt.
            self.stop_recording();
            self.patch(json!({ "hotkeys": { HOTKEYS[i].0: "" } }));
            return;
        }
        let vk = if is_key {
            vk_from_mac(code)
        } else {
            // flagsChanged: chỉ hiện modifier đang giữ.
            Some(0x10)
        };
        let Some(vk) = vk else { return };
        let r = if is_key { record(vk, mods) } else { mods.partial().map_or(Recorded::Ignored, Recorded::Incomplete) };
        match r {
            Recorded::Accelerator(a) => {
                self.stop_recording();
                if let Some(b) = self.c.borrow().hk.get(i) {
                    b.setTitle(&ns(&show_key(&a)));
                }
                self.patch(json!({ "hotkeys": { HOTKEYS[i].0: a } }));
            }
            Recorded::Incomplete(p) => {
                if let Some(b) = self.c.borrow().hk.get(i) {
                    b.setTitle(&ns(&show_key(&p.replace('…', ""))));
                }
            }
            Recorded::Ignored => {}
        }
    }
}
