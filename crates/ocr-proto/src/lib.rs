//! Giao thức giữa `chup.exe` và exe phụ `chup-ocr.exe` (SPEC 15.4, thiết kế `docs/plans/2026-11-01-ocr-design.md`).
//!
//! - Mỗi tin là một khung: `u32` little-endian (độ dài) rồi tới nội dung.
//! - Exe phụ nạp model xong thì gửi [`Hello`] (JSON).
//! - Chup gửi [`Request`] (nhị phân: header + pixel xám 8-bit), exe phụ trả [`Response`] (JSON).
//! - Chup đóng stdin là exe phụ thoát.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// Tăng khi đổi định dạng tin. Exe phụ lệch số này với `chup.exe` thì không dùng được.
pub const PROTO: u32 = 1;

/// Khung lớn nhất nhận vào (ảnh xám 16384 × 16384 = 256 MiB). Lớn hơn thì coi là luồng hỏng.
pub const MAX_FRAME: usize = 256 << 20;

const HEADER: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub proto: u32,
    /// Version Tesseract, chỉ để ghi log.
    pub tesseract: String,
}

/// Ảnh cần nhận diện: xám 8-bit, `width × height` byte, không padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.pixels.len());
        for v in [self.id, self.width, self.height] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.pixels);
        out
    }

    pub fn decode(mut buf: Vec<u8>) -> Result<Self, String> {
        if buf.len() < HEADER {
            return Err("tin quá ngắn".into());
        }
        let word = |i: usize| u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        let (id, width, height) = (word(0), word(4), word(8));
        let expected = width as usize * height as usize;
        if width == 0 || height == 0 || buf.len() - HEADER != expected {
            return Err(format!("kích thước ảnh không khớp: {width}×{height}, {} byte", buf.len() - HEADER));
        }
        buf.drain(..HEADER);
        Ok(Self { id, width, height, pixels: buf })
    }
}

/// Một dòng chữ. `bbox` = `[x, y, w, h]` (px của ảnh gửi đi), `conf` 0–100.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub text: String,
    pub bbox: [i32; 4],
    pub conf: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Ok { id: u32, lines: Vec<Line> },
    Err { id: u32, error: String },
}

impl Response {
    pub fn id(&self) -> u32 {
        match self {
            Response::Ok { id, .. } | Response::Err { id, .. } => *id,
        }
    }
}

pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "khung quá lớn"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// Đọc một khung. `Ok(None)` khi bên kia đóng luồng đúng ranh giới khung.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..])? {
            0 if got == 0 => return Ok(None),
            0 => return Err(io::ErrorKind::UnexpectedEof.into()),
            n => got += n,
        }
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("khung {len} byte vượt giới hạn")));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

pub fn write_json(w: &mut impl Write, v: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(v).map_err(io::Error::other)?;
    write_frame(w, &bytes)
}

pub fn read_json<T: for<'de> Deserialize<'de>>(r: &mut impl Read) -> io::Result<Option<T>> {
    match read_frame(r)? {
        None => Ok(None),
        Some(b) => serde_json::from_slice(&b).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip_through_frames() {
        let req = Request { id: 7, width: 3, height: 2, pixels: vec![1, 2, 3, 4, 5, 6] };
        let mut wire = vec![];
        write_frame(&mut wire, &req.encode()).unwrap();
        write_frame(&mut wire, &req.encode()).unwrap();
        let mut r = wire.as_slice();
        assert_eq!(Request::decode(read_frame(&mut r).unwrap().unwrap()).unwrap(), req);
        assert_eq!(Request::decode(read_frame(&mut r).unwrap().unwrap()).unwrap(), req);
        assert_eq!(read_frame(&mut r).unwrap(), None, "đóng đúng ranh giới khung");
    }

    #[test]
    fn bad_requests_are_rejected() {
        assert!(Request::decode(vec![0; 5]).is_err());
        let mut b = Request { id: 1, width: 2, height: 2, pixels: vec![0; 4] }.encode();
        b.pop();
        assert!(Request::decode(b).is_err());
        assert!(Request::decode(Request { id: 1, width: 0, height: 0, pixels: vec![] }.encode()).is_err());
    }

    #[test]
    fn truncated_or_huge_frames_are_errors() {
        let mut wire = vec![];
        write_frame(&mut wire, b"hello").unwrap();
        wire.truncate(6);
        assert!(read_frame(&mut wire.as_slice()).is_err());
        assert!(read_frame(&mut [1u8, 0].as_slice()).is_err(), "đứt giữa phần độ dài");
        let huge = ((MAX_FRAME + 1) as u32).to_le_bytes();
        assert!(read_frame(&mut huge.as_slice()).is_err());
    }

    #[test]
    fn response_json_shapes() {
        let ok = Response::Ok {
            id: 3,
            lines: vec![Line { text: "Xin chào".into(), bbox: [1, 2, 30, 10], conf: 91.5 }],
        };
        let err = Response::Err { id: 4, error: "hỏng".into() };
        for r in [&ok, &err] {
            let mut wire = vec![];
            write_json(&mut wire, r).unwrap();
            assert_eq!(read_json::<Response>(&mut wire.as_slice()).unwrap().as_ref(), Some(r));
        }
        assert_eq!(serde_json::to_value(&err).unwrap(), serde_json::json!({ "id": 4, "error": "hỏng" }));
        assert_eq!((ok.id(), err.id()), (3, 4));
        let hello: Hello = serde_json::from_str(r#"{"proto":1,"tesseract":"5.5.1"}"#).unwrap();
        assert_eq!(hello.proto, PROTO);
    }
}
