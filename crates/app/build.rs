//! Windows: nhúng manifest (DPI Per-Monitor v2, Common Controls v6) và icon của file exe.
//! Có `CHUP_OCR_DIR` (thư mục chứa `chup-ocr.exe` + `NOTICE-ocr.txt`, CI đặt) thì nhúng luôn engine OCR tiếng Việt
//! (xem `chup_core::ocr::helper`). Không đặt thì `chup.exe` chỉ dùng được `chup-ocr.exe` nằm cạnh nó.

use std::{fmt::Write as _, path::PathBuf};

use sha2::{Digest, Sha256};

/// Sinh `$OUT_DIR/ocr_embed.rs`: hằng `EMBEDDED: Option<Embedded>`.
fn embed_ocr() {
    println!("cargo:rerun-if-env-changed=CHUP_OCR_DIR");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("ocr_embed.rs");
    let code = match std::env::var_os("CHUP_OCR_DIR").filter(|d| !d.is_empty()) {
        None => "pub const EMBEDDED: Option<chup_core::ocr::helper::Embedded> = None;\n".to_string(),
        Some(dir) => {
            let dir = std::path::absolute(PathBuf::from(&dir))
                .unwrap_or_else(|e| panic!("CHUP_OCR_DIR={}: {e}", dir.to_string_lossy()));
            let exe = dir.join("chup-ocr.exe");
            let notice = dir.join("NOTICE-ocr.txt");
            for f in [&exe, &notice] {
                println!("cargo:rerun-if-changed={}", f.display());
            }
            let bytes = std::fs::read(&exe).unwrap_or_else(|e| panic!("đọc {}: {e}", exe.display()));
            assert!(notice.is_file(), "thiếu {}", notice.display());
            let mut sha = String::new();
            for b in Sha256::digest(&bytes) {
                let _ = write!(sha, "{b:02x}");
            }
            format!(
                "pub const EMBEDDED: Option<chup_core::ocr::helper::Embedded> = Some(chup_core::ocr::helper::Embedded {{\n    \
                 exe: include_bytes!({:?}),\n    notice: include_bytes!({:?}),\n    sha256: {sha:?},\n}});\n",
                exe.to_string_lossy(),
                notice.to_string_lossy(),
            )
        }
    };
    std::fs::write(&out, code).expect("ghi ocr_embed.rs");
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=chup.rc");
    embed_ocr();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // embed-manifest tự sinh object file, không cần rc.exe: check được cả khi cross-compile từ macOS.
    use embed_manifest::manifest::{ActiveCodePage, DpiAwareness, Setting, SupportedOS::*};
    let manifest = embed_manifest::new_manifest("Chup")
        .dpi_awareness(DpiAwareness::PerMonitorV2)
        .active_code_page(ActiveCodePage::Utf8)
        .long_path_aware(Setting::Enabled)
        .supported_os(Windows10..=Windows10);
    embed_manifest::embed_manifest(manifest).expect("không nhúng được manifest");

    // Icon của exe cần resource compiler (rc.exe trên Windows). Máy khác (cross-check) thì bỏ qua.
    if cfg!(windows) {
        let _ = embed_resource::compile("chup.rc", embed_resource::NONE);
    }
}
