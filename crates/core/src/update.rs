//! Tự cập nhật (SPEC 18, chup-22n). Nguồn: GitHub Release mới nhất của repo Chup (public), qua API
//! `releases/latest` (bỏ qua draft và pre-release). API trả SHA-256 của từng file (`digest`), nên Chup tải bộ cài
//! bằng `ocr::pack::Pack` như gói OCR: kiểm tra kích thước và SHA-256 rồi mới dùng. File nào thiếu `digest` thì không
//! tự cài, chỉ mở trang Release.
//!
//! Tên file trong Release (CI đặt cho Windows, máy dev tải lên cho macOS):
//! - `Chup_<v>_x64-setup.exe` (Full), `Chup_<v>_x64-setup-lite.exe` (Lite);
//! - `Chup-<v>.dmg` (macOS, ký bằng chứng chỉ của máy dev).

use std::{fmt, path::Path, path::PathBuf};

use serde::Deserialize;

use crate::ocr::pack::{Pack, PackFile};

/// API GitHub: Release mới nhất đã phát hành.
pub const LATEST_API: &str = "https://api.github.com/repos/duccuong2808/chup/releases/latest";
/// Chỉ nhận file tải từ Release của đúng repo này.
const DOWNLOAD_PREFIX: &str = "https://github.com/duccuong2808/chup/releases/download/";
/// Tự kiểm tra mỗi ngày một lần.
pub const CHECK_EVERY_SECS: u64 = 24 * 3600;
/// Lần kiểm tra đầu sau khi khởi động (để khởi động nhẹ), rồi cứ chừng này kiểm tra xem đã tới hạn chưa.
pub const TICK_MS: u32 = 10 * 60 * 1000;
pub const FIRST_TICK_MS: u32 = 30 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// "0.3.0", "v0.3.0". Hậu tố pre-release ("-beta.1") không nhận.
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches('v');
        let mut it = s.split('.').map(|p| p.parse::<u32>().ok());
        let v = Version(it.next()??, it.next()??, it.next()??);
        it.next().is_none().then_some(v)
    }

    /// Phiên bản đang chạy.
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Bộ cài hợp với bản đang chạy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Package {
    /// Windows, đang có `chup-ocr.exe` cạnh `chup.exe` (cài bằng bộ Full).
    WindowsFull,
    WindowsLite,
    MacDmg,
}

impl Package {
    pub fn file_name(self, v: Version) -> String {
        match self {
            Package::WindowsFull => format!("Chup_{v}_x64-setup.exe"),
            Package::WindowsLite => format!("Chup_{v}_x64-setup-lite.exe"),
            Package::MacDmg => format!("Chup-{v}.dmg"),
        }
    }
}

/// Bản mới tìm thấy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub version: Version,
    /// Trang Release (mở khi không tự cài được).
    pub page: String,
    /// Bộ cài cho máy này (`None`: Release không có file hợp lệ, chỉ mở trang).
    pub pack: Option<Pack>,
}

impl Update {
    pub fn is_newer(&self) -> bool {
        self.version > Version::current()
    }

    /// Đường dẫn bộ cài sau khi tải vào `root`.
    pub fn file(&self, root: &Path) -> Option<PathBuf> {
        let p = self.pack.as_ref()?;
        Some(p.dir(root).join(&p.files.first()?.name))
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
    browser_download_url: String,
}

/// Đọc trả lời của `LATEST_API`, chọn bộ cài `pkg`.
pub fn parse_latest(json: &str, pkg: Package) -> Result<Update, String> {
    let r: GhRelease = serde_json::from_str(json).map_err(|e| format!("JSON Release: {e}"))?;
    if r.draft || r.prerelease {
        return Err(format!("Release {} là draft / pre-release", r.tag_name));
    }
    let version = Version::parse(&r.tag_name).ok_or_else(|| format!("tag không phải phiên bản: {}", r.tag_name))?;
    let want = pkg.file_name(version);
    let pack = r.assets.iter().find(|a| a.name == want).and_then(|a| {
        let sha256 = a.digest.as_deref()?.strip_prefix("sha256:")?.to_ascii_lowercase();
        let base_url = a.browser_download_url.strip_suffix(&a.name)?;
        let ok = base_url.starts_with(DOWNLOAD_PREFIX) && sha256.len() == 64 && a.size > 0;
        ok.then(|| Pack {
            dir: format!("chup-{version}"),
            source: r.html_url.clone(),
            base_url: base_url.to_string(),
            files: vec![PackFile { name: a.name.clone(), size: a.size, sha256 }],
        })
    });
    Ok(Update { version, page: r.html_url, pack })
}

/// Tới hạn tự kiểm tra chưa (`last`, `now`: giây Unix). Đồng hồ máy lùi thì kiểm tra luôn.
pub fn check_due(last: u64, now: u64) -> bool {
    now < last || now - last >= CHECK_EVERY_SECS
}

/// Dọn bộ cài đã tải (sau khi cập nhật xong, hoặc trước lần tải mới).
pub fn cleanup(root: &Path) {
    if root.exists() {
        if let Err(e) = std::fs::remove_dir_all(root) {
            log::debug!("update: không dọn được {}: {e}", root.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "1e424ab550ee3d92fc0d640b29b7771d9bf2bca48429ae5b32428292ec37b39c";

    fn release(tag: &str, extra: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/duccuong2808/chup/releases/tag/{tag}",
            "draft":false,"prerelease":false{extra},
            "assets":[
              {{"name":"Chup_9.1.0_x64-setup-lite.exe","size":744466,"digest":"sha256:{SHA}",
                "browser_download_url":"https://github.com/duccuong2808/chup/releases/download/{tag}/Chup_9.1.0_x64-setup-lite.exe"}},
              {{"name":"Chup_9.1.0_x64-setup.exe","size":5176108,
                "browser_download_url":"https://github.com/duccuong2808/chup/releases/download/{tag}/Chup_9.1.0_x64-setup.exe"}},
              {{"name":"Chup-9.1.0.dmg","size":100,"digest":"sha256:{SHA}",
                "browser_download_url":"https://evil.example/Chup-9.1.0.dmg"}}
            ]}}"#
        )
    }

    #[test]
    fn version_parse_and_order() {
        assert_eq!(Version::parse("v0.10.2"), Some(Version(0, 10, 2)));
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::parse("1.2.3-beta"), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert_eq!(Version(1, 2, 3).to_string(), "1.2.3");
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn picks_package_with_digest_from_this_repo() {
        let u = parse_latest(&release("v9.1.0", ""), Package::WindowsLite).unwrap();
        assert_eq!(u.version, Version(9, 1, 0));
        assert!(u.is_newer());
        let p = u.pack.as_ref().unwrap();
        assert_eq!(p.base_url, "https://github.com/duccuong2808/chup/releases/download/v9.1.0/");
        assert_eq!(p.files[0].sha256, SHA);
        assert_eq!(p.files[0].size, 744466);
        assert_eq!(u.file(Path::new("/c")), Some(PathBuf::from("/c/chup-9.1.0/Chup_9.1.0_x64-setup-lite.exe")));
        // Thiếu digest: không tự cài, chỉ mở trang.
        assert_eq!(parse_latest(&release("v9.1.0", ""), Package::WindowsFull).unwrap().pack, None);
        // Link ngoài repo: không nhận.
        assert_eq!(parse_latest(&release("v9.1.0", ""), Package::MacDmg).unwrap().pack, None);
    }

    #[test]
    fn rejects_bad_release() {
        assert!(parse_latest(&release("nightly", ""), Package::WindowsLite).is_err());
        assert!(parse_latest(&release("v9.1.0", ",\"x\":1").replace("\"prerelease\":false", "\"prerelease\":true"), Package::WindowsLite).is_err());
        assert!(parse_latest("{", Package::WindowsLite).is_err());
        let old = parse_latest(&release("v0.0.1", ""), Package::WindowsLite).unwrap();
        assert!(!old.is_newer());
    }

    #[test]
    fn check_every_day() {
        assert!(check_due(0, 1_000_000));
        assert!(!check_due(1_000_000, 1_000_000 + 3600));
        assert!(check_due(1_000_000, 1_000_000 + CHECK_EVERY_SECS));
        assert!(check_due(1_000_000, 10), "đồng hồ lùi");
    }
}
