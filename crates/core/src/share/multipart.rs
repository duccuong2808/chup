//! Body `multipart/form-data` cho Litterbox / catbox.

use super::HttpRequest;

pub enum Part<'a> {
    Text(&'static str, &'a str),
    /// (tên field, tên file, MIME, dữ liệu).
    File(&'static str, &'static str, &'static str, &'a [u8]),
}

/// Dựng request POST multipart. Boundary ngẫu nhiên từ `rng`; dữ liệu (hiếm khi) chứa boundary thì sinh lại.
pub fn request(url: &str, parts: &[Part], rng: &mut dyn FnMut(&mut [u8])) -> HttpRequest {
    let boundary = loop {
        let mut b = [0u8; 12];
        rng(&mut b);
        let s = format!(
            "chup{}",
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        );
        let clash = parts.iter().any(|p| match p {
            Part::File(_, _, _, d) => d.windows(s.len()).any(|w| w == s.as_bytes()),
            Part::Text(_, v) => v.contains(&s),
        });
        if !clash {
            break s;
        }
    };
    let mut body = Vec::new();
    for p in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match p {
            Part::Text(name, value) => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                        .as_bytes(),
                );
            }
            Part::File(name, file, mime, data) => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\nContent-Type: {mime}\r\n\r\n")
                        .as_bytes(),
                );
                body.extend_from_slice(data);
                body.extend_from_slice(b"\r\n");
            }
        }
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    HttpRequest {
        url: url.into(),
        headers: vec![(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )],
        body,
    }
}
