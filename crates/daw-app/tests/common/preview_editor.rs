//! Test-only native editor worker for `DAW_PLUGIN_HOST`. It answers Rill's
//! editor protocol with a generated panel image instead of loading plug-in
//! code, and appends each captured plug-in to `captures.log` beside itself.
//! Standard library only: tests compile it directly with rustc.
use std::io::{BufRead, Write};

fn main() {
    if std::env::args().nth(1).as_deref() != Some("--editor") {
        eprintln!("The preview editor fixture only supports --editor");
        std::process::exit(2);
    }
    let log = std::env::current_exe()
        .unwrap()
        .with_file_name("captures.log");
    let mut output = std::io::stdout().lock();
    let mut plugin = String::new();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let reply = if line.contains(r#""command":"open""#) {
            plugin = format!("{}#{}", field(&line, "path"), field(&line, "class_id"));
            r#"{"event":"ready","parameters":[]}"#.to_owned()
        } else if line.contains(r#""command":"capture""#) {
            let path = field(&line, "path");
            let (width, height) = (360, 200);
            std::fs::write(&path, panel(width, height, seed(&plugin))).unwrap();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
                .unwrap();
            writeln!(file, "{plugin}").unwrap();
            format!(r#"{{"event":"captured","path":"{path}","width":{width},"height":{height}}}"#)
        } else if line.contains(r#""command":"close""#) {
            r#"{"event":"closed","request_id":null}"#.to_owned()
        } else {
            continue;
        };
        writeln!(output, "{reply}").unwrap();
        output.flush().unwrap();
        if reply.contains(r#""event":"closed""#) {
            return;
        }
    }
}

/// A JSON string field from Rill's single-line commands. Test paths contain no
/// escaped characters; a missing or null field is empty.
fn field(line: &str, name: &str) -> String {
    let key = format!(r#""{name}":""#);
    line.find(&key)
        .map(|start| {
            let rest = &line[start + key.len()..];
            rest[..rest.find('"').unwrap_or(rest.len())].to_owned()
        })
        .unwrap_or_default()
}

fn seed(text: &str) -> u32 {
    text.bytes()
        .fold(2_166_136_261u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(16_777_619))
}

/// A rack-style panel: a tinted face with a row of knobs, distinct per plug-in.
fn panel(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let tint = [(seed & 0x3f) as i32, ((seed >> 8) & 0x5f) as i32, ((seed >> 16) & 0x7f) as i32];
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            let knob = (0..4).any(|index| {
                let (cx, cy) = (45 + index * 90, height / 2 + 10);
                let (dx, dy) = (x as i32 - cx as i32, y as i32 - cy as i32);
                dx * dx + dy * dy < 26 * 26
            });
            let shade = if y < 28 {
                70
            } else if knob {
                150
            } else {
                30 + (y as i32 * 20 / height as i32)
            };
            raw.extend(tint.map(|channel| (shade + channel).clamp(0, 255) as u8));
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks = raw.chunks(65_535).count();
    for (index, block) in raw.chunks(65_535).enumerate() {
        zlib.push(u8::from(index + 1 == blocks));
        zlib.extend((block.len() as u16).to_le_bytes());
        zlib.extend((!(block.len() as u16)).to_le_bytes());
        zlib.extend(block);
    }
    zlib.extend(adler32(&raw).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = [width.to_be_bytes(), height.to_be_bytes()].concat();
    header.extend([8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend((data.len() as u32).to_be_bytes());
    png.extend(kind);
    png.extend(data);
    png.extend(crc32(&[kind.as_slice(), data].concat()).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}
