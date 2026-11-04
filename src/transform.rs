//! Pure translation of the reference's argument scan and rewrite loop.
use crate::paths::{join, normalize};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

/// Discovery results supplied by the caller. Configuration files must be in name order;
/// Windows mounts must already be deduplicated and ordered (see `mounts::parse`).
pub struct Inputs<'a> {
    pub home: &'a OsStr,
    pub config_files: &'a [OsString],
    pub windows_mounts: &'a [OsString],
}

#[derive(Debug, PartialEq, Eq)]
pub struct Output {
    pub args: Vec<OsString>,
    pub is_codex: bool,
    pub workspace: Option<OsString>,
    pub home_inserted: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RelativeWorkspace;

impl fmt::Display for RelativeWorkspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Error: --sandbox-policy-cwd must be absolute")
    }
}
impl std::error::Error for RelativeWorkspace {}

pub fn is_codex(args: &[OsString]) -> bool {
    let Some(separator) = args.iter().position(|a| a == "--") else {
        return false;
    };
    args[..separator]
        .windows(2)
        .any(|p| p[0] == "--argv0" && p[1] == "codex-linux-sandbox")
        && args.get(separator + 1).is_some_and(|command| {
            let bytes = command.as_bytes();
            bytes.contains(&b'/') && bytes.rsplit(|b| *b == b'/').next() == Some(b"codex")
        })
}

/// Scan the entire list, including command arguments, stopping at the first occurrence.
pub fn workspace(args: &[OsString]) -> Result<Option<OsString>, RelativeWorkspace> {
    let mut value = None;
    for (i, arg) in args.iter().enumerate() {
        if arg == "--sandbox-policy-cwd" {
            value = args.get(i + 1).cloned();
            break;
        }
        if let Some(suffix) = arg.as_bytes().strip_prefix(b"--sandbox-policy-cwd=") {
            value = Some(OsString::from_vec(suffix.to_vec()));
            break;
        }
    }
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let normalized = normalize(&value, true);
    if !normalized.as_bytes().starts_with(b"/") {
        return Err(RelativeWorkspace);
    }
    Ok(Some(normalized))
}

pub fn transform(args: &[OsString], inputs: &Inputs<'_>) -> Result<Output, RelativeWorkspace> {
    let mut out = Output {
        args: Vec::new(),
        is_codex: is_codex(args),
        workspace: None,
        home_inserted: false,
    };
    if !out.is_codex {
        out.args = args.to_vec();
        return Ok(out);
    }
    out.workspace = workspace(args)?;
    let git = out
        .workspace
        .as_deref()
        .map(|w| join(w, OsStr::new(".git")));
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            for mount in inputs.windows_mounts {
                out.args.extend(["--tmpfs".into(), mount.clone()]);
            }
            out.args.extend_from_slice(&args[i..]);
            break;
        }
        if arg == "--ro-bind"
            && i + 2 < args.len()
            && git
                .as_ref()
                .is_some_and(|g| args[i + 1] == *g && args[i + 2] == *g)
        {
            i += 3;
            continue;
        }
        if arg == "--dev" && args.get(i + 1).is_some_and(|a| a == "/dev") {
            out.args
                .extend(["--dev-bind".into(), "/dev".into(), "/dev".into()]);
            i += 2;
            continue;
        }
        if arg == "--bind" && i + 2 < args.len() {
            let matching = out
                .workspace
                .as_ref()
                .is_some_and(|w| args[i + 1] == *w && args[i + 2] == *w);
            if matching && !out.home_inserted {
                out.args.extend([
                    "--tmpfs".into(),
                    inputs.home.to_owned(),
                    "--dir".into(),
                    join(inputs.home, OsStr::new(".codex")),
                ]);
                for (option, suffix) in [
                    ("--ro-bind", ".codex/packages"),
                    ("--bind", ".codex/memories"),
                    ("--ro-bind", ".codex/AGENTS.md"),
                ] {
                    let path = join(inputs.home, OsStr::new(suffix));
                    out.args.extend([option.into(), path.clone(), path]);
                }
                out.home_inserted = true;
            }
            out.args.extend_from_slice(&args[i..i + 3]);
            if matching {
                for source in inputs.config_files {
                    let name = source.as_bytes().rsplit(|b| *b == b'/').next().unwrap();
                    let target = join(out.workspace.as_deref().unwrap(), OsStr::from_bytes(name));
                    out.args
                        .extend(["--ro-bind".into(), source.clone(), target]);
                }
            }
            i += 3;
            continue;
        }
        out.args.push(arg.clone());
        i += 1;
    }
    Ok(out)
}
