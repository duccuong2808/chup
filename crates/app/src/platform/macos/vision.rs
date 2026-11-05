//! OCR bằng Apple Vision (SPEC 15, thiết kế macOS mục 3). Tạm thời (Mốc 1): chưa có.

use chup_core::config::OcrLanguage;
use image::RgbaImage;

use super::{app::Action, win::emit};

pub struct Job {
    pub id: u32,
    pub lang: OcrLanguage,
    pub img: RgbaImage,
}

pub struct Worker;

impl Worker {
    pub fn start() -> Worker {
        Worker
    }
    pub fn submit(&self, job: Job) {
        emit(Action::OcrDone(job.id, Err("OCR chưa có trên macOS".into())));
    }
    pub fn cancel(&self) {}
}
