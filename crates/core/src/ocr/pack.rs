//! Các gói OCR tiếng Việt Chup tải về, mô tả trong `manifest.json` (CI đọc cùng file để đóng gói bản Full):
//! - `data`: `vie` + `eng.traineddata` của tessdata_fast (GitHub cộng đồng).
//! - `engine`: `chup-ocr.exe` + `NOTICE-ocr.txt`, Release `ocr-engine-vN` của repo Chup. Bộ cài Full đã đặt engine
//!   cạnh `chup.exe` nên không cần gói này; bộ cài Lite và `chup.exe` chạy rời thì tải.
//!
//! Cài một gói: tải từng file vào `<ocr>/<dir>.tmp/`, kiểm tra kích thước và SHA-256, xong hết thì rename thành
//! `<ocr>/<dir>/`. Lỗi bất kỳ bước nào thì xoá thư mục tạm: không bao giờ để lại gói dở dang ở chỗ app dùng.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        LazyLock,
    },
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Ngôn ngữ truyền cho Tesseract.
pub const TESSERACT_LANGS: &str = "vie+eng";
/// Tên exe của engine (trong gói `engine` và cạnh `chup.exe` khi cài bản Full).
pub const ENGINE_EXE: &str = "chup-ocr.exe";
/// Giấy phép của Tesseract và các thư viện link tĩnh, luôn đi cùng engine.
pub const ENGINE_NOTICE: &str = "NOTICE-ocr.txt";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PackFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    /// Thư mục trong `<ocr>`. Đổi nội dung gói thì đổi tên để gói cũ bị `cleanup_old` dọn.
    pub dir: String,
    /// Nguồn, cho người đọc (trang Giới thiệu, log).
    pub source: String,
    pub base_url: String,
    pub files: Vec<PackFile>,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub data: Pack,
    pub engine: Pack,
}

static MANIFEST: LazyLock<Manifest> =
    LazyLock::new(|| serde_json::from_str(include_str!("manifest.json")).expect("manifest.json hỏng"));

/// Dữ liệu tiếng Việt (tessdata_fast).
pub fn data() -> &'static Pack {
    &MANIFEST.data
}

/// Engine `chup-ocr.exe`.
pub fn engine() -> &'static Pack {
    &MANIFEST.engine
}

impl Pack {
    pub fn url(&self, f: &PackFile) -> String {
        format!("{}{}", self.base_url, f.name)
    }

    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Thư mục gói (với `data`: truyền cho Tesseract làm `tessdata`).
    pub fn dir(&self, ocr_root: &Path) -> PathBuf {
        ocr_root.join(&self.dir)
    }

    fn tmp_dir(&self, ocr_root: &Path) -> PathBuf {
        ocr_root.join(format!("{}.tmp", self.dir))
    }

    /// Đã có gói: đủ file, đúng kích thước. Rẻ, gọi mỗi lần OCR được. SHA-256 chỉ kiểm tra lúc cài.
    pub fn is_installed(&self, ocr_root: &Path) -> bool {
        let d = self.dir(ocr_root);
        self.files.iter().all(|f| fs::metadata(d.join(&f.name)).is_ok_and(|m| m.is_file() && m.len() == f.size))
    }

    /// Xoá gói (nút "Xoá" trong Cài đặt).
    pub fn remove(&self, ocr_root: &Path) -> Result<(), String> {
        match fs::remove_dir_all(self.dir(ocr_root)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                log::warn!("xoá gói OCR {}: {e}", self.dir);
                Err(crate::i18n::t().ocr_remove_failed.into())
            }
            _ => Ok(()),
        }
    }

    /// Tải và cài gói. `progress(đã tải của gói này)` gọi sau mỗi đoạn. `cancel` được kiểm tra sau mỗi đoạn.
    pub fn install(
        &self,
        fetch: &mut dyn Fetch,
        ocr_root: &Path,
        progress: &mut dyn FnMut(u64),
        cancel: &AtomicBool,
    ) -> Result<(), InstallError> {
        let tmp = self.tmp_dir(ocr_root);
        let result = self.install_into(fetch, &tmp, progress, cancel).and_then(|()| {
            let dst = self.dir(ocr_root);
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
        &self,
        fetch: &mut dyn Fetch,
        tmp: &Path,
        progress: &mut dyn FnMut(u64),
        cancel: &AtomicBool,
    ) -> Result<(), InstallError> {
        let io = |e: std::io::Error| InstallError::Io(e.to_string());
        if tmp.exists() {
            fs::remove_dir_all(tmp).map_err(io)?;
        }
        fs::create_dir_all(tmp).map_err(io)?;
        let mut done_before = 0u64;
        for f in &self.files {
            let mut out = fs::File::create(tmp.join(&f.name)).map_err(io)?;
            let mut hasher = Sha256::new();
            let mut written = 0u64;
            // Lý do dừng giữa chừng (do mình chủ động), ưu tiên hơn lỗi Fetch trả về.
            let mut stop: Option<InstallError> = None;
            let fetched = fetch.get(&self.url(f), &mut |chunk| {
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
                progress(done_before + written);
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
}

/// Dung lượng cho người dùng xem, vd. "4,4 MB".
pub fn size_label(bytes: u64) -> String {
    format!("{} MB", crate::i18n::decimal1(bytes as f64 / 1_048_576.0))
}

/// Xoá mọi thứ trong thư mục OCR trừ các gói hiện tại: gói cũ sau khi nâng cấp, thư mục tạm bị bỏ dở,
/// engine nhúng của bản trước (`helper-*`).
pub fn cleanup_old(ocr_root: &Path) {
    let Ok(entries) = fs::read_dir(ocr_root) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        if name != data().dir.as_str() && name != engine().dir.as_str() {
            let p = e.path();
            let r = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
            if let Err(err) = r {
                log::warn!("không xoá được {}: {err}", p.display());
            }
        }
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
        let t = crate::i18n::t();
        match self {
            InstallError::Network(_) => t.dl_network,
            InstallError::Corrupt(_) => t.dl_corrupt,
            InstallError::Io(_) => t.dl_io,
            InstallError::Cancelled => t.dl_cancelled,
            InstallError::Busy => t.dl_busy,
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

    /// Gói thử với nội dung tự chọn (SHA-256 tính đúng).
    fn pack_of(contents: &[(&str, &[u8])]) -> (Pack, HashMap<String, Vec<u8>>) {
        let pack = Pack {
            dir: "t-1".into(),
            source: "test".into(),
            base_url: "https://example.test/x/".into(),
            files: contents
                .iter()
                .map(|(n, b)| PackFile { name: n.to_string(), size: b.len() as u64, sha256: hex(&Sha256::digest(b)) })
                .collect(),
        };
        let files = contents.iter().map(|(n, b)| (format!("https://example.test/x/{n}"), b.to_vec())).collect();
        (pack, files)
    }

    #[test]
    fn manifest_is_consistent() {
        let (d, e) = (data(), engine());
        assert_eq!(d.total_size(), 4_644_363);
        assert_eq!(size_label(d.total_size()), "4,4 MB");
        assert_eq!(size_label(d.total_size() + e.total_size()), "11,1 MB");
        assert_ne!(d.dir, e.dir);
        for p in [d, e] {
            assert!(p.base_url.starts_with("https://") && p.base_url.ends_with('/'));
            for f in &p.files {
                assert_eq!(f.sha256.len(), 64);
                assert!(f.size > 0);
            }
        }
        assert!(d.url(&d.files[0]).starts_with("https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/8741641"));
        assert!(e.files.iter().any(|f| f.name == ENGINE_EXE) && e.files.iter().any(|f| f.name == ENGINE_NOTICE));
        assert!(e.url(&e.files[0]).starts_with("https://github.com/duccuong2808/chup/releases/download/ocr-engine-v"));
    }

    #[test]
    fn installs_checks_and_removes() {
        let r = root();
        let (p, files) = pack_of(&[("a.bin", &[1u8; 100_000]), ("b.txt", b"license")]);
        // Rác từ lần trước: thư mục tạm dở dang.
        fs::create_dir_all(p.tmp_dir(&r)).unwrap();
        fs::write(p.tmp_dir(&r).join("a.bin"), b"cut").unwrap();
        let mut f = Fake { files, fail: None, requested: vec![] };
        let mut last = 0;
        p.install(&mut f, &r, &mut |d| last = d, &AtomicBool::new(false)).unwrap();
        assert_eq!(last, p.total_size());
        assert!(p.is_installed(&r) && !p.tmp_dir(&r).exists());
        assert_eq!(f.requested, ["https://example.test/x/a.bin", "https://example.test/x/b.txt"]);
        // Thiếu một file = chưa có gói.
        fs::remove_file(p.dir(&r).join("b.txt")).unwrap();
        assert!(!p.is_installed(&r));
        p.remove(&r).unwrap();
        p.remove(&r).unwrap();
        assert!(!p.dir(&r).exists());
        fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn wrong_checksum_leaves_nothing_behind() {
        let r = root();
        let (mut p, files) = pack_of(&[("a.bin", &[1u8; 1000]), ("b.txt", b"x")]);
        p.files[0].sha256 = "0".repeat(64);
        let mut f = Fake { files, fail: None, requested: vec![] };
        let mut last = 0;
        let err = p.install(&mut f, &r, &mut |d| last = d, &AtomicBool::new(false)).unwrap_err();
        assert!(matches!(err, InstallError::Corrupt(_)), "{err:?}");
        assert_eq!(last, 1000, "dừng sau file đầu");
        assert!(!p.is_installed(&r));
        assert!(!p.tmp_dir(&r).exists() && !p.dir(&r).exists());
        fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn network_error_and_oversize_and_cancel() {
        let r = root();
        let (p, files) = pack_of(&[("a.bin", &[1u8; 1000]), ("b.txt", b"x")]);
        let mut f = Fake { files: files.clone(), fail: Some("a.bin".into()), requested: vec![] };
        let err = p.install(&mut f, &r, &mut |_| {}, &AtomicBool::new(false)).unwrap_err();
        assert_eq!(err, InstallError::Network("mất mạng".into()));
        assert_eq!(err.user_message(), "Không tải được. Kiểm tra mạng rồi thử lại.");
        assert!(!p.tmp_dir(&r).exists());

        let mut big = files.clone();
        big.get_mut("https://example.test/x/a.bin").unwrap().push(0);
        let mut f = Fake { files: big, fail: None, requested: vec![] };
        let err = p.install(&mut f, &r, &mut |_| {}, &AtomicBool::new(false)).unwrap_err();
        assert!(matches!(err, InstallError::Corrupt(ref m) if m.contains("lớn hơn")), "{err:?}");

        let mut f = Fake { files, fail: None, requested: vec![] };
        let err = p.install(&mut f, &r, &mut |_| {}, &AtomicBool::new(true)).unwrap_err();
        assert_eq!(err, InstallError::Cancelled);
        assert!(!p.tmp_dir(&r).exists());
        fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn cleanup_keeps_current_packs_only() {
        let r = root();
        for d in [data().dir.as_str(), engine().dir.as_str(), "vi-0", "helper-0a88a61f56d56b7b", "engine-v0.tmp"] {
            fs::create_dir_all(r.join(d)).unwrap();
        }
        cleanup_old(&r);
        let mut left: Vec<String> =
            fs::read_dir(&r).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        let mut want = vec![data().dir.clone(), engine().dir.clone()];
        want.sort();
        assert_eq!(left, want);
        fs::remove_dir_all(r).unwrap();
    }

    /// Dữ liệu thật đã tải sẵn (nếu có): `CHUP_TESSDATA=<thư mục>`. Kiểm SHA-256 trong manifest khớp file thật.
    #[test]
    fn real_data_matches_manifest() {
        let Some(d) = std::env::var_os("CHUP_TESSDATA") else {
            eprintln!("bỏ qua: chưa đặt CHUP_TESSDATA");
            return;
        };
        for f in &data().files {
            let b = fs::read(PathBuf::from(&d).join(&f.name)).unwrap();
            assert_eq!((b.len() as u64, hex(&Sha256::digest(&b))), (f.size, f.sha256.clone()), "{}", f.name);
        }
    }
}
