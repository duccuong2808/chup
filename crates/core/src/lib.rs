//! Lõi của Chup: không phụ thuộc hệ điều hành, test được trên mọi máy. Xem SPEC.md.
//! Lớp giao diện native của từng OS nằm ở crate `chup` (crates/app).

pub mod capture;
pub mod clipboard;
pub mod config;
pub mod coords;
pub mod freeze;
pub mod hotkey_name;
pub mod icons;
pub mod model;
pub mod ocr;
pub mod preview;
pub mod selection;
pub mod stack;
