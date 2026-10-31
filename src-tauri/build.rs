// Khai báo command của app để Tauri sinh permission `allow-<command>`;
// từng cửa sổ chỉ được cấp đúng command nó cần (xem capabilities/).
const COMMANDS: &[&str] = &[
    "region_selected",
    "selection_cancelled",
    "frame_state",
    "capture_now",
    "toggle_fade",
    "close_region",
    "stack_state",
    "stack_report_size",
    "stack_remove",
    "stack_clear",
    "shot_exists",
    "shot_open",
    "shot_reveal",
    "shot_copy",
    "settings_get",
    "settings_update",
    "pick_save_dir",
    "open_save_dir",
    "hotkeys_pause",
    "hotkeys_resume",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri build failed");
}
