//! Configuration discovery, separate from argument transformation. No environment mutation.
use crate::paths::{join, normalize};
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// `cwd` is the caller's absolute working directory, used like Path.absolute().
/// Relative XDG_CONFIG_HOME values retain '..' and do not resolve symlinks.
pub fn directory(home: &OsStr, xdg_config_home: Option<&OsStr>, cwd: &OsStr) -> OsString {
    let base = xdg_config_home
        .filter(|s| !s.is_empty())
        .map(OsStr::to_owned)
        .unwrap_or_else(|| join(home, OsStr::new(".config")));
    let absolute = if base.as_bytes().starts_with(b"/") {
        base
    } else {
        join(cwd, &base)
    };
    normalize(&join(&absolute, OsStr::new("bwrap-wrapper/cwd")), false)
}

/// Includes symlinks to regular files, skips broken links and directories, and includes dotfiles.
/// Like the reference, missing/non-directory paths yield no mounts, while listing errors propagate.
pub fn files(directory: &OsStr) -> io::Result<Vec<OsString>> {
    let path = Path::new(directory);
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = std::fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| python_name_key(&entry.file_name()));
    Ok(entries
        .into_iter()
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.path().into_os_string())
        .collect())
}

// Python sorts filesystem names after decoding UTF-8 with surrogateescape.
// Preserve that order even when invalid bytes occur alongside non-BMP characters.
fn python_name_key(name: &OsStr) -> Vec<u32> {
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
