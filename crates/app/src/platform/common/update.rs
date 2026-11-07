//! Tự cập nhật, phần chung Windows / macOS (SPEC 18, chup-22n): hỏi GitHub bản mới nhất, tải bộ cài vào
//! `<cache>/update/chup-<v>/` (kiểm tra kích thước và SHA-256 như gói OCR). Cài thì mỗi OS một kiểu (`updater.rs`).
//! Mọi hàm ở đây chạy ở thread nền.

use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};

use chup_core::{
    ocr::pack::Fetch,
    update::{self, Package, Update},
};

/// Trả lời của API GitHub dài cỡ vài chục KB; quá chừng này là lạ, dừng.
const MAX_JSON: usize = 1 << 20;

/// Thư mục tải bộ cài.
pub fn root(cache_dir: &Path) -> PathBuf {
    cache_dir.join("update")
}

/// Giây Unix lúc này (hạn tự kiểm tra lưu trong config).
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Hỏi GitHub bản mới nhất. Dọn bộ cài lần trước (cập nhật đã xong thì không cần nữa).
pub fn check(fetch: &mut dyn Fetch, pkg: Package, root: &Path) -> Result<Update, String> {
    update::cleanup(root);
    let mut body = Vec::new();
    fetch.get(update::LATEST_API, &mut |c| {
        body.extend_from_slice(c);
        body.len() <= MAX_JSON
    })?;
    let text = String::from_utf8(body).map_err(|e| format!("JSON không phải UTF-8: {e}"))?;
    update::parse_latest(&text, pkg)
}

/// Tải bộ cài của `u` (đã kiểm tra SHA-256). Trả đường dẫn file.
pub fn download(fetch: &mut dyn Fetch, u: &Update, root: &Path) -> Result<PathBuf, String> {
    let pack = u.pack.as_ref().ok_or("Release không có bộ cài cho máy này")?;
    update::cleanup(root);
    let t0 = std::time::Instant::now();
    pack.install(fetch, root, &mut |_| {}, &AtomicBool::new(false)).map_err(|e| format!("tải bộ cài: {e:?}"))?;
    log::info!("update: tải {} ({} byte) mất {:?}", pack.files[0].name, pack.total_size(), t0.elapsed());
    u.file(root).ok_or_else(|| "không có file bộ cài".into())
}
