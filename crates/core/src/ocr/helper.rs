//! Engine OCR tiếng Việt `chup-ocr.exe` nhúng sẵn trong `chup.exe` (CI build, xem `crates/app/build.rs`).
//!
//! Bộ cài vẫn đặt `chup-ocr.exe` cạnh `chup.exe`; bản nhúng dùng khi chạy `chup.exe` rời (không qua bộ cài)
//! hoặc khi exe phụ cạnh nó là bản cũ (khác PROTO). Repo Chup private nên không có chỗ công khai để tải exe:
//! nhúng thì khỏi cần mạng và hai exe luôn khớp phiên bản.
//!
//! Giải nén: ghi vào `<ocr>/helper-<hash>.tmp/`, đọc lại kiểm SHA-256, rồi rename thành `<ocr>/helper-<hash>/`.
//! Tên thư mục theo hash nên mỗi bản `chup.exe` có engine của riêng nó; bản cũ bị `pack::cleanup_old` dọn.

use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

pub const EXE_NAME: &str = "chup-ocr.exe";
/// License của Tesseract và các thư viện link tĩnh, luôn đi cùng exe.
pub const NOTICE_NAME: &str = "NOTICE-ocr.txt";

/// Exe phụ nhúng trong `chup.exe`. `sha256` tính lúc build.
#[derive(Clone, Copy, Debug)]
pub struct Embedded {
    pub exe: &'static [u8],
    pub notice: &'static [u8],
    pub sha256: &'static str,
}

impl Embedded {
    /// Tên thư mục giải nén, vd. `helper-79df64caf7bcfb2a`.
    pub fn dir_name(&self) -> String {
        format!("helper-{}", &self.sha256[..16.min(self.sha256.len())])
    }

    pub fn exe_path(&self, ocr_root: &Path) -> PathBuf {
        ocr_root.join(self.dir_name()).join(EXE_NAME)
    }

    /// Đã giải nén đủ (kiểm kích thước, rẻ). SHA-256 chỉ kiểm lúc giải nén.
    pub fn is_extracted(&self, ocr_root: &Path) -> bool {
        let d = ocr_root.join(self.dir_name());
        let ok = |name: &str, len: usize| fs::metadata(d.join(name)).is_ok_and(|m| m.is_file() && m.len() == len as u64);
        ok(EXE_NAME, self.exe.len()) && ok(NOTICE_NAME, self.notice.len())
    }

    /// Giải nén nếu chưa có, trả đường dẫn exe. Lỗi (chuỗi kỹ thuật) chỉ để ghi log.
    pub fn ensure(&self, ocr_root: &Path) -> Result<PathBuf, String> {
        let exe = self.exe_path(ocr_root);
        if self.is_extracted(ocr_root) {
            return Ok(exe);
        }
        let dst = ocr_root.join(self.dir_name());
        let tmp = ocr_root.join(format!("{}.tmp", self.dir_name()));
        let r = self.extract_into(&tmp).and_then(|()| {
            if dst.exists() {
                fs::remove_dir_all(&dst).map_err(|e| format!("xoá {}: {e}", dst.display()))?;
            }
            fs::rename(&tmp, &dst).map_err(|e| format!("rename {}: {e}", tmp.display()))
        });
        if r.is_err() {
            let _ = fs::remove_dir_all(&tmp);
        }
        r.map(|()| exe)
    }

    fn extract_into(&self, tmp: &Path) -> Result<(), String> {
        let io = |what: &str, e: std::io::Error| format!("{what} {}: {e}", tmp.display());
        if tmp.exists() {
            fs::remove_dir_all(tmp).map_err(|e| io("xoá", e))?;
        }
        fs::create_dir_all(tmp).map_err(|e| io("tạo", e))?;
        fs::write(tmp.join(NOTICE_NAME), self.notice).map_err(|e| io("ghi NOTICE vào", e))?;
        let exe = tmp.join(EXE_NAME);
        fs::write(&exe, self.exe).map_err(|e| io("ghi exe vào", e))?;
        // Đọc lại từ đĩa: phần mềm diệt virus có thể chặn hoặc cắt file vừa ghi.
        let back = fs::read(&exe).map_err(|e| io("đọc lại exe trong", e))?;
        let got = hex(&Sha256::digest(&back));
        if got != self.sha256 {
            return Err(format!("{EXE_NAME} giải nén sai SHA-256: {got}"));
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 dạng hex (build.rs của app tính giống hệt để ghi vào `Embedded::sha256`).
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &[u8] = b"MZ fake chup-ocr";
    const NOTICE: &[u8] = b"license";

    fn root() -> PathBuf {
        let d = std::env::temp_dir().join(format!("chup-helper-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn embedded(sha: &'static str) -> Embedded {
        Embedded { exe: EXE, notice: NOTICE, sha256: sha }
    }

    fn real_sha() -> &'static str {
        Box::leak(sha256_hex(EXE).into_boxed_str())
    }

    #[test]
    fn extracts_once_and_reextracts_when_damaged() {
        let r = root();
        let e = embedded(real_sha());
        assert!(!e.is_extracted(&r));
        let p = e.ensure(&r).unwrap();
        assert_eq!(p, r.join(e.dir_name()).join(EXE_NAME));
        assert_eq!(fs::read(&p).unwrap(), EXE);
        assert_eq!(fs::read(p.with_file_name(NOTICE_NAME)).unwrap(), NOTICE);
        assert!(e.is_extracted(&r));
        assert!(!r.join(format!("{}.tmp", e.dir_name())).exists());
        assert_eq!(e.dir_name().len(), "helper-".len() + 16);

        // Lần sau không ghi lại.
        let before = fs::metadata(&p).unwrap().modified().unwrap();
        e.ensure(&r).unwrap();
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), before);

        // File bị cắt (diệt virus, đĩa đầy lần trước): giải nén lại.
        fs::write(&p, b"MZ").unwrap();
        assert!(!e.is_extracted(&r));
        e.ensure(&r).unwrap();
        assert_eq!(fs::read(&p).unwrap(), EXE);
        fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn wrong_hash_leaves_nothing_behind() {
        let r = root();
        let e = embedded("0000000000000000000000000000000000000000000000000000000000000000");
        let err = e.ensure(&r).unwrap_err();
        assert!(err.contains("SHA-256"), "{err}");
        assert_eq!(fs::read_dir(&r).unwrap().count(), 0);
        fs::remove_dir_all(r).unwrap();
    }
}
