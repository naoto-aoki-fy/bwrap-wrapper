//! Append-only invocation log compatible with the Python shim's shell quoting.
use crate::transform::Output;
use std::ffi::{CStr, OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::fd::IntoRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub struct InvocationLog {
    path: OsString,
    timestamp: String,
    username: Vec<u8>,
}

impl InvocationLog {
    /// Prepare directories and identity before workspace validation, as Python does.
    pub fn prepare(path: &OsStr) -> io::Result<Self> {
        let normalized = crate::paths::normalize(path, false);
        let parent = Path::new(&normalized)
            .parent()
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        })?;
        let timestamp = local_timestamp()?;
        // SAFETY: this process entry point runs on one thread. Copy passwd storage immediately.
        let username = unsafe {
            let record = libc::getpwuid(libc::getuid());
            if record.is_null() || (*record).pw_name.is_null() {
                return Err(io::Error::other("no username for current UID"));
            }
            CStr::from_ptr((*record).pw_name).to_bytes().to_vec()
        };
        Ok(Self {
            path: path.to_owned(),
            timestamp,
            username,
        })
    }

    pub fn append(
        &self,
        real_bwrap: &OsStr,
        original: &[OsString],
        output: &Output,
        environment: &[(OsString, OsString)],
    ) -> io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut file = BufWriter::new(file);
        let timestamp = &self.timestamp;
        writeln!(file, "\n========== BWRAP START ==========")?;
        writeln!(file, "Timestamp: {timestamp}")?;
        file.write_all(b"User: ")?;
        write_utf8(&mut file, &self.username)?;
        writeln!(file)?;
        writeln!(file, "PID: {}", std::process::id())?;
        writeln!(file, "Codex invocation: {}", python_bool(output.is_codex))?;
        file.write_all(b"Bwrap: ")?;
        write_utf8(&mut file, real_bwrap.as_bytes())?;
        writeln!(file)?;
        file.write_all(b"Sandbox CWD: ")?;
        if let Some(workspace) = &output.workspace {
            write_utf8(&mut file, workspace.as_bytes())?;
        } else {
            file.write_all(b"(not specified)")?;
        }
        writeln!(file)?;
        writeln!(
            file,
            "Tmpfs inserted: {}",
            python_bool(output.home_inserted)
        )?;
        file.write_all(b"Original args: ")?;
        write_args(&mut file, original)?;
        file.write_all(b"\nModified args: ")?;
        write_args(&mut file, &output.args)?;
        file.write_all(b"\n\nEnvironment:\n")?;
        for (key, value) in environment {
            write_utf8(&mut file, key.as_bytes())?;
            file.write_all(b"=")?;
            write_utf8(&mut file, value.as_bytes())?;
            file.write_all(b"\n")?;
        }
        file.write_all(b"=========== BWRAP END ===========\n")?;
        let file = file.into_inner().map_err(|error| error.into_error())?;
        let descriptor = file.into_raw_fd();
        // SAFETY: ownership was consumed above; close once and report delayed I/O errors.
        if unsafe { libc::close(descriptor) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn python_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn write_utf8(file: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    std::str::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invocation log contains non-UTF-8 data",
        )
    })?;
    file.write_all(bytes)
}

fn write_args(file: &mut impl Write, args: &[OsString]) -> io::Result<()> {
    for (index, arg) in args.iter().enumerate() {
        if index != 0 {
            file.write_all(b" ")?;
        }
        let bytes = arg.as_bytes();
        // Python decodes argv with surrogateescape, then its strict UTF-8 log writer fails.
        std::str::from_utf8(bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invocation log contains non-UTF-8 argument",
            )
        })?;
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
        let format = b"%Y-%m-%dT%H:%M:%S\0";
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
        let mut timestamp = String::from_utf8(buffer[..count].to_vec())
            .map_err(|_| io::Error::other("local timestamp was not UTF-8"))?;
        let offset = local.tm_gmtoff;
        let sign = if offset < 0 { '-' } else { '+' };
        let seconds = offset.unsigned_abs();
        timestamp.push_str(&format!(
            "{sign}{:02}:{:02}",
            seconds / 3600,
            (seconds / 60) % 60
        ));
        if seconds % 60 != 0 {
            timestamp.push_str(&format!(":{:02}", seconds % 60));
        }
        Ok(timestamp)
    }
}
