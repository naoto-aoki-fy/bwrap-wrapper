//! Unix Python path operations, without resolving symlinks or losing argument bytes.
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};

pub(crate) fn join(base: &OsStr, suffix: &OsStr) -> OsString {
    let mut bytes = base.as_bytes().to_vec();
    if suffix.as_bytes().starts_with(b"/") {
        return suffix.to_owned();
    }
    if !bytes.is_empty() && !bytes.ends_with(b"/") {
        bytes.push(b'/');
    }
    bytes.extend_from_slice(suffix.as_bytes());
    OsString::from_vec(bytes)
}

// normpath resolves '..'; pathlib's construction of config paths leaves it intact.
pub(crate) fn normalize(path: &OsStr, resolve_parents: bool) -> OsString {
    let bytes = path.as_bytes();
    let root = if bytes.starts_with(b"//") && !bytes.starts_with(b"///") {
        2
    } else {
        usize::from(bytes.starts_with(b"/"))
    };
    let mut parts: Vec<&[u8]> = Vec::new();
    for part in bytes.split(|b| *b == b'/') {
        if part.is_empty() || part == b"." {
            continue;
        }
        if resolve_parents && part == b".." {
            if parts.last().is_some_and(|p| *p != b"..") {
                parts.pop();
                continue;
            }
            if root != 0 {
                continue;
            }
        }
        parts.push(part);
    }
    let mut result = vec![b'/'; root];
    for part in parts {
        if !result.is_empty() && !result.ends_with(b"/") {
            result.push(b'/');
        }
        result.extend_from_slice(part);
    }
    if result.is_empty() {
        result.push(b'.');
    }
    OsString::from_vec(result)
}
