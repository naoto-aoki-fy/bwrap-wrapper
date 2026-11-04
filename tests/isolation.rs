//! Opt-in real namespace tests; never substitute a fake Bubblewrap executable.
use bwrap_wrapper_rs::transform::{self, Inputs};
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn push(args: &mut Vec<OsString>, values: &[&OsStr]) {
    args.extend(values.iter().map(|v| v.to_os_string()));
}
fn run_case(nested: bool, legacy: bool) {
    let root = std::env::temp_dir().join(format!(
        "bwrap-isolation-{}-{nested}-{legacy}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    let _cleanup = Fixture(root.clone());
    let home = root.join("home");
    let workspace = if nested {
        home.join("projects/work")
    } else {
        root.join("work")
    };
    for path in [
        home.join(".codex/packages"),
        home.join(".codex/memories"),
        home.join(".codex/shell_snapshots"),
        workspace.join(".git"),
        root.join("other"),
    ] {
        std::fs::create_dir_all(path).unwrap();
    }
    for (path, contents) in [
        (home.join("private"), "private"),
        (home.join(".codex/AGENTS.md"), "agents"),
        (home.join(".codex/packages/data"), "package"),
        (home.join(".codex/memories/data"), "memory"),
        (home.join(".codex/shell_snapshots/snapshot.sh"), "true"),
        (workspace.join(".git/config"), "git"),
        (workspace.join("overlay"), "original"),
        (root.join("config"), "overlay"),
        (root.join("other/data"), "other"),
    ] {
        std::fs::write(path, contents).unwrap();
    }
    let helper = home.join(".codex/packages").join(if legacy {
        "codex"
    } else {
        "codex-linux-sandbox"
    });
    std::fs::write(&helper, "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = -- ]; then shift; exec \"$@\"; fi; shift; done\nexit 99\n").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut args: Vec<OsString> = ["--ro-bind", "/", "/", "--dev", "/dev"]
        .map(Into::into)
        .to_vec();
    let other = root.join("other");
    push(
        &mut args,
        &[
            OsStr::new("--ro-bind"),
            other.as_os_str(),
            other.as_os_str(),
        ],
    );
    push(
        &mut args,
        &[
            OsStr::new("--bind"),
            workspace.as_os_str(),
            workspace.as_os_str(),
        ],
    );
    let git = workspace.join(".git");
    push(
        &mut args,
        &[OsStr::new("--ro-bind"), git.as_os_str(), git.as_os_str()],
    );
    if legacy {
        args.extend(["--argv0".into(), "codex-linux-sandbox".into()]);
    }
    push(
        &mut args,
        &[
            OsStr::new("--"),
            helper.as_os_str(),
            OsStr::new("--sandbox-policy-cwd"),
            workspace.as_os_str(),
            OsStr::new("--"),
            OsStr::new("/bin/sh"),
            OsStr::new("-eu"),
            OsStr::new("-c"),
        ],
    );
    args.push(r#"
fail() { echo "$*" >&2; exit 1; }
[ ! -e "$HOME/private" ] || fail private-visible
[ ! -e "$HOME/.codex/shell_snapshots/snapshot.sh" ] || fail snapshot-visible
[ "$(stat -f -c %T "$HOME")" = tmpfs ] || fail home-not-tmpfs
[ "$(cat "$HOME/.codex/packages/data")" = package ]
[ "$(cat "$HOME/.codex/AGENTS.md")" = agents ]
[ "$(cat "$HOME/.codex/memories/data")" = memory ]
printf updated > "$HOME/.codex/memories/data"
if (printf bad > "$HOME/.codex/packages/data") 2>/dev/null; then fail package-writable; fi
if (printf bad > "$HOME/.codex/AGENTS.md") 2>/dev/null; then fail agents-writable; fi
printf workspace > "$WORKSPACE/new"
[ "$(cat "$WORKSPACE/.git/config")" = git ]
printf updated > "$WORKSPACE/.git/config"
[ "$(cat "$WORKSPACE/overlay")" = overlay ]
if (printf bad > "$WORKSPACE/overlay") 2>/dev/null; then fail overlay-writable; fi
[ "$(cat "$OTHER/data")" = other ]
if (printf bad > "$OTHER/data") 2>/dev/null; then fail unrelated-mount-writable; fi
if /bin/sh -c '. "$HOME/.codex/shell_snapshots/snapshot.sh"' 2>/dev/null; then fail hidden-snapshot-loaded; fi
echo isolation-ok
"#.into());
    let config = root.join("config");
    // The overlay destination uses the source basename.
    let config_dir = root.join("config-files");
    std::fs::create_dir(&config_dir).unwrap();
    let overlay = config_dir.join("overlay");
    std::fs::copy(config, &overlay).unwrap();
    let output = transform::transform(
        &args,
        &Inputs {
            home: home.as_os_str(),
            config_files: &[overlay.into_os_string()],
            windows_mounts: &[],
        },
    )
    .unwrap();
    assert!(output.is_codex && output.home_inserted);
    assert_eq!(output.workspace.as_deref(), Some(workspace.as_os_str()));
    let bwrap = std::env::var_os("BWRAP_ISOLATION_REAL").unwrap_or_else(|| "/usr/bin/bwrap".into());
    assert!(
        Path::new(&bwrap).is_absolute(),
        "use the absolute real Bubblewrap path"
    );
    // The observed current runtime launches its helper from .codex/tmp.
    // Demonstrate that this path is hidden, rather than expanding the allowlist.
    let hidden_helper = home.join(".codex/tmp/arg0-linux/example/codex-linux-sandbox");
    std::fs::create_dir_all(hidden_helper.parent().unwrap()).unwrap();
    std::fs::copy(&helper, &hidden_helper).unwrap();
    let mut hidden_args = output.args.clone();
    let separator = hidden_args.iter().position(|a| a == "--").unwrap();
    hidden_args[separator + 1] = hidden_helper.into_os_string();
    let hidden_result = Command::new(&bwrap)
        .args(hidden_args)
        .env_clear()
        .output()
        .unwrap();
    assert!(!hidden_result.status.success());
    assert!(
        String::from_utf8_lossy(&hidden_result.stderr).contains("No such file or directory"),
        "{hidden_result:?}"
    );
    let result = Command::new(bwrap)
        .args(output.args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &home)
        .env("WORKSPACE", &workspace)
        .env("OTHER", &other)
        .output()
        .expect("real Bubblewrap unavailable");
    assert!(
        result.status.success(),
        "nested={nested} legacy={legacy}: {result:?}"
    );
    assert_eq!(result.stdout, b"isolation-ok\n");
    assert_eq!(std::fs::read(workspace.join("new")).unwrap(), b"workspace");
    assert_eq!(std::fs::read(git.join("config")).unwrap(), b"updated");
    assert_eq!(
        std::fs::read(home.join(".codex/memories/data")).unwrap(),
        b"updated"
    );
    assert_eq!(std::fs::read(home.join("private")).unwrap(), b"private");
}

#[test]
#[ignore = "requires real Bubblewrap and working mount namespaces; see docs/isolation.md"]
fn real_bubblewrap_mount_isolation() {
    for nested in [false, true] {
        for legacy in [false, true] {
            run_case(nested, legacy);
        }
    }
}
