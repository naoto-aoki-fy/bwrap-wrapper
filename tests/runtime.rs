use bwrap_wrapper_rs::{config, mounts, transform};
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
        let fake = temp.path("fake-bwrap");
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
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.temp.path("home"))
            .env("BWRAP_REAL", &self.fake)
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
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.result).unwrap()
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
    assert!(log.contains("Codex invocation: false\n"));
    assert!(log.contains("Sandbox CWD: (not specified)\n"));
    assert!(log.contains("Original args: --dev /dev 'plain argument' 'it'\"'\"'s quoted' ''\n"));
    assert!(log.contains("FAKE_SECRET=inherited-value\n"));
    assert!(log.find("BWRAP_REAL=").unwrap() < log.find("FAKE_RESULT=").unwrap());
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
    let mut command = fixture.command(&args);
    command.env("XDG_CONFIG_HOME", &xdg);
    command.env("FAKE_EXIT", "23");
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(23));
    let lines = fixture.lines();
    let home = fixture.temp.path("home");
    let cwd = std::env::current_dir().unwrap();
    let config_dir = config::directory(home.as_os_str(), Some(xdg.as_os_str()), cwd.as_os_str());
    let config_files = config::files(&config_dir).unwrap();
    let windows_mounts = mounts::read_proc_mounts().unwrap();
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
    assert!(log.contains("Codex invocation: true\n"));
    assert!(log.contains("Sandbox CWD: /work\n"));
    assert!(log.contains("Tmpfs inserted: true\n"));
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

    let blocker = fixture.temp.path("not-a-directory");
    std::fs::write(&blocker, "file").unwrap();
    let mut log_failure = fixture.command(&["--", "/bin/true"]);
    log_failure.env("BWRAP_LOG", blocker.join("wrapper.log"));
    assert!(!log_failure.output().unwrap().status.success());
    assert!(!fixture.result.exists());
}

#[test]
fn exec_failure_is_reported_after_logging_and_nonexecutable_is_not_run() {
    let fixture = Fixture::new();
    let mut missing = fixture.command(&["--", "/bin/true"]);
    missing.env("BWRAP_REAL", fixture.temp.path("does-not-exist"));
    let output = missing.output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to execute"));
    assert!(fixture.log.exists());
    assert!(!fixture.result.exists());

    let nonexecutable = fixture.temp.path("no-execute");
    std::fs::write(&nonexecutable, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = std::fs::metadata(&nonexecutable).unwrap().permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(&nonexecutable, permissions).unwrap();
    let mut command = fixture.command(&["--", "/bin/true"]);
    command.env("BWRAP_REAL", nonexecutable);
    assert!(!command.output().unwrap().status.success());
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to write invocation log"));
}

#[test]
fn empty_bwrap_real_and_log_values_preserve_reference_failure_behavior() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["--", "/bin/true"]);
    command.env("BWRAP_REAL", "");
    assert!(!command.output().unwrap().status.success());
    assert!(fixture.log.exists());

    let fixture = Fixture::new();
    let mut command = fixture.command(&["--", "/bin/true"]);
    command.env("BWRAP_LOG", "");
    assert!(!command.output().unwrap().status.success());
    assert!(!fixture.result.exists());
}

#[test]
fn arbitrary_non_utf8_arguments_are_forwarded_without_loss() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let mut command = fixture.command(&[]);
    command.arg(OsString::from_vec(vec![b'x', 0xff, b'y']));
    let output = command.output().unwrap();
    assert!(output.status.success());
    assert!(
        fixture
            .bytes()
            .windows(8)
            .any(|window| window == b"ARG=x\xffy\n")
    );
    let log = std::fs::read(&fixture.log).unwrap();
    assert!(log.windows(3).any(|window| window == [b'x', 0xff, b'y']));
}

#[test]
fn relative_executable_path_uses_execv_without_path_search() {
    let fixture = Fixture::new();
    let mut command = fixture.command(&["argument"]);
    command
        .current_dir(&fixture.temp.0)
        .env("BWRAP_REAL", "fake-bwrap")
        .env("PATH", "/does-not-contain-the-executable");
    assert!(command.output().unwrap().status.success());
    assert_eq!(fixture.lines()[2], "ARG=argument");
}
