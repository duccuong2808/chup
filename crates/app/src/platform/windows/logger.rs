//! Log ra file `%LOCALAPPDATA%\app.chup.desktop\chup.log` (ghi đè mỗi lần khởi động).

use std::{
    fs::File,
    io::Write,
    path::Path,
    sync::Mutex,
    time::Instant,
};

struct FileLogger {
    file: Mutex<Option<File>>,
    start: Instant,
}

impl log::Log for FileLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info || m.target().starts_with("chup")
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let t = self.start.elapsed();
            let _ = writeln!(f, "[{:>8.3}s {:<5}] {}", t.as_secs_f64(), r.level(), r.args());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = f.flush();
        }
    }
}

pub fn init(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let logger = Box::new(FileLogger { file: Mutex::new(File::create(path).ok()), start: Instant::now() });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}
