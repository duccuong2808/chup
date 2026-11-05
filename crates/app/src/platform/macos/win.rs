//! Một UI thread (main thread của AppKit). Cửa sổ, tray, hotkey, worker không gọi thẳng vào App: chúng
//! `emit(Action)`, Action được gom vào hàng đợi và xử lý trên main thread (giống `WM_APP_ACTION` của Windows).
//! Hẹn giờ trên main thread bằng `NSTimer` (closure không cần `Send`).

use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2_foundation::NSTimer;

use super::app::Action;

static QUEUE: Mutex<Vec<Action>> = Mutex::new(Vec::new());
/// Đã hẹn `drain` trên main queue, chưa chạy.
static SCHEDULED: AtomicBool = AtomicBool::new(false);

/// Hàm xử lý hàng đợi trên main thread (mod.rs đặt).
static DRAIN: Mutex<Option<fn()>> = Mutex::new(None);

pub fn set_drain(f: fn()) {
    *DRAIN.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

/// Gửi Action từ bất kỳ thread nào.
pub fn emit(a: Action) {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push(a);
    schedule();
}

/// Hẹn `drain` trên main queue (gộp: nhiều `emit` liên tiếp chỉ hẹn một lần).
pub fn schedule() {
    if SCHEDULED.swap(true, Ordering::AcqRel) {
        return;
    }
    DispatchQueue::main().exec_async(|| {
        SCHEDULED.store(false, Ordering::Release);
        let f = *DRAIN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = f {
            f();
        }
    });
}

pub fn take_actions() -> Vec<Action> {
    std::mem::take(&mut *QUEUE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Hẹn giờ một lần trên main thread; thả `Timer` (hoặc gọi `cancel`) là huỷ.
pub struct Timer(Retained<NSTimer>);

impl Timer {
    pub fn cancel(&self) {
        self.0.invalidate();
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.0.invalidate();
    }
}

/// Chạy `f` sau `ms` mili giây trên main thread (phải gọi từ main thread).
pub fn after(ms: u32, f: impl FnOnce() + 'static) -> Timer {
    let cell = RefCell::new(Some(f));
    let block = RcBlock::new(move |_t: std::ptr::NonNull<NSTimer>| {
        if let Some(f) = cell.borrow_mut().take() {
            f();
        }
    });
    // Common modes: vẫn chạy khi menu (tray) đang mở hay đang kéo cửa sổ.
    let t = unsafe { NSTimer::timerWithTimeInterval_repeats_block(ms as f64 / 1000.0, false, &block) };
    unsafe { objc2_foundation::NSRunLoop::mainRunLoop().addTimer_forMode(&t, objc2_foundation::NSRunLoopCommonModes) };
    Timer(t)
}

/// Gửi Action sau `ms` mili giây.
pub fn emit_after(ms: u32, make: impl FnOnce() -> Action + 'static) -> Timer {
    after(ms, move || emit(make()))
}
