//! Ghi phím tắt trong màn hình Cài đặt: đổi mã phím ảo (VK) sang tên mà crate
//! `global-hotkey` parse được, và ghép thành chuỗi kiểu `Ctrl+Alt+A`.
//! Mã VK là số của Windows, nhưng hàm thuần nên test được trên mọi OS.

/// Modifier đang giữ khi nhấn phím.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl Mods {
    pub fn any(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.win
    }

    /// Tiền tố hiển thị khi người dùng mới giữ modifier, chưa nhấn phím chính: `Ctrl+Alt+…`.
    pub fn partial(&self) -> Option<String> {
        let names = self.names();
        (!names.is_empty()).then(|| format!("{}+…", names.join("+")))
    }

    fn names(&self) -> Vec<&'static str> {
        let mut v = vec![];
        if self.ctrl {
            v.push("Ctrl");
        }
        if self.alt {
            v.push("Alt");
        }
        if self.shift {
            v.push("Shift");
        }
        if self.win {
            v.push("Super");
        }
        v
    }
}

/// VK của các phím modifier (không phải phím chính).
pub fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// Tên phím chính theo định dạng của `global-hotkey`, hoặc `None` nếu không dùng làm hotkey được.
pub fn key_name(vk: u16) -> Option<String> {
    let s = match vk {
        0x41..=0x5A => return Some(char::from(vk as u8).to_string()), // A..Z
        0x30..=0x39 => return Some(char::from(vk as u8).to_string()), // 0..9
        0x70..=0x87 => return Some(format!("F{}", vk - 0x6F)),        // F1..F24
        0x60..=0x69 => return Some(format!("Numpad{}", vk - 0x60)),
        0x20 => "Space",
        0x09 => "Tab",
        0x0D => "Enter",
        0x08 => "Backspace",
        0x2E => "Delete",
        0x2D => "Insert",
        0x24 => "Home",
        0x23 => "End",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x25 => "ArrowLeft",
        0x26 => "ArrowUp",
        0x27 => "ArrowRight",
        0x28 => "ArrowDown",
        0x2C => "PrintScreen",
        0x13 => "Pause",
        0x6A => "NumpadMultiply",
        0x6B => "NumpadAdd",
        0x6D => "NumpadSubtract",
        0x6E => "NumpadDecimal",
        0x6F => "NumpadDivide",
        0xBA => "Semicolon",
        0xBB => "Equal",
        0xBC => "Comma",
        0xBD => "Minus",
        0xBE => "Period",
        0xBF => "Slash",
        0xC0 => "Backquote",
        0xDB => "BracketLeft",
        0xDC => "Backslash",
        0xDD => "BracketRight",
        0xDE => "Quote",
        _ => return None,
    };
    Some(s.to_string())
}

/// Kết quả khi người dùng nhấn 1 tổ hợp trong ô ghi phím.
#[derive(Debug, PartialEq, Eq)]
pub enum Recorded {
    /// Tổ hợp hợp lệ, ví dụ `Ctrl+Alt+A`.
    Accelerator(String),
    /// Chưa đủ: mới giữ modifier, hoặc phím thường mà không có Ctrl/Alt/Super.
    Incomplete(String),
    /// Phím không dùng làm hotkey được: bỏ qua.
    Ignored,
}

/// Ghép tổ hợp. Phím thường bắt buộc có Ctrl, Alt hoặc Super (để không chặn phím gõ chữ);
/// riêng F1–F24 được dùng một mình.
pub fn record(vk: u16, mods: Mods) -> Recorded {
    if is_modifier_vk(vk) {
        return match mods.partial() {
            Some(p) => Recorded::Incomplete(p),
            None => Recorded::Ignored,
        };
    }
    let Some(key) = key_name(vk) else { return Recorded::Ignored };
    let is_fn = (0x70..=0x87).contains(&vk);
    if !(is_fn || mods.ctrl || mods.alt || mods.win) {
        return Recorded::Incomplete(match mods.partial() {
            Some(p) => p,
            None => crate::i18n::t().hotkey_need_modifier.into(),
        });
    }
    let mut parts = mods.names();
    parts.push(&key);
    Recorded::Accelerator(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CA: Mods = Mods { ctrl: true, alt: true, shift: false, win: false };

    #[test]
    fn records_common_combos() {
        assert_eq!(record(0x41, CA), Recorded::Accelerator("Ctrl+Alt+A".into()));
        assert_eq!(record(0x35, CA), Recorded::Accelerator("Ctrl+Alt+5".into()));
        assert_eq!(record(0x70, Mods::default()), Recorded::Accelerator("F1".into()));
        assert_eq!(
            record(0x2C, Mods { shift: true, win: true, ..Default::default() }),
            Recorded::Accelerator("Shift+Super+PrintScreen".into())
        );
    }

    #[test]
    fn incomplete_and_ignored() {
        assert_eq!(record(0x11, CA), Recorded::Incomplete("Ctrl+Alt+…".into()));
        assert_eq!(record(0x41, Mods::default()), Recorded::Incomplete("Cần có Ctrl, Alt hoặc dùng phím F1–F24".into()));
        assert_eq!(
            record(0x41, Mods { shift: true, ..Default::default() }),
            Recorded::Incomplete("Shift+…".into())
        );
        assert_eq!(record(0x10, Mods::default()), Recorded::Ignored);
        assert_eq!(record(0xFF, CA), Recorded::Ignored);
    }
}
