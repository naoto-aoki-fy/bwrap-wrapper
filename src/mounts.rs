//! Injectable `/proc/mounts` parsing matching the Python UTF-8 text parser.
use std::ffi::OsString;

pub fn parse(contents: &str) -> Vec<OsString> {
    let mut mounts = Vec::new();
    for line in contents.lines() {
        let fields: Vec<_> = line.split_whitespace().take(4).collect();
        if fields.len() < 4
            || fields[2] != "9p"
            || !fields[3].split([',', ';']).any(|o| o == "aname=drvfs")
        {
            continue;
        }
        let mut decoded = String::new();
        let mut chars = fields[1].chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                let digits: String = chars.clone().take(3).collect();
                if digits.len() == 3 && digits.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
                    for _ in 0..3 {
                        chars.next();
                    }
                    // Python chr(), including octal escapes above ASCII, rather than raw bytes.
                    decoded.push(char::from_u32(u32::from_str_radix(&digits, 8).unwrap()).unwrap());
                    continue;
                }
            }
            decoded.push(ch);
        }
        mounts.push(decoded);
    }
    mounts.sort_by(|a, b| {
        a.bytes()
            .filter(|c| *c == b'/')
            .count()
            .cmp(&b.bytes().filter(|c| *c == b'/').count())
            .then_with(|| a.cmp(b))
    });
    mounts.dedup();
    mounts.into_iter().map(OsString::from).collect()
}

/// Read and parse the real mount table. I/O and UTF-8 errors are returned to the caller.
pub fn read_proc_mounts() -> std::io::Result<Vec<OsString>> {
    read_file("/proc/mounts")
}

/// Read and parse a mount table at an explicit path, useful for testing I/O failures.
pub fn read_file(path: impl AsRef<std::path::Path>) -> std::io::Result<Vec<OsString>> {
    let contents = std::fs::read_to_string(path)?;
    Ok(parse(&contents))
}
