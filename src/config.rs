//! Configuration discovery, separate from argument transformation. No environment mutation.
use crate::paths::{join, normalize};
use serde::Deserialize;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub logging: Logging,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Logging {
    pub enabled: bool,
}

/// `cwd` is the caller's absolute working directory, used like Path.absolute().
/// Relative XDG_CONFIG_HOME values retain '..' and do not resolve symlinks.
fn root(home: &OsStr, xdg_config_home: Option<&OsStr>, cwd: &OsStr) -> OsString {
    let base = xdg_config_home
        .filter(|s| !s.is_empty())
        .map(OsStr::to_owned)
        .unwrap_or_else(|| join(home, OsStr::new(".config")));
    let absolute = if base.as_bytes().starts_with(b"/") {
        base
    } else {
        join(cwd, &base)
    };
    normalize(&join(&absolute, OsStr::new("bwrap-wrapper")), false)
}

pub fn directory(home: &OsStr, xdg_config_home: Option<&OsStr>, cwd: &OsStr) -> OsString {
    join(&root(home, xdg_config_home, cwd), OsStr::new("cwd"))
}

pub fn settings_path(home: &OsStr, xdg_config_home: Option<&OsStr>, cwd: &OsStr) -> OsString {
    join(&root(home, xdg_config_home, cwd), OsStr::new("config.toml"))
}

/// Load optional settings before preparing any invocation-log artifacts.
pub fn load(home: &OsStr, xdg_config_home: Option<&OsStr>) -> Result<Settings, String> {
    let cwd = discovery_cwd(home, xdg_config_home).map_err(|e| {
        let path = settings_path(home, xdg_config_home, OsStr::new("."));
        format!(
            "failed to resolve configuration file {}: {e}",
            path.to_string_lossy()
        )
    })?;
    let path = settings_path(home, xdg_config_home, &cwd);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(error) => {
            return Err(format!(
                "failed to read configuration file {}: {error}",
                path.to_string_lossy()
            ));
        }
    };
    toml::from_str(&contents).map_err(|error| {
        format!(
            "invalid configuration file {}: {error}",
            path.to_string_lossy()
        )
    })
}

/// Includes symlinks to regular files, skips broken links and directories, and includes dotfiles.
/// Like the reference, missing/non-directory paths yield no mounts, while listing errors propagate.
pub fn files(directory: &OsStr) -> io::Result<Vec<OsString>> {
    let path = Path::new(directory);
    if !metadata_matches(path, std::fs::Metadata::is_dir)? {
        return Ok(Vec::new());
    }
    let mut entries = std::fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| python_name_sort_key(&entry.file_name()));
    let mut files = Vec::new();
    for entry in entries {
        if metadata_matches(&entry.path(), std::fs::Metadata::is_file)? {
            files.push(entry.path().into_os_string());
        }
    }
    Ok(files)
}

/// Preserve the audited Python pathlib's distinction between missing paths and I/O failures.
fn metadata_matches(path: &Path, matches: fn(&std::fs::Metadata) -> bool) -> io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(matches(&metadata)),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOENT | libc::ENOTDIR | libc::EBADF | libc::ELOOP)
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Discover at each matching bind; absolute config homes do not require a live cwd.
pub fn discover(home: &OsStr, xdg_config_home: Option<&OsStr>) -> io::Result<Vec<OsString>> {
    let cwd = discovery_cwd(home, xdg_config_home)?;
    files(&directory(home, xdg_config_home, &cwd))
}

fn discovery_cwd(home: &OsStr, xdg_config_home: Option<&OsStr>) -> io::Result<OsString> {
    let base = xdg_config_home
        .filter(|s| !s.is_empty())
        .map(OsStr::to_owned)
        .unwrap_or_else(|| join(home, OsStr::new(".config")));
    if base.as_bytes().starts_with(b"/") {
        Ok(OsString::from("/"))
    } else {
        Ok(std::env::current_dir()?.into_os_string())
    }
}

// Python sorts filesystem names after decoding UTF-8 with surrogateescape.
// Preserve that order even when invalid bytes occur alongside non-BMP characters.
pub(crate) fn python_name_sort_key(name: &OsStr) -> Vec<u32> {
    let mut bytes = name.as_bytes();
    let mut key = Vec::new();
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                key.extend(text.chars().map(u32::from));
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                key.extend(
                    std::str::from_utf8(&bytes[..valid])
                        .unwrap()
                        .chars()
                        .map(u32::from),
                );
                let invalid = error.error_len().unwrap_or(bytes.len() - valid);
                key.extend(
                    bytes[valid..valid + invalid]
                        .iter()
                        .map(|b| 0xdc00 + u32::from(*b)),
                );
                bytes = &bytes[valid + invalid..];
            }
        }
    }
    key
}
