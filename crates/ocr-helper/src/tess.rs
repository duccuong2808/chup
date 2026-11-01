//! FFI tối thiểu tới C API của Tesseract 5 (`tesseract/capi.h`). Viết tay để khỏi cần bindgen/libclang.

use std::{
    ffi::{c_char, c_float, c_int, c_uchar, c_void, CStr, CString},
    path::Path,
    ptr::NonNull,
};

use chup_ocr_proto::Line;

type Handle = c_void;
type ResultIter = c_void;
type PageIter = c_void;

/// `TessPageIteratorLevel::RIL_TEXTLINE`.
const RIL_TEXTLINE: c_int = 2;
/// `TessOcrEngineMode::OEM_LSTM_ONLY`.
const OEM_LSTM_ONLY: c_int = 1;
/// Ảnh chụp màn hình không có DPI; CLI của Tesseract tự dùng 70 (bộ số của spike đo với giá trị này).
const SOURCE_PPI: c_int = 70;

extern "C" {
    fn TessVersion() -> *const c_char;
    fn TessDeleteText(text: *const c_char);
    fn TessBaseAPICreate() -> *mut Handle;
    fn TessBaseAPIDelete(h: *mut Handle);
    fn TessBaseAPIInit2(h: *mut Handle, datapath: *const c_char, language: *const c_char, oem: c_int) -> c_int;
    fn TessBaseAPISetImage(h: *mut Handle, data: *const c_uchar, w: c_int, h_: c_int, bpp: c_int, bpl: c_int);
    fn TessBaseAPISetSourceResolution(h: *mut Handle, ppi: c_int);
    fn TessBaseAPIRecognize(h: *mut Handle, monitor: *mut c_void) -> c_int;
    fn TessBaseAPIGetIterator(h: *mut Handle) -> *mut ResultIter;
    fn TessBaseAPIClear(h: *mut Handle);
    fn TessResultIteratorDelete(it: *mut ResultIter);
    fn TessResultIteratorGetPageIterator(it: *mut ResultIter) -> *mut PageIter;
    fn TessResultIteratorNext(it: *mut ResultIter, level: c_int) -> c_int;
    fn TessResultIteratorGetUTF8Text(it: *const ResultIter, level: c_int) -> *mut c_char;
    fn TessResultIteratorConfidence(it: *const ResultIter, level: c_int) -> c_float;
    fn TessPageIteratorBoundingBox(
        it: *const PageIter,
        level: c_int,
        left: *mut c_int,
        top: *mut c_int,
        right: *mut c_int,
        bottom: *mut c_int,
    ) -> c_int;
}

pub fn version() -> String {
    // SAFETY: TessVersion trả chuỗi tĩnh, kết thúc bằng NUL.
    unsafe { CStr::from_ptr(TessVersion()) }.to_string_lossy().into_owned()
}

/// Một phiên Tesseract đã nạp model. Chỉ dùng trên 1 thread.
pub struct Api(NonNull<Handle>);

impl Api {
    pub fn new(tessdata: &Path, langs: &str) -> Result<Self, String> {
        let dir = CString::new(tessdata.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let langs_c = CString::new(langs).map_err(|e| e.to_string())?;
        // SAFETY: TessBaseAPICreate trả handle mới (hoặc NULL); Drop gọi TessBaseAPIDelete đúng 1 lần.
        let api = Api(NonNull::new(unsafe { TessBaseAPICreate() }).ok_or("TessBaseAPICreate trả NULL")?);
        // SAFETY: handle hợp lệ, 2 chuỗi sống tới hết lời gọi.
        let rc = unsafe { TessBaseAPIInit2(api.0.as_ptr(), dir.as_ptr(), langs_c.as_ptr(), OEM_LSTM_ONLY) };
        if rc != 0 {
            return Err(format!("không nạp được model {langs} từ {}", tessdata.display()));
        }
        Ok(api)
    }

    /// Nhận diện ảnh xám 8-bit `w × h` (không padding), trả về các dòng.
    pub fn recognize(&mut self, pixels: &[u8], w: u32, h: u32) -> Result<Vec<Line>, String> {
        let (wi, hi) = (c_int::try_from(w).map_err(|_| "ảnh quá rộng")?, c_int::try_from(h).map_err(|_| "ảnh quá cao")?);
        if pixels.len() != w as usize * h as usize {
            return Err("kích thước buffer không khớp".into());
        }
        let h_ = self.0.as_ptr();
        // SAFETY: buffer đủ w*h byte và sống tới hết hàm; Tesseract chép ảnh khi SetImage.
        unsafe {
            TessBaseAPISetImage(h_, pixels.as_ptr(), wi, hi, 1, wi);
            TessBaseAPISetSourceResolution(h_, SOURCE_PPI);
            if TessBaseAPIRecognize(h_, std::ptr::null_mut()) != 0 {
                TessBaseAPIClear(h_);
                return Err("Tesseract nhận diện lỗi".into());
            }
        }
        let mut lines = vec![];
        // SAFETY: iterator thuộc về ta (xoá bằng TessResultIteratorDelete); page iterator là cùng đối tượng,
        // không xoá riêng. Chuỗi UTF-8 trả về phải xoá bằng TessDeleteText.
        unsafe {
            let it = TessBaseAPIGetIterator(h_);
            if !it.is_null() {
                loop {
                    let p = TessResultIteratorGetUTF8Text(it, RIL_TEXTLINE);
                    if !p.is_null() {
                        let text = CStr::from_ptr(p).to_string_lossy().into_owned();
                        TessDeleteText(p);
                        let conf = TessResultIteratorConfidence(it, RIL_TEXTLINE);
                        let (mut l, mut t, mut r, mut b) = (0, 0, 0, 0);
                        let page = TessResultIteratorGetPageIterator(it);
                        if TessPageIteratorBoundingBox(page, RIL_TEXTLINE, &mut l, &mut t, &mut r, &mut b) != 0 {
                            lines.push(Line { text, bbox: [l, t, r - l, b - t], conf });
                        }
                    }
                    if TessResultIteratorNext(it, RIL_TEXTLINE) == 0 {
                        break;
                    }
                }
                TessResultIteratorDelete(it);
            }
            TessBaseAPIClear(h_);
        }
        Ok(lines)
    }
}

impl Drop for Api {
    fn drop(&mut self) {
        // SAFETY: handle tạo bởi TessBaseAPICreate, chưa bị xoá.
        unsafe { TessBaseAPIDelete(self.0.as_ptr()) }
    }
}
