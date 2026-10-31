//! Windows: nhúng manifest (DPI Per-Monitor v2, Common Controls v6) và icon của file exe.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=chup.rc");
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
