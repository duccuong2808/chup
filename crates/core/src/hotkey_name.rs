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

/// Mã phím của macOS (`kVK_*`, bàn phím ANSI) -> VK của Windows, để ô ghi phím trên Mac dùng chung `record`.
/// Modifier: Cmd -> 0x5B (Win), Shift -> 0x10, Option -> 0x12, Control -> 0x11.
pub fn vk_from_mac(code: u16) -> Option<u16> {
    const LETTERS: [(u16, u8); 26] = [
        (0x00, b'A'), (0x0B, b'B'), (0x08, b'C'), (0x02, b'D'), (0x0E, b'E'), (0x03, b'F'), (0x05, b'G'),
        (0x04, b'H'), (0x22, b'I'), (0x26, b'J'), (0x28, b'K'), (0x25, b'L'), (0x2E, b'M'), (0x2D, b'N'),
        (0x1F, b'O'), (0x23, b'P'), (0x0C, b'Q'), (0x0F, b'R'), (0x01, b'S'), (0x11, b'T'), (0x20, b'U'),
        (0x09, b'V'), (0x0D, b'W'), (0x07, b'X'), (0x10, b'Y'), (0x06, b'Z'),
    ];
    const DIGITS: [(u16, u8); 10] = [
        (0x1D, b'0'), (0x12, b'1'), (0x13, b'2'), (0x14, b'3'), (0x15, b'4'), (0x17, b'5'), (0x16, b'6'),
        (0x1A, b'7'), (0x1C, b'8'), (0x19, b'9'),
    ];
    // F1..F20 theo thứ tự.
    const FKEYS: [u16; 20] = [
        0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F,
        0x50, 0x5A,
    ];
    const NUMPAD: [u16; 10] = [0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5B, 0x5C];
    if let Some((_, c)) = LETTERS.iter().find(|(k, _)| *k == code) {
        return Some(*c as u16);
    }
    if let Some((_, c)) = DIGITS.iter().find(|(k, _)| *k == code) {
        return Some(*c as u16);
    }
    if let Some(i) = FKEYS.iter().position(|k| *k == code) {
        return Some(0x70 + i as u16);
    }
    if let Some(i) = NUMPAD.iter().position(|k| *k == code) {
        return Some(0x60 + i as u16);
    }
    Some(match code {
        0x37 | 0x36 => 0x5B,
        0x38 | 0x3C => 0x10,
        0x3A | 0x3D => 0x12,
        0x3B | 0x3E => 0x11,
        0x31 => 0x20,
        0x30 => 0x09,
        0x24 | 0x4C => 0x0D,
        0x33 => 0x08,
        0x75 => 0x2E,
        0x73 => 0x24,
        0x77 => 0x23,
        0x74 => 0x21,
        0x79 => 0x22,
        0x7B => 0x25,
        0x7E => 0x26,
        0x7C => 0x27,
        0x7D => 0x28,
        0x43 => 0x6A,
        0x45 => 0x6B,
        0x4E => 0x6D,
        0x41 => 0x6E,
        0x4B => 0x6F,
        0x29 => 0xBA,
        0x18 => 0xBB,
        0x2B => 0xBC,
        0x1B => 0xBD,
        0x2F => 0xBE,
        0x2C => 0xBF,
        0x32 => 0xC0,
        0x21 => 0xDB,
        0x2A => 0xDC,
        0x1E => 0xDD,
        0x27 => 0xDE,
        _ => return None,
    })
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

    #[test]
    fn mac_key_codes_map_to_vk() {
        let ctrl_cmd = Mods { ctrl: true, win: true, ..Default::default() };
        assert_eq!(record(vk_from_mac(0x00).unwrap(), ctrl_cmd), Recorded::Accelerator("Ctrl+Super+A".into()));
        assert_eq!(record(vk_from_mac(0x06).unwrap(), ctrl_cmd), Recorded::Accelerator("Ctrl+Super+Z".into()));
        assert_eq!(record(vk_from_mac(0x7A).unwrap(), Mods::default()), Recorded::Accelerator("F1".into()));
        assert_eq!(record(vk_from_mac(0x37).unwrap(), Mods { win: true, ..Default::default() }), Recorded::Incomplete("Super+…".into()));
        assert_eq!(vk_from_mac(0x1D), Some(b'0' as u16));
        assert_eq!(vk_from_mac(0x7E), Some(0x26));
        assert_eq!(vk_from_mac(0x35), None, "Esc không dùng làm hotkey");
    }
}
