//! Append-only invocation log compatible with the Python shim's shell quoting.
use crate::transform::Output;
use std::ffi::{CStr, OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub fn append(
    path: &OsStr,
    real_bwrap: &OsStr,
    original: &[OsString],
    output: &Output,
    environment: &[(OsString, OsString)],
) -> io::Result<()> {
    let parent = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    })?;
    let timestamp = local_timestamp()?;
    // SAFETY: getuid is side-effect free; getpwuid returns static storage copied before another libc call.
    let username = unsafe {
        let record = libc::getpwuid(libc::getuid());
        if record.is_null() || (*record).pw_name.is_null() {
            return Err(io::Error::last_os_error());
        }
        CStr::from_ptr((*record).pw_name).to_bytes().to_vec()
    };
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "\n========== BWRAP START ==========")?;
    writeln!(file, "Timestamp: {timestamp}")?;
    file.write_all(b"User: ")?;
    file.write_all(&username)?;
    writeln!(file)?;
    writeln!(file, "PID: {}", std::process::id())?;
    writeln!(file, "Codex invocation: {}", output.is_codex)?;
    file.write_all(b"Bwrap: ")?;
    file.write_all(real_bwrap.as_bytes())?;
    writeln!(file)?;
    file.write_all(b"Sandbox CWD: ")?;
    if let Some(workspace) = &output.workspace {
        file.write_all(workspace.as_bytes())?;
    } else {
        file.write_all(b"(not specified)")?;
    }
    writeln!(file)?;
    writeln!(file, "Tmpfs inserted: {}", output.home_inserted)?;
    file.write_all(b"Original args: ")?;
    write_args(&mut file, original)?;
    file.write_all(b"\nModified args: ")?;
    write_args(&mut file, &output.args)?;
    file.write_all(b"\n\nEnvironment:\n")?;
    for (key, value) in environment {
        file.write_all(key.as_bytes())?;
        file.write_all(b"=")?;
        file.write_all(value.as_bytes())?;
        file.write_all(b"\n")?;
    }
    file.write_all(b"=========== BWRAP END ===========\n")
}

fn write_args(file: &mut impl Write, args: &[OsString]) -> io::Result<()> {
    for (index, arg) in args.iter().enumerate() {
        if index != 0 {
            file.write_all(b" ")?;
        }
        let bytes = arg.as_bytes();
        if !bytes.is_empty() && bytes.iter().all(|byte| safe_byte(*byte)) {
            file.write_all(bytes)?;
        } else {
            file.write_all(b"'")?;
            for byte in bytes {
                if *byte == b'\'' {
                    file.write_all(b"'\"'\"'")?;
                } else {
                    file.write_all(&[*byte])?;
                }
            }
            file.write_all(b"'")?;
        }
    }
    Ok(())
}

fn safe_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte)
}

fn local_timestamp() -> io::Result<String> {
    let mut now: libc::time_t = 0;
    // SAFETY: pointers refer to writable, correctly sized libc values for the duration of each call.
    unsafe {
        if libc::time(&mut now) == -1 {
            return Err(io::Error::last_os_error());
        }
        let mut local: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut local).is_null() {
            return Err(io::Error::last_os_error());
        }
        let format = b"%Y-%m-%dT%H:%M:%S%:z\0";
        let mut buffer = [0u8; 128];
        let count = libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            format.as_ptr().cast(),
            &local,
        );
        if count == 0 {
            return Err(io::Error::other("could not format local timestamp"));
        }
        String::from_utf8(buffer[..count].to_vec())
            .map_err(|_| io::Error::other("local timestamp was not UTF-8"))
    }
}
