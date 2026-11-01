//! Đưa ảnh vào clipboard (SPEC 6.9).
//!
//! Windows tự gọi Win32 thay vì qua `arboard`, vì ảnh chụp to (4K ≈ 33 MB RGBA):
//! - Dùng lại byte PNG có sẵn (vừa mã hoá khi chụp, hoặc đọc thẳng từ file khi bấm Copy):
//!   không mã hoá PNG lần 2 như arboard.
//! - Dựng CF_DIBV5 bằng 1 lượt ghi thẳng vào bộ nhớ global (lật dọc + RGBA→BGRA cùng lúc),
//!   không tạo thêm bản sao trung gian.
//! - Chuẩn bị xong dữ liệu rồi mới mở clipboard, nên clipboard chỉ bị giữ vài ms.
//!
//! Thứ tự format giống Snipping Tool/arboard: "PNG" trước (giữ alpha, nhiều app ưu tiên),
//! rồi CF_DIBV5; Windows tự suy ra CF_DIB, CF_BITMAP cho app cũ.

use image::RgbaImage;

/// Kích thước `BITMAPV5HEADER`.
pub const DIBV5_HEADER_LEN: usize = 124;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(PNG_SIGNATURE)
}

/// Ghi `BITMAPV5HEADER` + điểm ảnh của `img` vào `dst` (dài `dibv5_len(img)`).
/// 32 bpp BI_BITFIELDS, BGRA không premultiply, hàng từ dưới lên: chiều cao dương vì
/// Word/WordPad không dán được DIB chiều cao âm.
pub fn write_dibv5(img: &RgbaImage, dst: &mut [u8]) {
    let (w, h) = img.dimensions();
    let row = w as usize * 4;
    assert_eq!(dst.len(), dibv5_len(img), "bộ đệm DIB sai kích thước");
    let (hdr, px) = dst.split_at_mut(DIBV5_HEADER_LEN);

    hdr.fill(0);
    let mut put = |off: usize, v: u32| hdr[off..off + 4].copy_from_slice(&v.to_le_bytes());
    put(0, DIBV5_HEADER_LEN as u32); // bV5Size
    put(4, w); // bV5Width
    put(8, h); // bV5Height (dương = bottom-up)
    put(12, 1 | (32 << 16)); // bV5Planes = 1, bV5BitCount = 32
    put(16, 3); // bV5Compression = BI_BITFIELDS
    put(20, (row * h as usize) as u32); // bV5SizeImage
    put(40, 0x00FF_0000); // R
    put(44, 0x0000_FF00); // G
    put(48, 0x0000_00FF); // B
    put(52, 0xFF00_0000); // A
    put(56, 0x7352_4742); // bV5CSType = LCS_sRGB ('sRGB')
    put(108, 4); // bV5Intent = LCS_GM_IMAGES

    let src = img.as_raw();
    for (y, out) in px.chunks_exact_mut(row).enumerate() {
        let sy = h as usize - 1 - y;
        let line = &src[sy * row..(sy + 1) * row];
        for (o, i) in out.chunks_exact_mut(4).zip(line.chunks_exact(4)) {
            o[0] = i[2];
            o[1] = i[1];
            o[2] = i[0];
            o[3] = i[3];
        }
    }
}

pub fn dibv5_len(img: &RgbaImage) -> usize {
    DIBV5_HEADER_LEN + img.width() as usize * img.height() as usize * 4
}

/// Copy ảnh. `png`: byte PNG của chính ảnh này nếu đã có (bỏ qua bước mã hoá lại).
#[cfg(windows)]
pub fn copy_image(img: &RgbaImage, png: Option<&[u8]>) -> Result<(), String> {
    win::copy(img, png)
}

#[cfg(not(windows))]
pub fn copy_image(img: &RgbaImage, _png: Option<&[u8]>) -> Result<(), String> {
    use std::borrow::Cow;
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: Cow::Borrowed(img.as_raw()),
    })
    .map_err(|e| e.to_string())
}

/// Text cho CF_UNICODETEXT: UTF-16, xuống dòng CRLF (Notepad cũ, nhiều app Win32 cần), kết thúc NUL.
pub fn utf16_crlf(text: &str) -> Vec<u16> {
    let mut out: Vec<u16> = text.replace("\r\n", "\n").replace('\n', "\r\n").encode_utf16().collect();
    out.push(0);
    out
}

/// Copy text (kết quả OCR).
#[cfg(windows)]
pub fn copy_text(text: &str) -> Result<(), String> {
    win::copy_text(text)
}

#[cfg(not(windows))]
pub fn copy_text(text: &str) -> Result<(), String> {
    arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)).map_err(|e| e.to_string())
}

/// Copy ảnh từ file (nút Copy trên stack). File PNG thì dùng thẳng byte của file.
pub fn copy_file_image(path: &std::path::Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Không đọc được ảnh: {e}"))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("Không đọc được ảnh: {e}"))?.to_rgba8();
    copy_image(&img, is_png(&bytes).then_some(&bytes[..]))
}

#[cfg(windows)]
mod win {
    use image::RgbaImage;
    use windows::{
        core::w,
        Win32::{
            Foundation::{GlobalFree, HANDLE, HGLOBAL},
            System::{
                DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData},
                Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            },
        },
    };

    const CF_DIBV5: u32 = 17;
    const CF_UNICODETEXT: u32 = 13;

    /// Khối bộ nhớ global; tự giải phóng nếu chưa giao cho clipboard.
    struct Global(Option<HGLOBAL>);

    impl Global {
        fn new(len: usize, fill: impl FnOnce(&mut [u8])) -> Result<Self, String> {
            let h = unsafe { GlobalAlloc(GMEM_MOVEABLE, len) }.map_err(|e| format!("Thiếu bộ nhớ: {e}"))?;
            let g = Global(Some(h));
            let p = unsafe { GlobalLock(h) } as *mut u8;
            if p.is_null() {
                return Err("Không khoá được bộ nhớ".into());
            }
            // SAFETY: GlobalLock trả về vùng ghi được dài ít nhất `len` byte, chỉ ta đang giữ.
            fill(unsafe { std::slice::from_raw_parts_mut(p, len) });
            unsafe {
                // Trả Err khi số lần khoá về 0 (là bình thường).
                let _ = GlobalUnlock(h);
            }
            Ok(g)
        }

        /// Giao cho clipboard (clipboard mở và đã EmptyClipboard). Thành công thì hệ thống sở hữu.
        fn give(&mut self, format: u32) -> Result<(), String> {
            let Some(h) = self.0 else { return Ok(()) };
            unsafe { SetClipboardData(format, Some(HANDLE(h.0))) }.map_err(|e| e.to_string())?;
            self.0 = None;
            Ok(())
        }
    }

    impl Drop for Global {
        fn drop(&mut self) {
            if let Some(h) = self.0.take() {
                // Thành công thì GlobalFree trả NULL, crate `windows` coi là Err: bỏ qua.
                let _ = unsafe { GlobalFree(Some(h)) };
            }
        }
    }

    struct Open;

    impl Open {
        /// App khác (trình quản lý clipboard, Office...) có thể đang giữ clipboard: thử lại
        /// trong ~0,5 s như Chromium/Firefox.
        fn new() -> Result<Self, String> {
            for _ in 0..100 {
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(Open);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err("Clipboard đang bị app khác giữ".into())
        }
    }

    impl Drop for Open {
        fn drop(&mut self) {
            let _ = unsafe { CloseClipboard() };
        }
    }

    pub fn copy_text(text: &str) -> Result<(), String> {
        let wide = super::utf16_crlf(text);
        let mut mem = Global::new(wide.len() * 2, |d| {
            for (o, c) in d.chunks_exact_mut(2).zip(&wide) {
                o.copy_from_slice(&c.to_le_bytes());
            }
        })?;
        let _open = Open::new()?;
        unsafe { EmptyClipboard() }.map_err(|e| format!("Không xoá được clipboard: {e}"))?;
        mem.give(CF_UNICODETEXT).map_err(|e| format!("Không đặt được text vào clipboard: {e}"))
    }

    pub fn copy(img: &RgbaImage, png: Option<&[u8]>) -> Result<(), String> {
        let t0 = std::time::Instant::now();
        let mut png_mem = match png {
            Some(b) => Some(Global::new(b.len(), |d| d.copy_from_slice(b))?),
            None => None,
        };
        let mut dib = Global::new(super::dibv5_len(img), |d| super::write_dibv5(img, d))?;
        let png_fmt = unsafe { RegisterClipboardFormatW(w!("PNG")) };
        let t_prep = t0.elapsed();

        let _open = Open::new()?;
        unsafe { EmptyClipboard() }.map_err(|e| format!("Không xoá được clipboard: {e}"))?;
        if let Some(m) = png_mem.as_mut().filter(|_| png_fmt != 0) {
            // Thiếu PNG vẫn còn DIB: chỉ ghi log.
            if let Err(e) = m.give(png_fmt) {
                log::warn!("clipboard: không đặt được PNG: {e}");
            }
        }
        dib.give(CF_DIBV5).map_err(|e| format!("Không đặt được ảnh vào clipboard: {e}"))?;
        log::debug!(
            "clipboard {}×{}: chuẩn bị {t_prep:?}, giữ clipboard {:?}, PNG {}",
            img.width(),
            img.height(),
            t0.elapsed() - t_prep,
            if png.is_some() { "có sẵn" } else { "không" }
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dibv5_header_and_pixels() {
        // 2×2: hàng trên đỏ/xanh lá, hàng dưới xanh dương/trắng mờ.
        let img = RgbaImage::from_raw(
            2,
            2,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128],
        )
        .unwrap();
        let mut d = vec![0xAA; dibv5_len(&img)];
        write_dibv5(&img, &mut d);
        let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        assert_eq!(d.len(), 124 + 16);
        assert_eq!(u32_at(0), 124);
        assert_eq!((u32_at(4), u32_at(8)), (2, 2));
        assert_eq!(u32_at(12), 1 | (32 << 16));
        assert_eq!(u32_at(16), 3);
        assert_eq!(u32_at(20), 16);
        assert_eq!(u32_at(52), 0xFF00_0000);
        assert_eq!(u32_at(60), 0, "phần còn lại của header phải là 0");
        // Bottom-up: hàng dưới của ảnh nằm đầu; BGRA.
        assert_eq!(&d[124..140], &[255, 0, 0, 255, 255, 255, 255, 128, 0, 0, 255, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn text_is_crlf_utf16_with_nul() {
        let w = utf16_crlf("Việt\nb\r\nc");
        assert_eq!(String::from_utf16(&w[..w.len() - 1]).unwrap(), "Việt\r\nb\r\nc");
        assert_eq!(*w.last().unwrap(), 0);
    }

    #[test]
    fn png_signature() {
        assert!(is_png(b"\x89PNG\r\n\x1a\nrest"));
        assert!(!is_png(b"GIF89a"));
    }
}
