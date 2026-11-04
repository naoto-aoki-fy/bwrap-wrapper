//! Reference-compatible rewriting with support for current Codex sandbox helpers.
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
    sandbox_helper_args(args).is_some()
        || args[..separator]
            .windows(2)
            .any(|p| p[0] == "--argv0" && p[1] == "codex-linux-sandbox")
            && args.get(separator + 1).is_some_and(|command| {
                let bytes = command.as_bytes();
                bytes.contains(&b'/') && bytes.rsplit(|b| *b == b'/').next() == Some(b"codex")
            })
}

/// Return only the direct helper options, excluding the nested command tail.
fn sandbox_helper_args(args: &[OsString]) -> Option<&[OsString]> {
    let separator = args.iter().position(|a| a == "--")?;
    // A dangling argv0 consumes the separator as its value, not as a command boundary.
    if separator > 0 && args[separator - 1] == "--argv0" {
        return None;
    }
    let command = args.get(separator + 1)?;
    if command.as_bytes().rsplit(|b| *b == b'/').next() != Some(b"codex-linux-sandbox") {
        return None;
    }
    let tail = &args[separator + 2..];
    let end = tail.iter().position(|a| a == "--").unwrap_or(tail.len());
    Some(&tail[..end])
}

/// Legacy calls scan the entire list; current helpers scan only their own options.
pub fn workspace(args: &[OsString]) -> Result<Option<OsString>, RelativeWorkspace> {
    let args = sandbox_helper_args(args).unwrap_or(args);
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

#[derive(Debug)]
pub enum DiscoveryError<E> {
    Workspace(RelativeWorkspace),
    Discovery(E),
}

impl<E: fmt::Display> fmt::Display for DiscoveryError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => error.fmt(f),
            Self::Discovery(error) => error.fmt(f),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for DiscoveryError<E> {}

pub fn transform(args: &[OsString], inputs: &Inputs<'_>) -> Result<Output, RelativeWorkspace> {
    match transform_with_discovery(
        args,
        inputs.home,
        || Ok::<_, std::convert::Infallible>(inputs.config_files.to_vec()),
        || Ok::<_, std::convert::Infallible>(inputs.windows_mounts.to_vec()),
    ) {
        Ok(output) => Ok(output),
        Err(DiscoveryError::Workspace(error)) => Err(error),
        Err(DiscoveryError::Discovery(never)) => match never {},
    }
}

/// Invoke discovery at the same points as Python: after each matching bind and
/// when the rewrite loop reaches the command separator. Callbacks can be pure fixtures.
pub fn transform_with_discovery<E>(
    args: &[OsString],
    home: &OsStr,
    mut config_files: impl FnMut() -> Result<Vec<OsString>, E>,
    mut windows_mounts: impl FnMut() -> Result<Vec<OsString>, E>,
) -> Result<Output, DiscoveryError<E>> {
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
    out.workspace = workspace(args).map_err(DiscoveryError::Workspace)?;
    let git = out
        .workspace
        .as_deref()
        .map(|w| join(w, OsStr::new(".git")));
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            for mount in windows_mounts().map_err(DiscoveryError::Discovery)? {
                out.args.extend(["--tmpfs".into(), mount]);
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
                    home.to_owned(),
                    "--dir".into(),
                    join(home, OsStr::new(".codex")),
                ]);
                for (option, suffix) in [
                    ("--ro-bind", ".codex/packages"),
                    ("--bind", ".codex/memories"),
                    ("--ro-bind", ".codex/AGENTS.md"),
                ] {
                    let path = join(home, OsStr::new(suffix));
                    out.args.extend([option.into(), path.clone(), path]);
                }
                out.home_inserted = true;
            }
            out.args.extend_from_slice(&args[i..i + 3]);
            if matching {
                for source in config_files().map_err(DiscoveryError::Discovery)? {
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
