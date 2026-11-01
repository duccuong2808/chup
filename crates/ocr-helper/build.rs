//! Link Tesseract: Windows lấy từ vcpkg (tĩnh, CRT tĩnh như chup.exe); OS khác dùng pkg-config (chỉ để dev/test).

fn main() {
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // crate vcpkg tự chọn triplet `x64-windows-static` khi bật crt-static (.cargo/config.toml)
        // và link cả các thư viện phụ thuộc (leptonica, libpng, zlib...) theo metadata của vcpkg.
        if let Err(e) = vcpkg::Config::new().emit_includes(false).find_package("tesseract") {
            panic!("Không tìm thấy Tesseract trong vcpkg (đặt VCPKG_ROOT, cài tesseract:x64-windows-static): {e}");
        }
        // Thư viện hệ thống mà các bản static của vcpkg cần (curl/schannel, libarchive...). Thừa cũng không sao:
        // linker chỉ lấy import được dùng tới.
        for lib in ["user32", "advapi32", "ws2_32", "crypt32", "secur32", "bcrypt", "normaliz", "iphlpapi", "wldap32"] {
            println!("cargo:rustc-link-lib={lib}");
        }
        // Code page UTF-8 cho các API "A"/fopen của CRT (phòng đường dẫn có dấu; chup.exe cũng truyền "." làm tessdata).
        use embed_manifest::manifest::{ActiveCodePage, Setting};
        let m = embed_manifest::new_manifest("Chup.Ocr").active_code_page(ActiveCodePage::Utf8).long_path_aware(Setting::Enabled);
        embed_manifest::embed_manifest(m).expect("không nhúng được manifest");
    } else if let Err(e) = pkg_config::Config::new().atleast_version("5.0").probe("tesseract") {
        panic!("Không tìm thấy Tesseract qua pkg-config (macOS: brew install tesseract pkg-config): {e}");
    }
}
