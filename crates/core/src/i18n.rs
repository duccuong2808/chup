//! Chuỗi giao diện theo ngôn ngữ (chup-81y). Mỗi chuỗi khai báo một lần, các bản dịch đứng cạnh nhau trong
//! [`strings!`], nên thiếu bản dịch là lỗi biên dịch.
//!
//! Ngôn ngữ hiện hành là global: lớp platform chọn (theo config `uiLanguage` và ngôn ngữ của hệ điều hành) rồi gọi
//! [`set`] trước khi dựng giao diện. Mặc định tiếng Việt. Test không bao giờ gọi `set` (test chạy song song).
//!
//! Chuỗi có tham số dùng `{}`, điền bằng [`fill`]. Không dịch: log, comment, tên phím tắt, mẫu ngày giờ.

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Vi,
    En,
}

/// Lựa chọn trong config. `System` = theo ngôn ngữ giao diện của hệ điều hành.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiLanguage {
    #[default]
    System,
    Vi,
    En,
}

impl UiLanguage {
    /// `system_vi`: hệ điều hành đang dùng tiếng Việt. Ngôn ngữ khác tiếng Việt thì dùng tiếng Anh.
    pub fn resolve(self, system_vi: bool) -> Lang {
        match self {
            UiLanguage::Vi => Lang::Vi,
            UiLanguage::En => Lang::En,
            UiLanguage::System if system_vi => Lang::Vi,
            UiLanguage::System => Lang::En,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::En as u8 {
        Lang::En
    } else {
        Lang::Vi
    }
}

/// Bảng chuỗi của ngôn ngữ hiện hành.
pub fn t() -> &'static Strings {
    of(current())
}

pub fn of(lang: Lang) -> &'static Strings {
    match lang {
        Lang::Vi => &VI,
        Lang::En => &EN,
    }
}

/// Thay lần lượt từng `{}` trong `template` bằng `args`.
pub fn fill(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut rest = template;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        match args.next() {
            Some(a) => out.push_str(&a.to_string()),
            None => out.push_str("{}"),
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// Số thập phân một chữ số theo ngôn ngữ ("4,4" / "4.4").
pub fn decimal1(v: f64) -> String {
    let s = format!("{v:.1}");
    if current() == Lang::Vi {
        s.replace('.', ",")
    } else {
        s
    }
}

macro_rules! strings {
    ($($name:ident: $vi:expr, $en:expr;)*) => {
        /// Mọi chuỗi giao diện của một ngôn ngữ.
        pub struct Strings {
            $(pub $name: &'static str,)*
        }
        pub static VI: Strings = Strings { $($name: $vi,)* };
        pub static EN: Strings = Strings { $($name: $en,)* };
        #[cfg(test)]
        const ALL: &[(&str, fn(&Strings) -> &'static str)] = &[$((stringify!($name), |s| s.$name),)*];
    };
}

strings! {
    // ---- Chung ----
    app_settings_title: "Chup — Cài đặt", "Chup — Settings";
    unused_hotkey: "Không dùng", "None";

    // ---- Toast của Model ----
    monitor_gone_choose_again: "Màn hình chứa vùng chụp không còn. Hãy chọn vùng mới.",
        "The monitor with the capture region is gone. Select a new region.";
    capture_failed: "Không chụp được màn hình", "Couldn't capture the screen";
    image_copied_clipboard: "Đã copy ảnh vào clipboard", "Image copied to clipboard";
    image_copied: "Đã copy ảnh", "Image copied";
    no_text_found: "Không tìm thấy chữ", "No text found";
    file_missing_removed: "File {} không còn, đã bỏ khỏi stack", "File {} no longer exists and was removed from the stack";
    monitor_unplugged_closed: "Màn hình chứa vùng chọn đã bị rút, vùng chọn đã đóng.",
        "The monitor with the selection was disconnected, so the selection was closed.";
    copied_lines_one: "Đã copy {} dòng: “{}”", "Copied {} line: “{}”";
    copied_lines_many: "Đã copy {} dòng: “{}”", "Copied {} lines: “{}”";

    // ---- Chụp, lưu, clipboard ----
    err_no_monitor_for_region: "Không tìm thấy màn hình của vùng chụp: {}", "Couldn't find the monitor of the region: {}";
    err_region_off_screen: "Vùng chụp nằm ngoài màn hình", "The region is outside the screen";
    err_capture_failed: "Chụp màn hình thất bại: {}", "Screen capture failed: {}";
    err_list_monitors: "Không liệt kê được màn hình: {}", "Couldn't list monitors: {}";
    err_capture_thread: "Thread chụp bị panic", "Capture thread panicked";
    err_thumbnail: "Tạo thumbnail bị lỗi", "Couldn't create the thumbnail";
    err_thumbnail_detail: "Tạo thumbnail bị lỗi: {}", "Couldn't create the thumbnail: {}";
    warn_clipboard: "Không copy được vào clipboard", "Couldn't copy to the clipboard";
    warn_clipboard_detail: "Không copy được vào clipboard: {}", "Couldn't copy to the clipboard: {}";
    err_save_to: "Không lưu được ảnh vào {}: {}", "Couldn't save the image to {}: {}";
    err_save_both: "Không lưu được ảnh vào {} ({}) lẫn {} ({})", "Couldn't save the image to {} ({}) or {} ({})";
    warn_saved_fallback: "Không ghi được vào {} ({}). Ảnh đã lưu tại {}", "Couldn't write to {} ({}). The image was saved to {}";
    err_too_many_same_name: "Quá nhiều file trùng tên", "Too many files with the same name";
    err_read_image: "Không đọc được ảnh: {}", "Couldn't read the image: {}";
    err_out_of_memory: "Thiếu bộ nhớ: {}", "Out of memory: {}";
    err_lock_memory: "Không khoá được bộ nhớ", "Couldn't lock memory";
    err_clipboard_busy: "Clipboard đang bị app khác giữ", "The clipboard is in use by another app";
    err_clipboard_empty: "Không xoá được clipboard: {}", "Couldn't clear the clipboard: {}";
    err_clipboard_text: "Không đặt được text vào clipboard: {}", "Couldn't put text on the clipboard: {}";
    err_clipboard_image: "Không đặt được ảnh vào clipboard: {}", "Couldn't put the image on the clipboard: {}";

    // ---- Config ----
    config_corrupt: "File cấu hình bị lỗi ({}). Đã lưu bản cũ thành {} và dùng cấu hình mặc định.",
        "The settings file is damaged ({}). The old file was saved as {} and default settings are used.";
    err_config_dir: "Không tạo được thư mục cấu hình: {}", "Couldn't create the settings folder: {}";
    err_config_write: "Không ghi được cấu hình: {}", "Couldn't save settings: {}";
    err_config_value: "Giá trị không hợp lệ: {}", "Invalid value: {}";
    err_pattern_empty: "Pattern tên file không được để trống", "The file name pattern can't be empty";
    err_hotkeys_duplicate: "Hai phím tắt đang trùng nhau", "Two shortcuts are the same";
    hotkey_need_modifier: "Cần có Ctrl, Alt hoặc dùng phím F1–F24", "Use Ctrl or Alt, or a key from F1–F24";

    // ---- Tự động chọn vùng: tên khung ----
    kind_window: "Cửa sổ", "Window";
    kind_button: "Nút", "Button";
    kind_edit: "Ô nhập", "Text box";
    kind_text: "Chữ", "Text";
    kind_image: "Ảnh", "Image";
    kind_link: "Liên kết", "Link";
    kind_list: "Danh sách", "List";
    kind_list_item: "Mục", "Item";
    kind_menu: "Menu", "Menu";
    kind_menu_item: "Mục menu", "Menu item";
    kind_tab: "Tab", "Tab";
    kind_toolbar: "Thanh công cụ", "Toolbar";
    kind_title_bar: "Thanh tiêu đề", "Title bar";
    kind_document: "Tài liệu", "Document";
    kind_table: "Bảng", "Table";
    kind_group: "Nhóm", "Group";
    kind_pane: "Vùng", "Area";
    kind_detected: "Vùng ảnh", "Image area";

    // ---- OCR ----
    ocr_remove_failed: "Không xoá được OCR tiếng Việt, hãy thử lại sau.", "Couldn't remove Vietnamese OCR. Try again later.";
    dl_network: "Không tải được. Kiểm tra mạng rồi thử lại.", "Download failed. Check your connection and try again.";
    dl_corrupt: "File tải về bị lỗi. Hãy thử lại.", "The downloaded file is damaged. Try again.";
    dl_io: "Không ghi được vào máy. Kiểm tra dung lượng ổ đĩa rồi thử lại.",
        "Couldn't write to disk. Check free space and try again.";
    dl_cancelled: "Đã huỷ tải.", "Download cancelled.";
    dl_busy: "Lần tải trước đang dừng. Hãy thử lại sau vài giây.", "The previous download is still stopping. Try again in a few seconds.";
    ocr_removed: "Đã xoá OCR tiếng Việt", "Vietnamese OCR removed";
    ocr_installed: "Đã tải OCR tiếng Việt", "Vietnamese OCR downloaded";
    ocr_no_windows_en: "Windows chưa có OCR tiếng Anh. Hãy chọn \"Tiếng Việt + Anh\" trong Cài đặt.",
        "Windows has no English OCR. Choose \"Vietnamese + English\" in Settings.";
    ocr_failed_retry: "OCR lỗi, hãy thử lại.", "OCR failed. Try again.";
    ocr_engine_mismatch: "Engine OCR tiếng Việt không khớp phiên bản Chup. Hãy cài lại Chup.",
        "The Vietnamese OCR engine doesn't match this Chup version. Reinstall Chup.";
    ocr_engine_missing: "Thiếu engine OCR tiếng Việt. Hãy tải lại trong Cài đặt > Copy chữ (OCR).",
        "The Vietnamese OCR engine is missing. Download it again in Settings > Copy text (OCR).";
    ocr_engine_blocked: "Không chạy được engine OCR tiếng Việt. Phần mềm diệt virus có thể đã chặn chup-ocr.exe.",
        "Couldn't run the Vietnamese OCR engine. Antivirus software may have blocked chup-ocr.exe.";
    ocr_pack_broken: "Gói OCR tiếng Việt bị lỗi. Hãy xoá gói rồi tải lại trong Cài đặt.",
        "The Vietnamese OCR files are damaged. Remove and download them again in Settings.";
    ocr_read_image_failed: "Không đọc được ảnh để OCR", "Couldn't read the image for OCR";
    ocr_running: "Đang nhận diện chữ…", "Recognizing text…";
    upd_title: "Cập nhật Chup", "Update Chup";
    upd_ask: "Đã có Chup {} (đang dùng {}). Cập nhật bây giờ?\n\nChup sẽ tự đóng, cài bản mới rồi mở lại.",
        "Chup {} is available (you have {}). Update now?\n\nChup will close, install the new version and reopen.";
    upd_ask_page: "Đã có Chup {} (đang dùng {}). Mở trang tải về?", "Chup {} is available (you have {}). Open the download page?";
    upd_now: "Cập nhật", "Update";
    upd_open_page: "Mở trang tải", "Open download page";
    upd_later: "Để sau", "Later";
    upd_latest: "Chup {} là bản mới nhất.", "Chup {} is up to date.";
    upd_check_failed: "Không kiểm tra được bản mới. Hãy thử lại sau.", "Couldn't check for updates. Try again later.";
    upd_checking: "Đang kiểm tra bản mới…", "Checking for updates…";
    upd_downloading: "Đang tải Chup {}…", "Downloading Chup {}…";
    upd_failed: "Không cập nhật được Chup. Hãy tải bản mới từ trang dự án.", "Couldn't update Chup. Download the new version from the project page.";
    ocr_running_slow: "Đang nhận diện chữ… Lần đầu có thể mất khoảng 1 phút", "Recognizing text… The first time can take about a minute";
    dl_title: "Chup – OCR tiếng Việt", "Chup – Vietnamese OCR";
    dl_confirm: "OCR tiếng Việt cần tải thêm {} từ GitHub.\n\nTải về bây giờ?",
        "Vietnamese OCR needs to download {} from GitHub.\n\nDownload now?";
    dl_progress: "Đã tải {} / {} MB", "Downloaded {} / {} MB";
    dl_heading: "Đang tải OCR tiếng Việt", "Downloading Vietnamese OCR";
    dl_connecting: "Đang kết nối…", "Connecting…";

    // ---- Các lỗi khác của app ----
    err_create_dir: "Không tạo được {}: {}", "Couldn't create {}: {}";
    err_open_image: "Không mở được ảnh: {}", "Couldn't open the image: {}";
    err_freeze_memory: "Không đủ bộ nhớ để hiện ảnh đóng băng", "Not enough memory to show the frozen screen";
    err_selection_outside: "Vùng chọn nằm ngoài ảnh", "The selection is outside the image";
    err_preview_memory: "Không đủ bộ nhớ để mở preview", "Not enough memory to open the preview";
    err_drag: "Không kéo được ảnh: {}", "Couldn't drag the image: {}";
    err_open_path: "Không mở được {}", "Couldn't open {}";
    err_not_found: "Không tìm thấy {}", "Couldn't find {}";
    err_open_explorer: "Không mở được Explorer: {}", "Couldn't open Explorer: {}";
    err_run_key: "Không mở được khoá Run: {}", "Couldn't open the Run registry key: {}";
    err_autostart: "Không đổi được chế độ khởi động cùng Windows: {}", "Couldn't change start with Windows: {}";

    // ---- macOS (SPEC 11) ----
    mac_err_autostart: "Không đổi được chế độ mở cùng macOS: {}", "Couldn't change open at login: {}";
    mac_err_autostart_bundle: "Chỉ bật được khi chạy Chup.app (bản đóng gói)",
        "Only available when running Chup.app (the packaged app)";
    mac_perm_title: "Chup cần quyền ghi màn hình", "Chup needs Screen Recording permission";
    mac_perm_body: "Mở Cài đặt hệ thống > Quyền riêng tư & Bảo mật > Ghi màn hình & âm thanh hệ thống, bật Chup. Nếu macOS yêu cầu thì thoát và mở lại Chup.",
        "Open System Settings > Privacy & Security > Screen & System Audio Recording and turn on Chup. Quit and reopen Chup if macOS asks you to.";
    mac_perm_open: "Mở Cài đặt hệ thống", "Open System Settings";
    mac_perm_later: "Để sau", "Later";
    mac_err_capture: "Không chụp được màn hình: {}", "Couldn't capture the screen: {}";
    mac_st_autostart: "Mở cùng macOS (khi đăng nhập)", "Open at login";
    mac_st_hide_ui_note: "Thường không cần bật: macOS đã loại cửa sổ của Chup khỏi ảnh chụp.",
        "Usually not needed: macOS already keeps Chup's windows out of the screenshot.";
    mac_st_ocr_en: "Tiếng Anh", "English";
    mac_st_ocr_note: "Dùng Apple Vision có sẵn trong macOS, không cần tải thêm.",
        "Uses Apple Vision built into macOS, nothing to download.";
    mac_st_about_desc: "Chụp màn hình theo vùng cho macOS: stack ảnh kéo thả, chụp đóng băng, Preview, copy chữ từ ảnh.",
        "Region screenshots for macOS: a drag-and-drop stack, freeze capture, Preview, copy text from images.";
    mac_st_hotkeys_hint: "Bấm vào ô rồi nhấn tổ hợp phím (cần ⌃, ⌥ hoặc ⌘; F1–F20 dùng một mình được). Delete để tắt, Esc để huỷ.",
        "Click a box and press the keys (needs ⌃, ⌥ or ⌘; F1–F20 work alone). Delete turns it off, Esc cancels.";
    mac_st_clear: "Xoá", "Clear";
    mac_st_ui_language_system: "Theo macOS", "Same as macOS";
    mac_st_auto_img_note: "Trên macOS, khung từ hình ảnh chỉ dùng khi chụp đóng băng.",
        "On macOS, frames from the image are only used in freeze capture.";
    mac_st_ax_ok: "Quyền Trợ năng: đã có (nhận được nút, ô nhập, đoạn chữ trong cửa sổ).",
        "Accessibility permission: granted (buttons, fields and text inside windows are detected).";
    mac_st_ax_missing: "Quyền Trợ năng: chưa có. Chỉ nhận được khung cửa sổ.",
        "Accessibility permission: not granted. Only whole windows are detected.";
    mac_st_ax_grant: "Cấp quyền Trợ năng…", "Grant Accessibility…";
    err_hotkey_init: "Không khởi tạo được phím tắt: {}", "Couldn't set up shortcuts: {}";
    err_tray: "Không tạo được icon khay: {}", "Couldn't create the tray icon: {}";
    hotkey_taken: "Không dùng được {} vì app khác (hoặc hệ thống) đang dùng tổ hợp này. Hãy chọn tổ hợp khác.",
        "Can't use {} because another app (or the system) uses it. Choose a different shortcut.";
    hotkey_invalid: "Phím tắt \"{}\" không hợp lệ. Hãy bấm vào ô và nhấn lại tổ hợp phím.",
        "Shortcut \"{}\" is invalid. Click the box and press the keys again.";
    hotkey_taken_recording: "{} đang được app khác (hoặc hệ thống) dùng nên Chup không đăng ký được. Hãy chọn tổ hợp khác.",
        "{} is used by another app (or the system), so Chup can't register it. Choose a different shortcut.";

    // ---- Overlay, toolbar, stack, Preview ----
    hint_select: "Kéo để chọn vùng · Esc hoặc chuột phải để huỷ", "Drag to select a region · Esc or right-click to cancel";
    hint_select_auto: "Click để chọn khung đang tô · Kéo để chọn vùng · Lăn chuột đổi cỡ khung · Esc để huỷ",
        "Click to pick the highlighted frame · Drag to select · Scroll to change frame size · Esc to cancel";
    hint_frozen: "Ảnh đã đóng băng · Kéo để chọn vùng · Enter để lưu · Esc để huỷ",
        "Screen frozen · Drag to select · Enter to save · Esc to cancel";
    hint_frozen_auto: "Ảnh đã đóng băng · Click chọn khung hoặc kéo chọn vùng · Enter để lưu · Esc để huỷ",
        "Screen frozen · Click a frame or drag to select · Enter to save · Esc to cancel";
    hint_freeze_failed: "Không chụp được màn hình này · Esc để huỷ", "Couldn't capture this screen · Esc to cancel";
    hint_ocr: "Chụp text · Kéo để chọn vùng có chữ · Esc để huỷ", "Capture text · Drag over the text · Esc to cancel";
    btn_capture: "Chụp", "Capture";
    btn_capture_enter: "Chụp ↵", "Capture ↵";
    btn_fade: "Mờ", "Fade";
    btn_save: "Lưu", "Save";
    btn_text: "Chữ", "Text";
    stack_clear_all: "Xoá hết", "Clear all";
    menu_copy_text: "Copy chữ", "Copy text";
    menu_select_all_text: "Chọn tất cả chữ", "Select all text";
    menu_copy_image: "Copy ảnh", "Copy image";

    // ---- Menu tray ----
    tray_section_capture: "Chụp", "Capture";
    tray_select: "Chọn vùng", "Select region";
    tray_recapture: "Chụp lại vùng cuối", "Capture last region";
    tray_freeze: "Chụp đóng băng", "Freeze capture";
    tray_freeze_delayed: "Chụp đóng băng sau 3 giây", "Freeze capture in 3 seconds";
    tray_open_folder: "Mở folder ảnh", "Open screenshots folder";
    tray_section_options: "Tuỳ chọn", "Options";
    tray_freeze_toggle: "Phím tắt chụp đóng băng", "Freeze capture shortcut";
    tray_settings: "Cài đặt…", "Settings…";
    tray_quit: "Thoát", "Quit";
    tray_waiting: "Chup — sắp chụp đóng băng…", "Chup — freeze capture coming…";
    tray_hotkey_errors: "Chup — ⚠ {} phím tắt lỗi, xem Cài đặt", "Chup — ⚠ {} shortcut(s) failed, see Settings";

    // ---- Cài đặt ----
    page_general: "Chung", "General";
    page_hotkeys: "Phím tắt", "Shortcuts";
    page_capture: "Chụp ảnh", "Capture";
    page_ocr: "Copy chữ (OCR)", "Copy text (OCR)";
    page_stack_preview: "Stack và Preview", "Stack and Preview";
    page_about: "Giới thiệu", "About";
    st_saved: "Đã lưu", "Saved";
    st_sec_save: "Lưu ảnh", "Saving";
    st_save_dir: "Folder lưu ảnh", "Screenshots folder";
    st_pick_folder: "Chọn folder…", "Choose folder…";
    st_open_folder: "Mở folder", "Open folder";
    st_pick_folder_title: "Chọn folder lưu ảnh", "Choose the screenshots folder";
    st_file_name: "Tên file", "File name";
    st_file_name_hint: "Phần trong { } là ngày giờ: yyyy MM dd HH mm ss SSS. Ví dụ: {}",
        "The part in { } is the date and time: yyyy MM dd HH mm ss SSS. Example: {}";
    st_sec_system: "Hệ thống", "System";
    st_autostart: "Khởi động cùng Windows", "Start with Windows";
    st_ui_language: "Ngôn ngữ giao diện", "Display language";
    st_ui_language_system: "Theo Windows", "Same as Windows";
    st_hotkeys_hint: "Bấm vào ô rồi nhấn tổ hợp phím. Delete để tắt phím tắt, Esc để huỷ.",
        "Click a box and press the keys. Delete turns the shortcut off, Esc cancels.";
    st_recording: "Nhấn tổ hợp phím…", "Press the keys…";
    st_hk_select: "Chọn vùng", "Select region";
    st_hk_capture: "Chụp vùng hiện tại / vùng cuối", "Capture current / last region";
    st_hk_freeze: "Chụp đóng băng (menu, tooltip)", "Freeze capture (menus, tooltips)";
    st_hk_freeze_on: "Bật phím chụp đóng băng (tắt để app khác dùng được phím này)",
        "Use the freeze capture shortcut (turn off to let other apps use this key)";
    st_hk_ocr: "Chụp text (chọn vùng rồi copy chữ)", "Capture text (select, then copy the text)";
    st_sec_auto: "Tự động chọn vùng", "Automatic region detection";
    st_auto_on: "Rê chuột là tô sẵn khung cửa sổ, nút, ảnh…", "Highlight windows, buttons, images… under the pointer";
    st_auto_img: "Đoán khung từ hình ảnh khi app không tự báo", "Detect frames from the image when the app doesn't report them";
    st_auto_hint: "Click để chọn khung đang tô. Lăn chuột hoặc ↑↓ để đổi cỡ khung, giữ Shift để tạm tắt.",
        "Click to pick the highlighted frame. Scroll or press ↑↓ to change its size, hold Shift to pause.";
    st_sec_after: "Sau khi chụp", "After capture";
    st_copy: "Copy ảnh vào clipboard", "Copy the image to the clipboard";
    st_after_normal: "Vùng chọn ở chế độ thường", "Selection in normal mode";
    st_after_hint: "Ở chế độ Mờ, vùng chọn luôn được giữ.", "In Fade mode the selection is always kept.";
    st_after_close: "Đóng vùng chọn", "Close the selection";
    st_after_keep: "Giữ vùng chọn để chụp tiếp", "Keep it for the next capture";
    st_sec_during: "Trong lúc chụp", "While capturing";
    st_hide_ui: "Ẩn toolbar và stack trong lúc chụp", "Hide the toolbar and stack while capturing";
    st_hide_ui_note_ok: "Thường không cần bật: Windows đã loại toolbar và stack khỏi ảnh chụp. Bật nếu vẫn thấy chúng lọt vào ảnh.",
        "Usually not needed: Windows already keeps the toolbar and stack out of screenshots. Turn on if they still show up.";
    st_hide_ui_note_old: "Windows này không hỗ trợ loại cửa sổ khỏi ảnh chụp (cần Windows 10 bản 2004 trở lên), nên Chup luôn ẩn toolbar và stack trong lúc chụp.",
        "This Windows can't keep windows out of screenshots (needs Windows 10 version 2004 or later), so Chup always hides the toolbar and stack while capturing.";
    st_sec_ocr: "Nhận diện chữ trong ảnh", "Text recognition";
    st_ocr_language: "Ngôn ngữ", "Language";
    st_ocr_en: "Tiếng Anh (có sẵn trong Windows)", "English (built into Windows)";
    st_ocr_vi: "Tiếng Việt + Anh", "Vietnamese + English";
    st_ocr_ready: "OCR tiếng Việt: đã sẵn sàng.", "Vietnamese OCR: ready.";
    st_ocr_remove: "Xoá bản đã tải", "Remove download";
    st_ocr_missing_all: "OCR tiếng Việt: chưa tải. Tải từ GitHub: engine (repo Chup) và dữ liệu (tesseract-ocr/tessdata_fast).",
        "Vietnamese OCR: not downloaded. Downloads from GitHub: the engine (Chup repo) and data (tesseract-ocr/tessdata_fast).";
    st_ocr_missing_data: "OCR tiếng Việt: chưa tải dữ liệu. Tải từ GitHub (tesseract-ocr/tessdata_fast).",
        "Vietnamese OCR: data not downloaded. Downloads from GitHub (tesseract-ocr/tessdata_fast).";
    st_ocr_download: "Tải về ({})", "Download ({})";
    st_ocr_auto: "Tự nhận diện chữ khi mở Preview", "Recognize text when Preview opens";
    st_ocr_auto_hint: "Để bôi đen và copy chữ ngay trên ảnh Preview.", "So you can select and copy text right on the Preview.";
    st_sec_stack: "Stack ảnh", "Screenshot stack";
    st_edge: "Cạnh màn hình", "Screen edge";
    st_edge_right: "Phải", "Right";
    st_edge_left: "Trái", "Left";
    st_monitor: "Màn hình", "Monitor";
    st_monitor_primary: "Màn hình chính", "Primary monitor";
    st_monitor_cursor: "Màn hình đang có con trỏ", "Monitor with the pointer";
    st_sec_preview: "Preview (bấm vào thumbnail)", "Preview (click a thumbnail)";
    st_pv_top: "Luôn nằm trên cửa sổ khác", "Always on top";
    st_pv_opacity: "Opacity", "Opacity";
    st_pv_opacity_hint: "Ctrl + lăn chuột để chỉnh tạm.", "Ctrl + scroll to adjust temporarily.";
    st_pv_position: "Chỗ mở", "Opens at";
    st_pv_pos_last: "Chỗ lần trước", "Last position";
    st_pv_pos_stack: "Cạnh stack", "Next to the stack";
    st_pv_pos_center: "Giữa màn hình", "Center of the screen";
    st_pv_size: "Cỡ khi mở", "Size when opened";
    st_pv_size_hint: "Chup tự tính cỡ theo từng ảnh. Kéo mép để to, nhỏ hơn: ảnh sau theo tỉ lệ đó.",
        "Chup sizes each image automatically. Drag the edges to change it: later images follow that ratio.";
    st_pv_size_max: "Khoảng {}% màn hình", "About {}% of the screen";
    st_about_desc: "Chụp màn hình theo vùng cho Windows: stack ảnh kéo thả, chụp đóng băng, Preview, copy chữ từ ảnh.",
        "Region screenshots for Windows: a drag-and-drop stack, freeze capture, Preview, copy text from images.";
    st_about_version: "Phiên bản {}", "Version {}";
    st_about_local_build: "bản build tay", "local build";
    st_about_built: "build lúc {} UTC", "built {} UTC";
    st_about_web: "Trang dự án", "Project page";
    st_about_log: "Mở thư mục log", "Open log folder";
    st_about_sec_ocr: "OCR tiếng Việt", "Vietnamese OCR";
    st_sec_update: "Cập nhật", "Updates";
    st_update_auto: "Tự kiểm tra bản mới mỗi ngày (hỏi trước khi cài)", "Check for new versions daily (asks before installing)";
    st_update_check: "Kiểm tra cập nhật", "Check for updates";
    st_update_last: "Kiểm tra lần cuối: {}.", "Last checked: {}.";
    st_update_never: "Chưa kiểm tra lần nào.", "Never checked.";
    st_about_whats_new: "Có gì mới", "What's new";
    st_about_issues: "Báo lỗi, góp ý", "Report an issue";
    st_about_notice: "Giấy phép Tesseract và thư viện", "Tesseract and library licenses";
    st_about_tesseract: "Tesseract, giấy phép Apache-2.0.", "Tesseract, Apache-2.0 license.";
    st_about_engine_installed: "Engine: chup-ocr.exe cài cùng Chup (bộ cài Full).", "Engine: chup-ocr.exe installed with Chup (Full installer).";
    st_about_engine_downloaded: "Engine: đã tải từ {}.", "Engine: downloaded from {}.";
    st_about_engine_missing: "Engine: chưa có (tải ở trang Copy chữ).", "Engine: not downloaded (see the Copy text page).";
    st_about_data: "Dữ liệu: {}.", "Data: {}.";
    st_about_data_missing: "Dữ liệu: chưa tải.", "Data: not downloaded.";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_string_is_translated_and_placeholders_match() {
        for (name, get) in ALL {
            let (vi, en) = (get(&VI), get(&EN));
            assert!(!vi.is_empty() && !en.is_empty(), "{name} rỗng");
            assert_eq!(vi.matches("{}").count(), en.matches("{}").count(), "{name}: số chỗ {{}} khác nhau");
        }
    }

    #[test]
    fn fill_and_resolve() {
        assert_eq!(fill("Đã copy {} dòng: “{}”", &[&3, &"abc"]), "Đã copy 3 dòng: “abc”");
        assert_eq!(fill("a {} b", &[]), "a {} b");
        assert_eq!(fill("{}%", &[&50]), "50%");
        assert_eq!(UiLanguage::System.resolve(true), Lang::Vi);
        assert_eq!(UiLanguage::System.resolve(false), Lang::En);
        assert_eq!(UiLanguage::Vi.resolve(false), Lang::Vi);
        assert_eq!(UiLanguage::En.resolve(true), Lang::En);
        assert_eq!(current(), Lang::Vi, "mặc định tiếng Việt");
    }
}
