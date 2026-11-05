//! Cửa sổ của Chup trên macOS: `NSPanel` không viền, trong suốt, layer-backed (thiết kế macOS mục 2.3).
//!
//! - View lật trục (`isFlipped`): toạ độ sự kiện tính từ góc trên-trái như Windows, đổi sang physical px.
//! - Sự kiện chuột / phím giao cho `Handler` của cửa sổ (overlay, frame, toolbar, stack). Handler không được giữ
//!   borrow state khi gọi AppKit (như quy tắc của vỏ Windows).
//! - Vẽ: `present` đưa ảnh của `Surface` lên `layer.contents` (scale theo màn hình).
//! - Kéo chuột: AppKit tự gửi `mouseDragged` cho view đã nhận `mouseDown`, kể cả khi ra ngoài cửa sổ (như SetCapture).

use std::{
    cell::{Cell, RefCell},
    rc::Weak,
};

use chup_core::coords::Rect;
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::AnyObject,
    DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSCursor, NSCursorFrameResizeDirections, NSCursorFrameResizePosition, NSEvent, NSEventModifierFlags, NSPanel, NSResponder, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowLevel,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::CATransaction;

use super::{gfx::Surface, util};

/// Modifier đang giữ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub cmd: bool,
}

impl Mods {
    pub fn from_flags(f: NSEventModifierFlags) -> Self {
        Mods {
            shift: f.contains(NSEventModifierFlags::Shift),
            ctrl: f.contains(NSEventModifierFlags::Control),
            alt: f.contains(NSEventModifierFlags::Option),
            cmd: f.contains(NSEventModifierFlags::Command),
        }
    }

    /// Modifier đang giữ lúc này (ngoài sự kiện).
    pub fn now() -> Self {
        Self::from_flags(NSEvent::modifierFlags_class())
    }
}

/// Sự kiện đã đổi sang physical px (toạ độ trong cửa sổ, gốc trên-trái).
#[derive(Clone, Debug)]
pub enum Ev {
    Down { p: (i32, i32), clicks: isize, mods: Mods },
    Drag { p: (i32, i32) },
    Up { p: (i32, i32) },
    Move { p: (i32, i32) },
    Enter,
    Leave,
    RightDown { p: (i32, i32) },
    RightUp { p: (i32, i32) },
    /// `dy` > 0: cuộn lên. `lines`: chuột có nấc (không phải trackpad).
    Scroll { p: (i32, i32), dy: f64, lines: bool, mods: Mods },
    Key { code: u16, mods: Mods },
    Flags { mods: Mods },
}

/// Mã phím ảo của macOS (kVK_*).
pub mod key {
    pub const ESCAPE: u16 = 0x35;
    pub const RETURN: u16 = 0x24;
    pub const KEYPAD_ENTER: u16 = 0x4C;
    pub const UP: u16 = 0x7E;
    pub const DOWN: u16 = 0x7D;
    pub const A: u16 = 0x00;
    pub const C: u16 = 0x08;
    pub const W: u16 = 0x0D;
    pub const EQUAL: u16 = 0x18;
    pub const MINUS: u16 = 0x1B;
    pub const ZERO: u16 = 0x1D;
    pub const KEYPAD_PLUS: u16 = 0x45;
    pub const KEYPAD_MINUS: u16 = 0x4E;
    pub const KEYPAD_0: u16 = 0x52;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Arrow,
    Cross,
    Hand,
    IBeam,
    Move,
    ResizeNS,
    ResizeEW,
    ResizeNWSE,
    ResizeNESW,
}

impl Cursor {
    pub fn set(self) {
        let c = match self {
            Cursor::Arrow => NSCursor::arrowCursor(),
            Cursor::Cross => NSCursor::crosshairCursor(),
            Cursor::Hand => NSCursor::pointingHandCursor(),
            Cursor::IBeam => NSCursor::IBeamCursor(),
            Cursor::Move => NSCursor::openHandCursor(),
            Cursor::ResizeNS => resize(NSCursorFrameResizePosition::Top),
            Cursor::ResizeEW => resize(NSCursorFrameResizePosition::Left),
            Cursor::ResizeNWSE => resize(NSCursorFrameResizePosition::TopLeft),
            Cursor::ResizeNESW => resize(NSCursorFrameResizePosition::TopRight),
        };
        c.set();
    }
}

fn resize(p: NSCursorFrameResizePosition) -> Retained<NSCursor> {
    NSCursor::frameResizeCursorFromPosition_inDirections(p, NSCursorFrameResizeDirections::All)
}

pub trait Handler {
    fn event(&self, ev: Ev);
    /// Con trỏ ở điểm `p` (px trong cửa sổ).
    fn cursor(&self, _p: (i32, i32)) -> Cursor {
        Cursor::Arrow
    }
}

pub struct ViewIvars {
    handler: RefCell<Option<Weak<dyn Handler>>>,
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
}

define_class!(
    // SAFETY: NSView không có yêu cầu riêng khi kế thừa; ChupView không cài Drop.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ChupView"]
    #[ivars = ViewIvars]
    pub struct ChupView;

    impl ChupView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        // Bấm lần đầu vào panel không active vẫn là một lần bấm (không bị nuốt để kích hoạt cửa sổ).
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            let p = self.px(e);
            self.send(Ev::Down { p, clicks: e.clickCount(), mods: Mods::from_flags(e.modifierFlags()) });
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, e: &NSEvent) {
            self.send(Ev::Drag { p: self.px(e) });
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            self.send(Ev::Up { p: self.px(e) });
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            self.send(Ev::Move { p: self.px(e) });
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _e: &NSEvent) {
            self.send(Ev::Enter);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _e: &NSEvent) {
            self.send(Ev::Leave);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, e: &NSEvent) {
            self.send(Ev::RightDown { p: self.px(e) });
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, e: &NSEvent) {
            self.send(Ev::RightUp { p: self.px(e) });
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, e: &NSEvent) {
            let lines = !e.hasPreciseScrollingDeltas();
            self.send(Ev::Scroll { p: self.px(e), dy: e.scrollingDeltaY(), lines, mods: Mods::from_flags(e.modifierFlags()) });
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, e: &NSEvent) {
            self.send(Ev::Key { code: e.keyCode(), mods: Mods::from_flags(e.modifierFlags()) });
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, e: &NSEvent) {
            self.send(Ev::Flags { mods: Mods::from_flags(e.modifierFlags()) });
        }

        #[unsafe(method(cursorUpdate:))]
        fn cursor_update(&self, e: &NSEvent) {
            let p = self.px(e);
            let h = self.ivars().handler.borrow().as_ref().and_then(Weak::upgrade);
            if let Some(h) = h {
                h.cursor(p).set();
            }
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            if let Some(old) = self.ivars().tracking.borrow_mut().take() {
                self.removeTrackingArea(&old);
            }
            let opts = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::CursorUpdate
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect;
            let owner: &AnyObject = self.as_ref();
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    <NSTrackingArea as objc2::AnyThread>::alloc(),
                    NSRect::ZERO,
                    opts,
                    Some(owner),
                    None,
                )
            };
            self.addTrackingArea(&area);
            *self.ivars().tracking.borrow_mut() = Some(area);
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
        }
    }
);

impl ChupView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ViewIvars { handler: RefCell::new(None), tracking: RefCell::new(None) });
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn scale(&self) -> f64 {
        self.window().map_or(1.0, |w| w.backingScaleFactor())
    }

    /// Điểm của sự kiện -> px trong cửa sổ (gốc trên-trái).
    fn px(&self, e: &NSEvent) -> (i32, i32) {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        let k = self.scale();
        ((p.x * k).floor() as i32, (p.y * k).floor() as i32)
    }

    fn send(&self, ev: Ev) {
        // Lấy handler ra trước, không giữ borrow ivars khi handler chạy (có thể gọi lại vào view).
        let h = self.ivars().handler.borrow().as_ref().and_then(Weak::upgrade);
        if let Some(h) = h {
            h.event(ev);
        }
    }
}

define_class!(
    // SAFETY: NSPanel không có yêu cầu riêng khi kế thừa; ChupPanel không cài Drop.
    #[unsafe(super(NSPanel, objc2_app_kit::NSWindow, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ChupPanel"]
    #[ivars = Cell<bool>]
    pub struct ChupPanel;

    impl ChupPanel {
        // Panel không viền mặc định không thành key window: overlay cần làm key để nhận Esc / Enter.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            self.ivars().get()
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }
    }
);

/// Kiểu cửa sổ.
#[derive(Clone, Copy, Debug)]
pub struct Opts {
    /// Được làm key window (nhận phím) mà không activate app.
    pub key: bool,
    pub level: NSWindowLevel,
}

/// Trên menu bar và Dock (overlay chọn vùng).
pub const LEVEL_OVERLAY: NSWindowLevel = 1000; // NSScreenSaverWindowLevel
/// Trên cửa sổ thường và cửa sổ nổi (frame, toolbar, stack).
pub const LEVEL_UI: NSWindowLevel = 25; // NSStatusWindowLevel
pub const LEVEL_FLOATING: NSWindowLevel = 3;
pub const LEVEL_NORMAL: NSWindowLevel = 0;

pub struct Panel {
    pub window: Retained<ChupPanel>,
    pub view: Retained<ChupView>,
    /// Rect (px) đang đặt.
    rect: Cell<Rect>,
    /// Ảnh đang hiện (để tự kiểm tra: xuất ra PNG).
    last: RefCell<Option<objc2_core_foundation::CFRetained<objc2_core_graphics::CGImage>>>,
}

impl Panel {
    pub fn new(mtm: MainThreadMarker, opts: Opts) -> Panel {
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        let this = ChupPanel::alloc(mtm).set_ivars(Cell::new(opts.key));
        let window: Retained<ChupPanel> = unsafe {
            msg_send![super(this), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
        };
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(false);
        window.setHidesOnDeactivate(false);
        window.setLevel(opts.level);
        window.setAnimationBehavior(NSWindowAnimationBehavior::None);
        window.setAcceptsMouseMovedEvents(true);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        let view = ChupView::new(mtm);
        view.setWantsLayer(true);
        window.setContentView(Some(&view));
        Panel { window, view, rect: Cell::new(Rect::default()), last: RefCell::new(None) }
    }

    pub fn set_handler(&self, h: Weak<dyn Handler>) {
        *self.view.ivars().handler.borrow_mut() = Some(h);
    }

    pub fn rect(&self) -> Rect {
        self.rect.get()
    }

    /// Đặt cửa sổ ở rect px toàn desktop.
    pub fn set_rect(&self, r: Rect) {
        self.rect.set(r);
        let (frame, _) = util::to_window_frame(r);
        self.window.setFrame_display(frame, false);
    }

    /// Đưa góc trên-trái cỡ `r.w × r.h` của `s` lên cửa sổ, đặt cửa sổ ở `r`.
    pub fn present(&self, s: &Surface, r: Rect) {
        let Some(img) = s.image((r.w as i32, r.h as i32)) else { return };
        let (frame, k) = util::to_window_frame(r);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        if self.rect.get() != r {
            self.rect.set(r);
            self.window.setFrame_display(frame, false);
        }
        if let Some(layer) = self.view.layer() {
            layer.setContentsScale(k);
            let obj: &AnyObject = unsafe { &*(&*img as *const objc2_core_graphics::CGImage).cast::<AnyObject>() };
            unsafe { layer.setContents(Some(obj)) };
        }
        CATransaction::commit();
        *self.last.borrow_mut() = Some(img);
    }

    /// Hiện, không activate app. `key`: làm key window (nhận phím).
    pub fn show(&self, key: bool) {
        self.window.orderFrontRegardless();
        if key {
            self.window.makeKeyWindow();
            let r: &NSResponder = &self.view;
            self.window.makeFirstResponder(Some(r));
        }
    }

    pub fn hide(&self) {
        self.window.orderOut(None);
        *self.last.borrow_mut() = None;
        if let Some(layer) = self.view.layer() {
            unsafe { layer.setContents(None) };
        }
    }

    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    pub fn set_alpha(&self, a: f64) {
        self.window.setAlphaValue(a);
    }

    pub fn set_ignores_mouse(&self, on: bool) {
        self.window.setIgnoresMouseEvents(on);
    }

    pub fn set_level(&self, level: NSWindowLevel) {
        self.window.setLevel(level);
    }

    /// Không cho app chia sẻ / chụp màn hình thấy cửa sổ (chỉ có tác dụng với API chụp cũ).
    pub fn exclude_from_capture(&self) {
        self.window.setSharingType(objc2_app_kit::NSWindowSharingType::None);
    }

    /// Ảnh đang hiện (RGBA), cho chế độ tự kiểm tra.
    pub fn snapshot(&self) -> Option<image::RgbaImage> {
        self.last.borrow().as_ref().and_then(|i| super::capture::to_rgba(i))
    }

    pub fn raw_handle(&self) -> ViewHandle {
        ViewHandle(Retained::as_ptr(&self.view) as *mut _)
    }
}

/// Handle của view cho crate `drag` (raw-window-handle, AppKit).
pub struct ViewHandle(*mut std::ffi::c_void);

impl raw_window_handle::HasWindowHandle for ViewHandle {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let ptr = std::ptr::NonNull::new(self.0).ok_or(raw_window_handle::HandleError::Unavailable)?;
        let raw = raw_window_handle::RawWindowHandle::AppKit(raw_window_handle::AppKitWindowHandle::new(ptr));
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}
