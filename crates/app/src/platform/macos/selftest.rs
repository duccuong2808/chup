//! Tự kiểm tra cho dev (`CHUP_SELFTEST=<thư mục>`): chạy luồng chính bằng sự kiện bơm thẳng vào handler của cửa
//! sổ (như chuột thật), xuất ảnh mỗi cửa sổ đang hiện ra PNG để xem, rồi thoát. Không cần quyền Accessibility.
//! Freeze dùng ảnh tổng hợp nếu chưa có quyền Screen Recording; có quyền thì chụp thật một vùng nhỏ.
//! Kết quả: `<thư mục>/*.png`, `<thư mục>/shots/*.png`, và log `~/Library/Logs/Chup/chup.log`.

use std::path::{Path, PathBuf};

use chup_core::{capture, coords::Rect};
use image::RgbaImage;

use super::{
    app::{Action, App},
    capture as sck,
    panel::{key, Ev, Mods},
    win::{self, emit},
};

pub fn dir() -> Option<PathBuf> {
    std::env::var_os("CHUP_SELFTEST").map(PathBuf::from)
}

/// Bắt đầu sau khi App dựng xong.
pub fn start() {
    if dir().is_some() {
        std::mem::forget(win::emit_after(800, || Action::SelfTest(0)));
    }
}

fn next(step: u32, ms: u32) {
    std::mem::forget(win::emit_after(ms, move || Action::SelfTest(step)));
}

fn save(dir: &Path, name: &str, img: Option<RgbaImage>) {
    match img {
        Some(i) => match i.save(dir.join(name)) {
            Ok(()) => log::info!("selftest: {name} {}×{}", i.width(), i.height()),
            Err(e) => log::error!("selftest: lưu {name}: {e}"),
        },
        None => log::error!("selftest: {name}: không có ảnh"),
    }
}

/// Ảnh tổng hợp: dải màu kèm lưới (để thấy rõ chỗ crop và scale).
pub fn synthetic(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let grid = x % 100 < 2 || y % 100 < 2;
        if grid {
            image::Rgba([40, 40, 40, 255])
        } else {
            image::Rgba([(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 160, 255])
        }
    })
}

fn down(p: (i32, i32)) -> Ev {
    Ev::Down { p, clicks: 1, mods: Mods::default() }
}

impl App {
    pub fn self_test(&mut self, step: u32) {
        let Some(dir) = dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        log::info!("selftest: bước {step}");
        match step {
            0 => {
                log::info!("selftest: màn hình {:?}, quyền ghi màn hình: {}", self.model.monitors, sck::has_permission());
                let shots = dir.join("shots");
                emit(Action::ConfigPatch(serde_json::json!({
                    "saveDir": shots, "copyToClipboard": false,
                })));
                emit(Action::StartSelection);
                next(1, 700);
            }
            1 => {
                let o = self.focus_overlay();
                if let Some(o) = &o {
                    o.inject(down((200, 200)));
                    o.inject(Ev::Drag { p: (500, 300) });
                    o.inject(Ev::Drag { p: (1000, 700) });
                    save(&dir, "01-overlay-drag.png", o.snapshot());
                    o.inject(Ev::Up { p: (1000, 700) });
                }
                next(2, 700);
            }
            2 => {
                save(&dir, "02-frame.png", self.frame_ref().snapshot());
                save(&dir, "02-toolbar.png", self.toolbar_ref().snapshot());
                // Rê chuột lên nút đầu (Chụp).
                let t = self.toolbar_ref();
                if let Some(img) = t.snapshot() {
                    t.inject(Ev::Move { p: (img.width() as i32 - 200, img.height() as i32 / 2) });
                }
                save(&dir, "03-toolbar-hover.png", t.snapshot());
                emit(Action::ToggleFade);
                next(3, 500);
            }
            3 => {
                save(&dir, "04-frame-faded.png", self.frame_ref().snapshot());
                save(&dir, "04-toolbar-faded.png", self.toolbar_ref().snapshot());
                // Lưu một ảnh tổng hợp như vừa chụp: stack hiện thẻ.
                let opts = self.model.persist_options();
                let img = synthetic(800, 450);
                emit(Action::Persisted(capture::persist_shot(&img, &opts)));
                next(4, 800);
            }
            4 => {
                let s = self.stack_ref();
                save(&dir, "05-stack.png", s.snapshot());
                s.inject(Ev::Enter);
                s.inject(Ev::Move { p: (60, 60) });
                save(&dir, "06-stack-hover.png", s.snapshot());
                emit(Action::Toast(chup_core::model::Toast::info("Thông báo thử: đã lưu ảnh vào thư mục Chup")));
                next(5, 500);
            }
            5 => {
                save(&dir, "07-stack-toast.png", self.stack_ref().snapshot());
                emit(Action::CloseRegion);
                emit(Action::FreezeNow);
                next(6, 1200);
            }
            6 => {
                let o = self.focus_overlay();
                if let Some(o) = &o {
                    o.inject(down((300, 300)));
                    o.inject(Ev::Drag { p: (700, 500) });
                    o.inject(Ev::Drag { p: (1300, 900) });
                    save(&dir, "08-freeze-drag.png", o.snapshot());
                    o.inject(Ev::Up { p: (1300, 900) });
                    save(&dir, "09-freeze-selected.png", o.snapshot());
                    o.inject(Ev::Key { code: key::RETURN, mods: Mods::default() });
                }
                next(7, 1200);
            }
            7 => {
                let n = std::fs::read_dir(dir.join("shots")).map(|d| d.count()).unwrap_or(0);
                log::info!("selftest: thư mục lưu có {n} file");
                save(&dir, "10-stack-after-freeze.png", self.stack_ref().snapshot());
                if sck::has_permission() {
                    let d = dir.clone();
                    sck::grab_region(Rect::new(0, 0, 600, 400), move |r| match r {
                        Ok(img) => save(&d, "11-real-capture.png", Some(img)),
                        Err(e) => log::error!("selftest: chụp thật lỗi: {e}"),
                    });
                } else {
                    log::warn!("selftest: chưa có quyền ghi màn hình, bỏ qua chụp thật");
                }
                emit(Action::OpenSettings);
                next(8, 1500);
            }
            8 => {
                if let Some(s) = self.settings_ref() {
                    s.snapshot_tabs(&dir);
                    s.close();
                }
                ocr_test(&dir);
                next(9, 3000);
            }
            9 => {
                self.open_preview_for_test(dir.join("12-ocr-input.png"));
                next(10, 1500);
            }
            10 => {
                let p = self.preview_ref();
                save(&dir, "13-preview.png", p.snapshot());
                p.inject(Ev::Enter);
                p.inject(Ev::Move { p: (40, 40) });
                save(&dir, "14-preview-hover.png", p.snapshot());
                // OCR nền khi mở Preview (SPEC 17.2).
                next(11, 2500);
            }
            11 => {
                let p = self.preview_ref();
                // Bấm đúp lên từ thứ hai ("màn").
                p.inject(Ev::Down { p: (190, 80), clicks: 2, mods: Mods::default() });
                p.inject(Ev::Up { p: (190, 80) });
                save(&dir, "15-preview-select-word.png", p.snapshot());
                p.inject(Ev::Scroll { p: (300, 80), dy: 3.0, lines: true, mods: Mods::default() });
                save(&dir, "16-preview-zoom.png", p.snapshot());
                p.inject(Ev::Key { code: key::ESCAPE, mods: Mods::default() });
                p.inject(Ev::Key { code: key::ESCAPE, mods: Mods::default() });
                log::info!("selftest: Preview đóng: {}", !p.is_visible());
                next(12, 500);
            }
            _ => {
                log::info!("selftest: xong");
                emit(Action::Quit);
            }
        }
    }
}

/// Câu tiếng Việt vẽ bằng CoreText -> Vision, so với câu gốc.
fn ocr_test(dir: &Path) {
    use chup_ui::Canvas as _;
    const TEXT: &str = "Chụp màn hình tiếng Việt: ảnh, chữ, dấu hỏi ngã nặng";
    let (w, h) = (1400, 120);
    let Some(s) = super::gfx::Surface::new(w, h) else { return };
    let Some(c) = super::gfx::Canvas::new(&s) else { return };
    let Some(f) = super::gfx::Font::new(40.0, false, chup_ui::Align::Leading, false) else { return };
    c.begin(chup_ui::Color::WHITE);
    c.text(TEXT, &f, chup_ui::rectf(20.0, 0.0, w as f32 - 40.0, h as f32), chup_ui::Color::hex(0x111111));
    c.end();
    drop(c);
    let Some(img) = super::gfx::to_rgba_image(&s) else { return };
    let _ = img.save(dir.join("12-ocr-input.png"));
    std::thread::spawn(move || {
        let r = objc2::rc::autoreleasepool(|_| super::vision::recognize(&img, chup_core::config::OcrLanguage::Vi));
        match r {
            Ok(lines) => {
                let got = chup_core::ocr::TextLayout::build(lines, 1.0).all_text();
                log::info!("selftest: OCR {:?} (khớp: {})", got, got.trim() == TEXT);
            }
            Err(e) => log::error!("selftest: OCR lỗi: {e}"),
        }
    });
}
