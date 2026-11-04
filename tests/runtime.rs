use bwrap_wrapper_rs::{config, mounts, runtime, transform};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "bwrap-wrapper-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    temp: TempDir,
    fake: std::path::PathBuf,
    log: std::path::PathBuf,
    result: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new();
        let fake = temp.path("bwrap");
        std::fs::write(&fake, "#!/bin/sh\nprintf 'PID=%s\\n' \"$$\" > \"$FAKE_RESULT\"\nprintf 'ENV=%s\\n' \"$FAKE_SECRET\" >> \"$FAKE_RESULT\"\nprintf 'ARG=%s\\n' \"$@\" >> \"$FAKE_RESULT\"\nif [ -f \"$BWRAP_LOG\" ]; then printf 'LOG=present\\n' >> \"$FAKE_RESULT\"; fi\nexit \"$FAKE_EXIT\"\n").unwrap();
        let mut permissions = std::fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake, permissions).unwrap();
        Self {
            log: temp.path("state/wrapper.log"),
            result: temp.path("result"),
            fake,
            temp,
        }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bwrap"));
        command
            .env_clear()
            .env("PATH", &self.temp.0)
            .env("HOME", self.temp.path("home"))
            .env("BWRAP_LOG", &self.log)
            .env("FAKE_RESULT", &self.result)
            .env("FAKE_EXIT", "0")
            .env("FAKE_SECRET", "inherited-value")
            .args(args);
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(&self.result)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn codex_command(&self, args: &[&str]) -> Command {
        let args_path = self.temp.path("args");
        let mut encoded = Vec::new();
        for arg in args {
            encoded.extend_from_slice(arg.as_bytes());
            encoded.push(0);
        }
        std::fs::write(&args_path, encoded).unwrap();
        let mounts_path = self.temp.path("mounts");
        std::fs::write(
            &mounts_path,
            "x /mnt/d 9p rw;aname=drvfs\nx /c 9p aname=drvfs\nx /c 9p aname=drvfs\n",
        )
        .unwrap();
        let template = self.command(&[]);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.env_clear();
        for (key, value) in template.get_envs() {
            if let Some(value) = value {
                command.env(key, value);
            }
        }
        command
            .args(["--exact", "fixture_driver", "--nocapture"])
            .env("BWRAP_TEST_ARGS", args_path)
            .env("BWRAP_TEST_MOUNTS", mounts_path);
        command
    }
}

#[test]
fn non_codex_passthrough_logs_before_exec_and_inherits_environment_and_pid() {
    let fixture = Fixture::new();
    let args = ["--dev", "/dev", "plain argument", "it\'s quoted", ""];
    let mut command = fixture.command(&args);
    let child = command.spawn().unwrap();
    let pid = child.id();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(0));
    let lines = fixture.lines();
    assert_eq!(lines[0], format!("PID={pid}"));
    assert_eq!(lines[1], "ENV=inherited-value");
    assert_eq!(
        &lines[2..],
        [
            "ARG=--dev",
            "ARG=/dev",
            "ARG=plain argument",
            "ARG=it's quoted",
            "ARG=",
            "LOG=present"
        ]
    );

    let log = std::fs::read_to_string(&fixture.log).unwrap();
    assert_eq!(
        std::fs::metadata(&fixture.log)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(log.starts_with("\n========== BWRAP START ==========\nTimestamp: "));
    assert!(log.contains("Codex invocation: False\n"));
    assert!(log.contains("Sandbox CWD: (not specified)\n"));
    assert!(log.contains("Original args: --dev /dev 'plain argument' 'it'\"'\"'s quoted' ''\n"));
    assert!(log.contains("FAKE_SECRET=inherited-value\n"));
    assert!(log.find("BWRAP_LOG=").unwrap() < log.find("FAKE_RESULT=").unwrap());
}

#[test]
fn existing_log_permissions_are_preserved() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.log.parent().unwrap()).unwrap();
    std::fs::write(&fixture.log, "existing\n").unwrap();
    let mut permissions = std::fs::metadata(&fixture.log).unwrap().permissions();
    permissions.set_mode(0o640);
    std::fs::set_permissions(&fixture.log, permissions).unwrap();
    assert!(fixture.run(&["--", "/bin/true"]).status.success());
    assert_eq!(
        std::fs::metadata(&fixture.log)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
}

#[test]
fn codex_arguments_are_transformed_and_exit_status_propagates() {
    let fixture = Fixture::new();
    let xdg = fixture.temp.path("xdg");
    let config_dir = xdg.join("bwrap-wrapper/cwd");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join(".overlay"), "overlay").unwrap();
    std::fs::create_dir(config_dir.join("directory")).unwrap();
    let args = [
        "--argv0",
        "codex-linux-sandbox",
        "--sandbox-policy-cwd",
        "/work/./",
        "--dev",
        "/dev",
        "--ro-bind",
        "/work/.git",
        "/work/.git",
        "--bind",
        "/work",
        "/work",
        "--",
        "/opt/codex",
        "tail arg",
    ];
    let mut command = fixture.codex_command(&args);
    command.env("XDG_CONFIG_HOME", &xdg);
    command.env("FAKE_EXIT", "23");
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(23));
    let lines = fixture.lines();
    let home = fixture.temp.path("home");
    let cwd = std::env::current_dir().unwrap();
    let config_dir = config::directory(home.as_os_str(), Some(xdg.as_os_str()), cwd.as_os_str());
    let config_files = config::files(&config_dir).unwrap();
    let windows_mounts = mounts::read_file(fixture.temp.path("mounts")).unwrap();
    let original = args.map(OsString::from);
    let expected = transform::transform(
        &original,
        &transform::Inputs {
            home: home.as_os_str(),
            config_files: &config_files,
            windows_mounts: &windows_mounts,
        },
    )
    .unwrap()
    .args
    .iter()
    .map(|arg| format!("ARG={}", arg.to_string_lossy()))
    .collect::<Vec<_>>();
    assert_eq!(&lines[2..lines.len() - 1], expected);
    assert_eq!(lines.last().unwrap(), "LOG=present");
    let log = std::fs::read_to_string(&fixture.log).unwrap();
    assert!(log.contains("Codex invocation: True\n"));
    assert!(log.contains("Sandbox CWD: /work\n"));
    assert!(log.contains("Tmpfs inserted: True\n"));
}

#[test]
fn missing_home_relative_workspace_and_log_failure_never_execute() {
    let fixture = Fixture::new();
    let mut missing_home = fixture.command(&["--", "/bin/true"]);
    missing_home.env_remove("HOME");
    let output = missing_home.output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("HOME environment variable is required")
    );
    assert!(!fixture.result.exists());

    let output = fixture.run(&[
        "--argv0",
        "codex-linux-sandbox",
        "--sandbox-policy-cwd=relative",
        "--",
        "/bin/codex",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must be absolute"));
    assert!(!fixture.result.exists());
    assert!(!fixture.log.exists());
    assert!(fixture.log.parent().unwrap().is_dir());

    let blocker = fixture.temp.path("not-a-directory");
    std::fs::write(&blocker, "file").unwrap();
    let mut log_failure = fixture.command(&["--", "/bin/true"]);
    log_failure.env("BWRAP_LOG", blocker.join("wrapper.log"));
    assert!(!log_failure.output().unwrap().status.success());
    assert!(!fixture.result.exists());
}

#[test]
fn missing_or_nonexecutable_bwrap_is_reported() {
    let fixture = Fixture::new();
    std::fs::set_permissions(&fixture.fake, std::fs::Permissions::from_mode(0o600)).unwrap();
    let output = fixture.run(&[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("found in PATH"));
    assert!(!fixture.result.exists());
    std::fs::remove_file(&fixture.fake).unwrap();
    assert!(!fixture.run(&[]).status.success());
    let mut command = fixture.command(&[]);
    command.env_remove("PATH");
    assert!(String::from_utf8_lossy(&command.output().unwrap().stderr).contains("PATH is not set"));
}

#[test]
fn path_skips_self_aliases_and_invalid_candidates() {
    let fixture = Fixture::new();
    let mut directories = Vec::new();
    for name in [
        "self",
        "symlink",
        "hardlink",
        "nonexec",
        "directory",
        "broken",
    ] {
        let dir = fixture.temp.path(name);
        std::fs::create_dir(&dir).unwrap();
        directories.push(dir);
    }
    let own = fixture.temp.path("wrapper");
    std::fs::copy(env!("CARGO_BIN_EXE_bwrap"), &own).unwrap();
    std::os::unix::fs::symlink(&own, directories[0].join("bwrap")).unwrap();
    std::os::unix::fs::symlink(directories[0].join("bwrap"), directories[1].join("bwrap")).unwrap();
    std::fs::hard_link(&own, directories[2].join("bwrap")).unwrap();
    std::fs::write(directories[3].join("bwrap"), "not executable").unwrap();
    std::fs::create_dir(directories[4].join("bwrap")).unwrap();
    std::os::unix::fs::symlink("absent", directories[5].join("bwrap")).unwrap();
    let template = fixture.command(&["argument"]);
    let mut command = Command::new(&own);
    command.env_clear().args(["argument"]);
    for (key, value) in template.get_envs() {
        command.env(key, value.unwrap());
    }
    command.env("PATH", std::env::join_paths(&directories).unwrap());
    assert!(!command.output().unwrap().status.success());
    directories.push(fixture.temp.0.clone());
    command.env("PATH", std::env::join_paths(&directories).unwrap());
    command.env("BWRAP_REAL", "/ignored/legacy/override");
    assert!(command.output().unwrap().status.success());
    assert_eq!(fixture.lines()[2], "ARG=argument");
}

#[test]
fn logging_failure_stops_before_attempting_the_default_real_executable() {
    let fixture = Fixture::new();
    let blocker = fixture.temp.path("not-a-directory");
    std::fs::write(&blocker, "file").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_bwrap"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", fixture.temp.path("home"))
        .env("BWRAP_LOG", blocker.join("wrapper.log"))
        .args(["--", "/bin/true"]);
    let output = command.output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to prepare invocation log"));
}

#[test]
fn empty_log_value_fails() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["--", "/bin/true"]);
    command.env("BWRAP_LOG", "");
    assert!(!command.output().unwrap().status.success());
    assert!(!fixture.result.exists());
}

#[test]
fn non_utf8_arguments_fail_logging_before_exec_like_python() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let mut command = fixture.command(&[]);
    command.arg(OsString::from_vec(vec![b'x', 0xff, b'y']));
    let output = command.output().unwrap();
    assert!(!output.status.success());
    assert!(!fixture.result.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("non-UTF-8"));
}

#[test]
fn empty_path_entry_searches_current_directory() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["argument"]);
    command.current_dir(&fixture.temp.0).env("PATH", "");
    assert!(command.output().unwrap().status.success());
    assert_eq!(fixture.lines()[2], "ARG=argument");
}

/// Invoked in a child test process by codex_command and scripts/check_reference.py.
#[test]
fn fixture_driver() {
    use std::os::unix::ffi::OsStringExt;
    let Some(path) = std::env::var_os("BWRAP_TEST_ARGS") else {
        return;
    };
    let encoded = std::fs::read(path).unwrap();
    let args = if encoded.is_empty() {
        Vec::new()
    } else {
        assert_eq!(encoded.last(), Some(&0));
        encoded[..encoded.len() - 1]
            .split(|b| *b == 0)
            .map(|arg| OsString::from_vec(arg.to_vec()))
            .collect::<Vec<_>>()
    };
    let mounts = std::env::var_os("BWRAP_TEST_MOUNTS").unwrap();
    if let Err(error) = runtime::run_with_args(&args, || mounts::read_file(&mounts)) {
        eprintln!("bwrap-wrapper: {error}");
        std::process::exit(1);
    }
}

#[test]
fn timestamps_have_iso_offsets_and_python_booleans() {
    for (timezone, suffix) in [
        ("UTC0", "+00:00"),
        ("JST-9", "+09:00"),
        ("EST5", "-05:00"),
        ("XST-0:00:30", "+00:00:30"),
    ] {
        let fixture = Fixture::new();
        let mut command = fixture.command(&["Unicode: 日本語", "a'b", "line\nbreak"]);
        command.env("TZ", timezone);
        assert!(command.output().unwrap().status.success());
        let log = std::fs::read_to_string(&fixture.log).unwrap();
        let timestamp = log
            .lines()
            .find_map(|line| line.strip_prefix("Timestamp: "))
            .unwrap();
        assert!(timestamp.ends_with(suffix), "{timestamp}");
        assert_eq!(&timestamp[10..11], "T");
        assert_eq!(&timestamp[19..20], &suffix[..1]);
        assert!(log.contains("Tmpfs inserted: False\n"));
    }
}

#[test]
fn signal_termination_and_standard_streams_propagate() {
    use std::os::unix::process::ExitStatusExt;
    let fixture = Fixture::new();
    std::fs::write(
        &fixture.fake,
        "#!/bin/sh\necho stdout-message\necho stderr-message >&2\nkill -TERM $$\n",
    )
    .unwrap();
    let output = fixture.run(&["--", "/bin/true"]);
    assert_eq!(output.status.signal(), Some(libc::SIGTERM));
    assert_eq!(output.stdout, b"stdout-message\n");
    assert_eq!(output.stderr, b"stderr-message\n");
}

#[test]
fn absolute_configuration_home_works_after_cwd_is_deleted() {
    let fixture = Fixture::new();
    let deleted = fixture.temp.path("deleted-cwd");
    std::fs::create_dir(&deleted).unwrap();
    let driver = fixture.codex_command(&[
        "--argv0",
        "codex-linux-sandbox",
        "--sandbox-policy-cwd=/w",
        "--bind",
        "/w",
        "/w",
        "--",
        "/opt/codex",
    ]);
    let mut command = Command::new("/bin/sh");
    command.env_clear();
    for (key, value) in driver.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    command.env("BWRAP_TEST_CWD", &deleted)
        .env("BWRAP_TEST_DRIVER", driver.get_program())
        .env("XDG_CONFIG_HOME", fixture.temp.path("absolute-config"))
        .args(["-c", "cd \"$BWRAP_TEST_CWD\" && /bin/rmdir \"$BWRAP_TEST_CWD\" && exec \"$BWRAP_TEST_DRIVER\" --exact fixture_driver --nocapture"]);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fixture.result.exists());
}

#[test]
fn standard_input_is_inherited() {
    use std::io::Write;
    use std::process::Stdio;
    let fixture = Fixture::new();
    std::fs::write(
        &fixture.fake,
        "#!/bin/sh\nIFS= read -r line\nprintf 'received:%s\\n' \"$line\"\n",
    )
    .unwrap();
    let mut command = fixture.command(&[]);
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"input with spaces\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"received:input with spaces\n");
}

#[test]
fn non_utf8_environment_values_prevent_execution_like_python() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let mut command = fixture.command(&[]);
    command.env("INVALID_VALUE", OsString::from_vec(vec![0xff]));
    assert!(!command.output().unwrap().status.success());
    assert!(!fixture.result.exists());
}

#[test]
fn missing_or_invalid_utf8_mount_tables_stop_before_logging_and_exec() {
    for invalid_utf8 in [false, true] {
        let fixture = Fixture::new();
        let mut command =
            fixture.codex_command(&["--argv0", "codex-linux-sandbox", "--", "/opt/codex"]);
        let path = fixture.temp.path("mounts");
        if invalid_utf8 {
            std::fs::write(path, [0xff]).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
        assert!(!command.output().unwrap().status.success());
        assert!(!fixture.result.exists());
        assert!(!fixture.log.exists());
        assert!(fixture.log.parent().unwrap().is_dir());
    }
}

#[test]
fn current_helper_logs_recognition_and_home_insertion() {
    let fixture = Fixture::new();
    let args: Vec<_> = include_str!("fixtures/current-sandbox.txt")
        .lines()
        .collect();
    let output = fixture.codex_command(&args).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let log = std::fs::read_to_string(&fixture.log).unwrap();
    assert!(log.contains("Codex invocation: True\n"));
    assert!(log.contains("Sandbox CWD: /work/project\n"));
    assert!(log.contains("Tmpfs inserted: True\n"));
}
