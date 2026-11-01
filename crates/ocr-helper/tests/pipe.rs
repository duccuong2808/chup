//! Chạy `chup-ocr` thật qua pipe với ảnh của spike OCR, qua đúng pipeline của Chup (prepare -> helper -> postprocess).
//! Cần dữ liệu tessdata_fast: `CHUP_TESSDATA=<thư mục có vie/eng.traineddata>`. Không đặt thì bỏ qua.

use std::{
    io::{BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use chup_core::{config::OcrLanguage, ocr};
use chup_ocr_proto::{read_json, write_frame, Hello, Request, Response, PROTO};

fn tessdata() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("CHUP_TESSDATA")?);
    d.join("vie.traineddata").is_file().then_some(d)
}

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn cer(got: &str, truth: &str) -> f64 {
    let norm = |s: &str| s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
    let (g, t): (Vec<char>, Vec<char>) = (norm(got).chars().collect(), norm(truth).chars().collect());
    levenshtein(&g, &t) as f64 / t.len() as f64
}

#[test]
fn recognizes_spike_images_over_pipe() {
    let Some(dir) = tessdata() else {
        eprintln!("bỏ qua: chưa đặt CHUP_TESSDATA");
        return;
    };
    let truth = std::fs::read_to_string(data("truth.txt")).unwrap();
    let t0 = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_chup-ocr"))
        .arg("--tessdata")
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let hello: Hello = read_json(&mut stdout).unwrap().unwrap();
    assert_eq!(hello.proto, PROTO);
    eprintln!("khởi động + nạp model: {:?} (tesseract {})", t0.elapsed(), hello.tesseract);

    // np200: chụp ở 200% (không phóng). np100: 100% (prepare phóng ×2).
    for (id, (file, scale)) in [("np200.png", 2.0), ("np100.png", 1.0)].into_iter().enumerate() {
        let img = image::open(data(file)).unwrap().to_rgba8();
        let gray = ocr::prepare(&img, scale, OcrLanguage::Vi);
        let (w, h) = gray.dimensions();
        let req = Request { id: id as u32 + 1, width: w, height: h, pixels: gray.into_raw() };
        let t = Instant::now();
        write_frame(&mut stdin, &req.encode()).unwrap();
        let resp: Response = read_json(&mut stdout).unwrap().unwrap();
        let Response::Ok { id: rid, lines } = resp else { panic!("{file}: {resp:?}") };
        assert_eq!(rid, req.id);
        let text = ocr::postprocess(lines);
        let c = cer(&text, &truth);
        eprintln!("{file}: {:?}, CER {:.2}%\n{text}", t.elapsed(), c * 100.0);
        assert!(c <= 0.03, "{file}: CER {:.2}% > 3%", c * 100.0);
    }

    // Tin hỏng: trả lỗi, không chết.
    write_frame(&mut stdin, &[1, 2, 3]).unwrap();
    let resp: Response = read_json(&mut stdout).unwrap().unwrap();
    assert!(matches!(resp, Response::Err { id: 0, .. }), "{resp:?}");

    // Đóng stdin: exe phụ thoát sạch.
    stdin.flush().unwrap();
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn bad_tessdata_exits_with_code_2() {
    let status = Command::new(env!("CARGO_BIN_EXE_chup-ocr"))
        .args(["--tessdata", "/khong/co/thu/muc/nay"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}
