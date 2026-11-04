//! Linux process integration: environment discovery, logging, then `execv`.
use crate::{config, logging, mounts, transform};
use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;

struct Settings {
    home: OsString,
    log_file: OsString,
    config_home: Option<OsString>,
}

impl Settings {
    fn from_values(
        home: Option<OsString>,
        log_file: Option<OsString>,
        config_home: Option<OsString>,
    ) -> Result<Self, String> {
        let home = home.ok_or("HOME environment variable is required")?;
        let log_file = log_file.unwrap_or_else(|| {
            let mut value = home.as_bytes().to_vec();
            value.extend_from_slice(b"/.local/state/bwrap-wrapper.log");
            OsString::from_vec(value)
        });
        Ok(Self {
            home,
            log_file,
            config_home,
        })
    }

    fn from_environment() -> Result<Self, String> {
        Self::from_values(
            std::env::var_os("HOME"),
            std::env::var_os("BWRAP_LOG"),
            std::env::var_os("XDG_CONFIG_HOME"),
        )
    }
}

pub fn run() -> Result<(), String> {
    // SAFETY: umask is process-wide and intentionally set before any filesystem writes.
    unsafe {
        libc::umask(0o077);
    }
    let settings = Settings::from_environment()?;
    let original: Vec<OsString> = std::env::args_os().skip(1).collect();
    run_with_mount_reader(&original, mounts::read_proc_mounts, settings)
}

/// Run the same integration with an explicit mount-table reader. No process-global
/// test overrides or extra configuration variables are added to the executable.
pub fn run_with_args(
    original: &[OsString],
    mount_reader: impl FnMut() -> io::Result<Vec<OsString>>,
) -> Result<(), String> {
    // SAFETY: this is an alternative process entry point, called before filesystem writes.
    unsafe {
        libc::umask(0o077);
    }
    run_with_mount_reader(original, mount_reader, Settings::from_environment()?)
}

fn run_with_mount_reader(
    original: &[OsString],
    mount_reader: impl FnMut() -> io::Result<Vec<OsString>>,
    settings: Settings,
) -> Result<(), String> {
    let log = logging::InvocationLog::prepare(&settings.log_file)
        .map_err(|e| format!("failed to prepare invocation log: {e}"))?;
    let output = transform::transform_with_discovery(
        original,
        &settings.home,
        || config::discover(&settings.home, settings.config_home.as_deref()),
        mount_reader,
    )
    .map_err(|e| e.to_string())?;

    let real_bwrap = find_bwrap()?;
    let mut environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    environment.sort_by_cached_key(|(key, _)| config::python_name_sort_key(key));
    log.append(&real_bwrap, original, &output, &environment)
        .map_err(|e| format!("failed to write invocation log: {e}"))?;
    execv(&real_bwrap, &output.args)
        .map_err(|e| format!("failed to execute {}: {e}", real_bwrap.to_string_lossy()))
}

/// Follow links and compare file identity so aliases of this executable cannot recurse.
fn find_bwrap() -> Result<OsString, String> {
    let own = std::fs::metadata("/proc/self/exe")
        .map_err(|e| format!("failed to identify current executable: {e}"))?;
    let path = std::env::var_os("PATH").ok_or("PATH is not set; cannot find bwrap")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join("bwrap");
        let Ok(metadata) = std::fs::metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() || (metadata.dev(), metadata.ino()) == (own.dev(), own.ino()) {
            continue;
        }
        let Ok(c_path) = CString::new(candidate.as_os_str().as_bytes()) else {
            continue;
        };
        // SAFETY: c_path is a valid, live NUL-terminated path.
        if unsafe { libc::access(c_path.as_ptr(), libc::X_OK) } == 0 {
            return Ok(candidate.into_os_string());
        }
    }
    Err("no executable bwrap other than this wrapper found in PATH".into())
}

fn execv(path: &OsStr, args: &[OsString]) -> io::Result<()> {
    let path = CString::new(path.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "executable path contains NUL"))?;
    let mut strings = Vec::with_capacity(args.len() + 1);
    strings.push(path.clone());
    for arg in args {
        strings.push(
            CString::new(arg.as_bytes()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "argument contains NUL")
            })?,
        );
    }
    let mut pointers: Vec<*const libc::c_char> = strings.iter().map(|s| s.as_ptr()).collect();
    pointers.push(std::ptr::null());
    // SAFETY: C strings and the null-terminated argv pointer array remain alive until execv replaces this process.
    unsafe {
        libc::execv(path.as_ptr(), pointers.as_ptr());
    }
    Err(io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn environment_defaults_and_empty_values_match_reference() {
        let settings = Settings::from_values(Some("/home/u".into()), None, None).unwrap();
        assert_eq!(settings.log_file, "/home/u/.local/state/bwrap-wrapper.log");
        assert_eq!(settings.config_home, None);

        let settings =
            Settings::from_values(Some("/home/u/".into()), Some("".into()), Some("".into()))
                .unwrap();
        assert_eq!(settings.log_file, "");
        assert_eq!(
            settings.config_home.as_deref(),
            Some(std::ffi::OsStr::new(""))
        );
    }
}
