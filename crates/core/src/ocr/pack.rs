//! Gói dữ liệu OCR tiếng Việt: `vie` + `eng` của tessdata_fast, tải từ GitHub cộng đồng
//! (repo Chup không phát hành file OCR). Engine `chup-ocr.exe` nằm sẵn trong bộ cài, không thuộc gói.
//!
//! Cài: tải từng file vào `<ocr>/vi-N.tmp/`, kiểm tra kích thước và SHA-256, xong hết thì rename thành `<ocr>/vi-N/`.
//! Lỗi bất kỳ bước nào thì xoá thư mục tạm: không bao giờ để lại gói dở dang ở chỗ app dùng.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use sha2::{Digest, Sha256};

/// Tên thư mục của gói. Đổi dữ liệu (commit khác) thì tăng số để gói cũ tự bị dọn.
pub const PACK_DIR: &str = "vi-1";
/// Commit ghim của https://github.com/tesseract-ocr/tessdata_fast (2024-08-01).
pub const TESSDATA_COMMIT: &str = "87416418657359cb625c412a48b6e1d6d41c29bd";
/// Ngôn ngữ truyền cho Tesseract.
pub const TESSERACT_LANGS: &str = "vie+eng";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub const FILES: [PackFile; 2] = [
    PackFile {
        name: "vie.traineddata",
        size: 531_275,
        sha256: "79df64caf7bcfb2a27df5042ecb6121e196eada34da774956995747636d5bfa1",
    },
    PackFile {
        name: "eng.traineddata",
        size: 4_113_088,
        sha256: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2",
    },
];

pub fn url(f: &PackFile) -> String {
    format!("https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/{TESSDATA_COMMIT}/{}", f.name)
}

pub fn total_size() -> u64 {
    FILES.iter().map(|f| f.size).sum()
}

/// Dung lượng cho người dùng xem, vd. "4,4 MB".
pub fn size_label() -> String {
    format!("{:.1} MB", total_size() as f64 / 1_048_576.0).replace('.', ",")
}

/// Thư mục gói (truyền cho Tesseract làm `tessdata`).
pub fn dir(ocr_root: &Path) -> PathBuf {
    ocr_root.join(PACK_DIR)
}

fn tmp_dir(ocr_root: &Path) -> PathBuf {
    ocr_root.join(format!("{PACK_DIR}.tmp"))
}

/// Đã có gói: đủ file, đúng kích thước. Rẻ, gọi mỗi lần OCR được. SHA-256 chỉ kiểm tra lúc cài.
pub fn is_installed(ocr_root: &Path) -> bool {
    let d = dir(ocr_root);
    FILES.iter().all(|f| fs::metadata(d.join(f.name)).map(|m| m.is_file() && m.len() == f.size).unwrap_or(false))
}

/// Xoá mọi thứ trong thư mục OCR trừ gói hiện tại (gói cũ sau khi nâng cấp, thư mục tạm bị bỏ dở).
pub fn cleanup_old(ocr_root: &Path) {
    let Ok(entries) = fs::read_dir(ocr_root) else { return };
    for e in entries.flatten() {
        if e.file_name() != PACK_DIR {
            let p = e.path();
            let r = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
            if let Err(err) = r {
                log::warn!("không xoá được {}: {err}", p.display());
            }
        }
    }
}

/// Xoá gói (nút "Xoá gói" trong Cài đặt).
pub fn remove(ocr_root: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir(ocr_root)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Không xoá được gói OCR: {e}")),
        _ => Ok(()),
    }
}

/// Tải một URL theo luồng. `on_chunk` trả `false` để dừng giữa chừng (huỷ, file sai).
pub trait Fetch {
    fn get(&mut self, url: &str, on_chunk: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), String>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Mạng hoặc HTTP lỗi (chi tiết chỉ để ghi log).
    Network(String),
    /// File tải về sai kích thước hoặc SHA-256.
    Corrupt(String),
    /// Không ghi được đĩa.
    Io(String),
    Cancelled,
    /// Lần tải trước (vừa huỷ) chưa dừng hẳn.
    Busy,
}

impl InstallError {
    /// Câu báo cho người dùng (tiếng Việt, không lộ chi tiết kỹ thuật).
    pub fn user_message(&self) -> &'static str {
        match self {
            InstallError::Network(_) => "Không tải được gói. Kiểm tra mạng rồi thử lại.",
            InstallError::Corrupt(_) => "File tải về bị lỗi. Hãy thử lại.",
            InstallError::Io(_) => "Không ghi được gói vào máy. Kiểm tra dung lượng ổ đĩa rồi thử lại.",
            InstallError::Cancelled => "Đã huỷ tải.",
            InstallError::Busy => "Lần tải trước đang dừng. Hãy thử lại sau vài giây.",
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Tải và cài gói. `progress(đã tải, tổng)` gọi sau mỗi đoạn. `cancel` được kiểm tra sau mỗi đoạn.
pub fn install(
    fetch: &mut dyn Fetch,
    ocr_root: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &AtomicBool,
) -> Result<(), InstallError> {
    let tmp = tmp_dir(ocr_root);
    let result = install_into(fetch, &tmp, progress, cancel).and_then(|()| {
        let dst = dir(ocr_root);
        if dst.exists() {
            fs::remove_dir_all(&dst).map_err(|e| InstallError::Io(e.to_string()))?;
        }
        fs::rename(&tmp, &dst).map_err(|e| InstallError::Io(e.to_string()))
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

fn install_into(
    fetch: &mut dyn Fetch,
    tmp: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &AtomicBool,
) -> Result<(), InstallError> {
    let io = |e: std::io::Error| InstallError::Io(e.to_string());
    if tmp.exists() {
        fs::remove_dir_all(tmp).map_err(io)?;
    }
    fs::create_dir_all(tmp).map_err(io)?;
    let total = total_size();
    let mut done_before = 0u64;
    for f in &FILES {
        let mut out = fs::File::create(tmp.join(f.name)).map_err(io)?;
        let mut hasher = Sha256::new();
        let mut written = 0u64;
        // Lý do dừng giữa chừng (do mình chủ động), ưu tiên hơn lỗi Fetch trả về.
        let mut stop: Option<InstallError> = None;
        let fetched = fetch.get(&url(f), &mut |chunk| {
            if cancel.load(Ordering::Relaxed) {
                stop = Some(InstallError::Cancelled);
                return false;
            }
            written += chunk.len() as u64;
            if written > f.size {
                stop = Some(InstallError::Corrupt(format!("{} lớn hơn {} byte", f.name, f.size)));
                return false;
            }
            if let Err(e) = out.write_all(chunk) {
                stop = Some(InstallError::Io(e.to_string()));
                return false;
            }
            hasher.update(chunk);
            progress(done_before + written, total);
            true
        });
        if let Some(e) = stop {
            return Err(e);
        }
        fetched.map_err(InstallError::Network)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled);
        }
        if written != f.size {
            return Err(InstallError::Corrupt(format!("{}: {written}/{} byte", f.name, f.size)));
        }
        let got = hex(&hasher.finalize());
        if got != f.sha256 {
            return Err(InstallError::Corrupt(format!("{}: SHA-256 {got}", f.name)));
        }
        out.sync_all().map_err(io)?;
        done_before += f.size;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Fetcher giả: trả nội dung theo URL, chia đoạn 64 KiB.
    struct Fake {
        files: HashMap<String, Vec<u8>>,
        fail: Option<String>,
        requested: Vec<String>,
    }

    impl Fetch for Fake {
        fn get(&mut self, url: &str, on_chunk: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), String> {
            self.requested.push(url.to_string());
            if self.fail.as_deref().is_some_and(|f| url.ends_with(f)) {
                on_chunk(b"partial");
                return Err("mất mạng".into());
            }
            let body = self.files.get(url).ok_or("404")?;
            for c in body.chunks(64 << 10) {
                if !on_chunk(c) {
                    return Err("dừng".into());
                }
            }
            Ok(())
        }
    }

    fn root() -> PathBuf {
        let d = std::env::temp_dir().join(format!("chup-pack-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Dữ liệu thật đã tải sẵn (nếu có): `CHUP_TESSDATA=<thư mục>`. Không có thì test dùng nội dung giả,
    /// khi đó SHA-256 sẽ sai và chỉ các test về lỗi chạy được.
    fn real_files() -> Option<HashMap<String, Vec<u8>>> {
        let d = PathBuf::from(std::env::var_os("CHUP_TESSDATA")?);
        FILES.iter().map(|f| Some((url(f), fs::read(d.join(f.name)).ok()?))).collect()
    }

    fn fake_files() -> HashMap<String, Vec<u8>> {
        FILES.iter().map(|f| (url(f), vec![7u8; f.size as usize])).collect()
    }

    #[test]
    fn manifest_is_consistent() {
        assert_eq!(total_size(), 4_644_363);
        assert_eq!(size_label(), "4,4 MB");
        for f in &FILES {
            assert_eq!(f.sha256.len(), 64);
            assert!(url(f).starts_with("https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/8741641"));
        }
    }

    #[test]
    fn wrong_checksum_leaves_nothing_behind() {
        let r = root();
        let mut f = Fake { files: fake_files(), fail: None, requested: vec![] };
        let mut last = (0, 0);
        let err = install(&mut f, &r, &mut |d, t| last = (d, t), &AtomicBool::new(false)).unwrap_err();
        assert!(matches!(err, InstallError::Corrupt(_)), "{err:?}");
        assert_eq!(last, (FILES[0].size, total_size()), "dừng sau file đầu");
        assert!(!is_installed(&r));
        assert!(!tmp_dir(&r).exists() && !dir(&r).exists());
        fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn network_error_and_oversize_and_cancel() {
        let r = root();
        let mut f = Fake { files: fake_files(), fail: Some("vie.traineddata".into()), requested: vec![] };
        let err = install(&mut f, &r, &mut |_, _| {}, &AtomicBool::new(false)).unwrap_err();
        assert_eq!(err, InstallError::Network("mất mạng".into()));
        assert_eq!(err.user_message(), "Không tải được gói. Kiểm tra mạng rồi thử lại.");
        assert!(!tmp_dir(&r).exists());

        let mut files = fake_files();
        files.get_mut(&url(&FILES[0])).unwrap().push(0);
        let mut f = Fake { files, fail: None, requested: vec![] };
        let err = install(&mut f, &r, &mut |_, _| {}, &AtomicBool::new(false)).unwrap_err();
        assert!(matches!(err, InstallError::Corrupt(ref m) if m.contains("lớn hơn")), "{err:?}");

        let mut f = Fake { files: fake_files(), fail: None, requested: vec![] };
        let err = install(&mut f, &r, &mut |_, _| {}, &AtomicBool::new(true)).unwrap_err();
        assert_eq!(err, InstallError::Cancelled);
        assert!(!tmp_dir(&r).exists());
        fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn install_real_data_then_cleanup_and_remove() {
        let Some(files) = real_files() else {
            eprintln!("bỏ qua: chưa đặt CHUP_TESSDATA");
            return;
        };
        let r = root();
        // Rác từ lần trước: thư mục tạm dở dang và gói cũ.
        fs::create_dir_all(tmp_dir(&r)).unwrap();
        fs::write(tmp_dir(&r).join("vie.traineddata"), b"cut").unwrap();
        fs::create_dir_all(r.join("vi-0")).unwrap();
        let mut f = Fake { files, fail: None, requested: vec![] };
        install(&mut f, &r, &mut |_, _| {}, &AtomicBool::new(false)).unwrap();
        assert!(is_installed(&r));
        assert_eq!(f.requested.len(), 2);
        cleanup_old(&r);
        assert!(!r.join("vi-0").exists() && dir(&r).exists());
        // Thiếu một file = chưa có gói.
        fs::remove_file(dir(&r).join("eng.traineddata")).unwrap();
        assert!(!is_installed(&r));
        remove(&r).unwrap();
        remove(&r).unwrap();
        assert!(!dir(&r).exists());
        fs::remove_dir_all(r).unwrap();
    }
}
