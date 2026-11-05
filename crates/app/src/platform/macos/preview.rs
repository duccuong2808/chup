//! Preview khi bấm thumbnail (SPEC 17). Tạm thời (Mốc 1): mở ảnh bằng app mặc định; Mốc 2 thay bằng cửa sổ của Chup.

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

use chup_core::{coords::Rect, ocr::TextLayout, preview::View};
use image::RgbaImage;
use objc2::MainThreadMarker;

pub struct Preview {
    pending: RefCell<Option<PathBuf>>,
}

impl Preview {
    pub fn create(_mtm: MainThreadMarker) -> Rc<Preview> {
        Rc::new(Preview { pending: RefCell::new(None) })
    }
    pub fn request(&self, path: PathBuf) {
        *self.pending.borrow_mut() = Some(path);
    }
    pub fn is_pending(&self, path: &PathBuf) -> bool {
        self.pending.borrow().as_ref() == Some(path)
    }
    pub fn cancel_pending(&self) {
        *self.pending.borrow_mut() = None;
    }
    pub fn show(&self, path: PathBuf, _img: RgbaImage, _view: View, _work: Rect, _top: bool, _opacity: u8) {
        self.cancel_pending();
        let _ = super::util::shell_open(&path);
    }
    pub fn is_visible(&self) -> bool {
        false
    }
    pub fn apply_config(&self, _top: bool, _opacity: u8) {}
    pub fn set_ocr_busy(&self, _busy: Option<PathBuf>) {}
    pub fn set_text(&self, _path: &std::path::Path, _text: Arc<TextLayout>) {}
    pub fn image_of(&self, _path: &std::path::Path) -> Option<RgbaImage> {
        None
    }
}
