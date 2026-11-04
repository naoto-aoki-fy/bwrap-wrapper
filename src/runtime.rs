//! Linux process integration: environment discovery, logging, then `execv`.
use crate::{config, logging, mounts, transform};
use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

struct Settings {
    home: OsString,
    real_bwrap: OsString,
    log_file: OsString,
    config_home: Option<OsString>,
}

impl Settings {
    fn from_values(
        home: Option<OsString>,
        real_bwrap: Option<OsString>,
        log_file: Option<OsString>,
        config_home: Option<OsString>,
    ) -> Result<Self, String> {
        let home = home.ok_or("HOME environment variable is required")?;
        let real_bwrap = real_bwrap.unwrap_or_else(|| "/usr/bin/bwrap".into());
        let log_file = log_file.unwrap_or_else(|| {
            let mut value = home.as_bytes().to_vec();
            value.extend_from_slice(b"/.local/state/bwrap-wrapper.log");
            OsString::from_vec(value)
        });
        Ok(Self {
            home,
            real_bwrap,
            log_file,
            config_home,
        })
    }

    fn from_environment() -> Result<Self, String> {
        Self::from_values(
            std::env::var_os("HOME"),
            std::env::var_os("BWRAP_REAL"),
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
    let recognized = transform::is_codex(&original);

    let workspace = if recognized {
        transform::workspace(&original).map_err(|e| e.to_string())?
    } else {
        None
    };
    let config_files = if recognized && has_workspace_bind(&original, workspace.as_deref()) {
        let cwd = std::env::current_dir()
            .map_err(|e| format!("failed to read current directory: {e}"))?;
        let directory = config::directory(
            &settings.home,
            settings.config_home.as_deref(),
            cwd.as_os_str(),
        );
        config::files(&directory).map_err(|e| format!("failed to read configuration files: {e}"))?
    } else {
        Vec::new()
    };
    let windows_mounts = if recognized && reaches_command_separator(&original, workspace.as_deref())
    {
        mounts::read_proc_mounts().map_err(|e| format!("failed to read /proc/mounts: {e}"))?
    } else {
        Vec::new()
    };
    let output = transform::transform(
        &original,
        &transform::Inputs {
            home: &settings.home,
            config_files: &config_files,
            windows_mounts: &windows_mounts,
        },
    )
    .map_err(|e| e.to_string())?;

    let mut environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    environment.sort_by_cached_key(|(key, _)| config::python_name_sort_key(key));
    logging::append(
        &settings.log_file,
        &settings.real_bwrap,
        &original,
        &output,
        &environment,
    )
    .map_err(|e| format!("failed to write invocation log: {e}"))?;
    execv(&settings.real_bwrap, &output.args).map_err(|e| {
        format!(
            "failed to execute {}: {e}",
            settings.real_bwrap.to_string_lossy()
        )
    })
}

fn reaches_command_separator(args: &[OsString], workspace: Option<&OsStr>) -> bool {
    let git = workspace.map(|path| crate::paths::join(path, OsStr::new(".git")));
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            return true;
        }
        if arg == "--ro-bind"
            && i + 2 < args.len()
            && git
                .as_ref()
                .is_some_and(|path| args[i + 1] == *path && args[i + 2] == *path)
        {
            i += 3;
        } else if arg == "--dev" && args.get(i + 1).is_some_and(|a| a == "/dev") {
            i += 2;
        } else if arg == "--bind" && i + 2 < args.len() {
            i += 3;
        } else {
            i += 1;
        }
    }
    false
}

fn has_workspace_bind(args: &[OsString], workspace: Option<&OsStr>) -> bool {
    let Some(workspace) = workspace else {
        return false;
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            return false;
        }
        if arg == "--bind" && i + 2 < args.len() {
            if args[i + 1] == workspace && args[i + 2] == workspace {
                return true;
            }
            i += 3;
        } else if arg == "--dev" && args.get(i + 1).is_some_and(|a| a == "/dev") {
            i += 2;
        } else if arg == "--ro-bind" && i + 2 < args.len() {
            let git = crate::paths::join(workspace, OsStr::new(".git"));
            if args[i + 1] == git && args[i + 2] == git {
                i += 3;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    false
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
    use super::{Settings, reaches_command_separator};
    use std::ffi::OsString;

    #[test]
    fn environment_defaults_and_empty_values_match_reference() {
        let settings = Settings::from_values(Some("/home/u".into()), None, None, None).unwrap();
        assert_eq!(settings.real_bwrap, "/usr/bin/bwrap");
        assert_eq!(settings.log_file, "/home/u/.local/state/bwrap-wrapper.log");
        assert_eq!(settings.config_home, None);

        let settings = Settings::from_values(
            Some("/home/u/".into()),
            Some("".into()),
            Some("".into()),
            Some("".into()),
        )
        .unwrap();
        assert_eq!(settings.real_bwrap, "");
        assert_eq!(settings.log_file, "");
        assert_eq!(
            settings.config_home.as_deref(),
            Some(std::ffi::OsStr::new(""))
        );
    }

    #[test]
    fn mount_discovery_is_skipped_when_an_operand_consumes_separator() {
        let consumed = [
            "--argv0",
            "codex-linux-sandbox",
            "--bind",
            "source",
            "--",
            "/bin/codex",
        ]
        .map(OsString::from);
        assert!(!reaches_command_separator(&consumed, None));

        let ordinary = [
            "--argv0",
            "codex-linux-sandbox",
            "--dev",
            "/dev",
            "--",
            "/bin/codex",
        ]
        .map(OsString::from);
        assert!(reaches_command_separator(&ordinary, None));
    }
}
