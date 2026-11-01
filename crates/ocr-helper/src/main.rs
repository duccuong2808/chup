//! `chup-ocr.exe`: exe phụ chạy Tesseract cho OCR tiếng Việt (SPEC 15.4).
//!
//! `chup-ocr --tessdata <thư mục> [--langs vie+eng]`
//!
//! Nạp model một lần, gửi `Hello`, rồi nhận `Request` / trả `Response` qua stdin/stdout (crate `chup-ocr-proto`)
//! cho tới khi stdin đóng. Lỗi khởi động: in ra stderr, exit code 2. Tesseract chỉ ghi log ra stderr,
//! nên stdout chỉ chứa khung tin.

mod tess;

use std::{
    io::{self, BufReader, BufWriter},
    path::PathBuf,
    process::ExitCode,
};

use chup_ocr_proto::{read_frame, write_json, Hello, Request, Response, PROTO};

struct Args {
    tessdata: PathBuf,
    langs: String,
}

fn parse_args() -> Result<Args, String> {
    let mut tessdata = None;
    let mut langs = "vie+eng".to_string();
    let mut it = std::env::args_os().skip(1);
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--tessdata") => tessdata = it.next().map(PathBuf::from),
            Some("--langs") => langs = it.next().and_then(|s| s.into_string().ok()).ok_or("thiếu giá trị --langs")?,
            Some("--version") => {
                println!("chup-ocr {} (tesseract {}, proto {PROTO})", env!("CARGO_PKG_VERSION"), tess::version());
                std::process::exit(0);
            }
            _ => return Err(format!("tham số lạ: {a:?}")),
        }
    }
    Ok(Args { tessdata: tessdata.ok_or("thiếu --tessdata <thư mục>")?, langs })
}

fn serve(api: &mut tess::Api) -> io::Result<()> {
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = BufWriter::new(io::stdout().lock());
    write_json(&mut output, &Hello { proto: PROTO, tesseract: tess::version() })?;
    while let Some(frame) = read_frame(&mut input)? {
        let resp = match Request::decode(frame) {
            // id không đọc được: trả id 0, phía Chup sẽ coi là lỗi của job đang chạy.
            Err(e) => Response::Err { id: 0, error: e },
            Ok(req) => match api.recognize(&req.pixels, req.width, req.height) {
                Ok(lines) => Response::Ok { id: req.id, lines },
                Err(error) => Response::Err { id: req.id, error },
            },
        };
        write_json(&mut output, &resp)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("chup-ocr: {e}");
            return ExitCode::from(2);
        }
    };
    let mut api = match tess::Api::new(&args.tessdata, &args.langs) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("chup-ocr: {e}");
            return ExitCode::from(2);
        }
    };
    match serve(&mut api) {
        Ok(()) => ExitCode::SUCCESS,
        // Chup đã đóng pipe (huỷ, thoát): không phải lỗi đáng báo.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("chup-ocr: {e}");
            ExitCode::FAILURE
        }
    }
}
