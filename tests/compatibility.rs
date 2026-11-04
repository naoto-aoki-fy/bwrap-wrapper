use bwrap_wrapper_rs::{
    config, mounts,
    transform::{self, Inputs},
};
use std::ffi::{OsStr, OsString};
use std::os::unix::{ffi::OsStringExt, fs::symlink};
use std::sync::atomic::{AtomicUsize, Ordering};

fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn codex(before: &[&str], after: &[&str]) -> Vec<OsString> {
    let mut args = strings(&["--argv0", "codex-linux-sandbox"]);
    args.extend(strings(before));
    args.extend(strings(&["--", "/opt/bin/codex"]));
    args.extend(strings(after));
    args
}
fn rewrite(args: &[OsString]) -> transform::Output {
    transform::transform(
        args,
        &Inputs {
            home: OsStr::new("/home/u"),
            config_files: &[],
            windows_mounts: &[],
        },
    )
    .unwrap()
}
fn home_mounts() -> Vec<OsString> {
    strings(&[
        "--tmpfs",
        "/home/u",
        "--dir",
        "/home/u/.codex",
        "--ro-bind",
        "/home/u/.codex/packages",
        "/home/u/.codex/packages",
        "--bind",
        "/home/u/.codex/memories",
        "/home/u/.codex/memories",
        "--ro-bind",
        "/home/u/.codex/AGENTS.md",
        "/home/u/.codex/AGENTS.md",
    ])
}

#[test]
fn codex_detection_and_false_positives() {
    assert!(transform::is_codex(&codex(&[], &[])));
    assert!(transform::is_codex(&strings(&[
        "--argv0",
        "codex-linux-sandbox",
        "--",
        "./codex"
    ])));
    for args in [
        vec![],
        strings(&["--argv0", "codex-linux-sandbox"]),
        strings(&["--argv0", "codex-linux-sandbox", "--"]),
        strings(&["--argv0", "codex-linux-sandbox", "--", "codex"]),
        strings(&["--argv0", "codex-linux-sandbox", "--", "/bin/codex/"]),
        strings(&["--argv0", "codex-linux-sandbox", "--", "/bin/codex-other"]),
        strings(&["--argv0", "other", "--", "/bin/codex"]),
        strings(&["--argv0", "--", "codex-linux-sandbox", "/bin/codex"]),
        strings(&["--", "/bin/codex", "--argv0", "codex-linux-sandbox"]),
        strings(&[
            "--argv0",
            "codex-linux-sandbox",
            "--",
            "/bin/other",
            "--",
            "/bin/codex",
        ]),
    ] {
        assert!(!transform::is_codex(&args), "{args:?}");
        let out = transform::transform(
            &args,
            &Inputs {
                home: OsStr::new("/home/u"),
                config_files: &strings(&["/cfg/file"]),
                windows_mounts: &strings(&["/mnt/c"]),
            },
        )
        .unwrap();
        assert_eq!(out.args, args);
        assert!(!out.home_inserted);
        assert_eq!(out.workspace, None);
    }
}

#[test]
fn non_codex_does_not_validate_or_rewrite() {
    let args = strings(&[
        "--dev",
        "/dev",
        "--sandbox-policy-cwd=relative",
        "--",
        "/bin/true",
    ]);
    assert_eq!(rewrite(&args).args, args);
}

#[test]
fn workspace_formats_and_python_normalization() {
    for policy in [
        strings(&["--sandbox-policy-cwd", "/w/./a/../"]),
        strings(&["--sandbox-policy-cwd=/w/./a/../"]),
    ] {
        assert_eq!(transform::workspace(&policy).unwrap(), Some("/w".into()));
    }
    for (input, expected) in [
        ("//w///a/..", "//w"),
        ("///w", "/w"),
        ("/../../", "/"),
        ("/a/../../b", "/b"),
        ("/a/...", "/a/..."),
    ] {
        assert_eq!(
            transform::workspace(&strings(&[&format!("--sandbox-policy-cwd={input}")])).unwrap(),
            Some(expected.into())
        );
    }
    assert_eq!(
        rewrite(&codex(&[], &["--sandbox-policy-cwd=/after"])).workspace,
        Some("/after".into())
    );
}

#[test]
fn workspace_missing_empty_first_occurrence_and_relative() {
    for args in [
        vec![],
        strings(&["--sandbox-policy-cwd"]),
        strings(&["--sandbox-policy-cwd", ""]),
        strings(&["--sandbox-policy-cwd="]),
        strings(&["--sandbox-policy-cwd=", "--sandbox-policy-cwd=/later"]),
    ] {
        assert_eq!(transform::workspace(&args), Ok(None));
    }
    for value in ["relative", ".", "../w", "--other"] {
        assert_eq!(
            transform::workspace(&strings(&["--sandbox-policy-cwd", value])),
            Err(transform::RelativeWorkspace)
        );
    }
    assert_eq!(
        transform::workspace(&codex(&["--sandbox-policy-cwd"], &[])),
        Err(transform::RelativeWorkspace)
    );
    let missing = codex(&[], &["--sandbox-policy-cwd"]);
    assert_eq!(rewrite(&missing).workspace, None);
    assert_eq!(
        transform::workspace(&strings(&[
            "--sandbox-policy-cwd=/first",
            "--sandbox-policy-cwd=/second"
        ])),
        Ok(Some("/first".into()))
    );
    assert_eq!(
        transform::RelativeWorkspace.to_string(),
        "Error: --sandbox-policy-cwd must be absolute"
    );
}

#[test]
fn dev_rewrite_and_command_tail_preservation() {
    let args = codex(
        &[
            "--dev",
            "/dev",
            "--dev",
            "/other",
            "--dev-bind",
            "/dev",
            "/else",
        ],
        &["--dev", "/dev", "--bind", "/w", "/w", "--", "tail"],
    );
    let expected = codex(
        &[
            "--dev-bind",
            "/dev",
            "/dev",
            "--dev",
            "/other",
            "--dev-bind",
            "/dev",
            "/else",
        ],
        &["--dev", "/dev", "--bind", "/w", "/w", "--", "tail"],
    );
    assert_eq!(rewrite(&args).args, expected);
}

#[test]
fn git_removal_uses_exact_bytes_and_removes_each_match() {
    let args = codex(
        &[
            "--sandbox-policy-cwd=/w/./",
            "--ro-bind",
            "/w/.git",
            "/w/.git",
            "--ro-bind",
            "/w/./.git",
            "/w/./.git",
            "--ro-bind",
            "/w/.git",
            "/other",
            "--ro-bind",
            "/w/.git",
            "/w/.git",
        ],
        &["--ro-bind", "/w/.git", "/w/.git"],
    );
    assert_eq!(
        rewrite(&args).args,
        codex(
            &[
                "--sandbox-policy-cwd=/w/./",
                "--ro-bind",
                "/w/./.git",
                "/w/./.git",
                "--ro-bind",
                "/w/.git",
                "/other"
            ],
            &["--ro-bind", "/w/.git", "/w/.git"]
        )
    );
    let no_workspace = codex(&["--ro-bind", "/w/.git", "/w/.git"], &[]);
    assert_eq!(rewrite(&no_workspace).args, no_workspace);
}

#[test]
fn home_once_overlays_after_each_workspace_bind() {
    let args = codex(
        &[
            "--sandbox-policy-cwd=/w",
            "--bind",
            "/w",
            "/w",
            "--bind",
            "/w",
            "/w",
        ],
        &[],
    );
    let files = strings(&["/cfg/.hidden", "/cfg/Z", "/cfg/a"]);
    let result = transform::transform(
        &args,
        &Inputs {
            home: OsStr::new("/home/u"),
            config_files: &files,
            windows_mounts: &[],
        },
    )
    .unwrap();
    let mut expected = strings(&["--argv0", "codex-linux-sandbox", "--sandbox-policy-cwd=/w"]);
    expected.extend(home_mounts());
    for _ in 0..2 {
        expected.extend(strings(&[
            "--bind",
            "/w",
            "/w",
            "--ro-bind",
            "/cfg/.hidden",
            "/w/.hidden",
            "--ro-bind",
            "/cfg/Z",
            "/w/Z",
            "--ro-bind",
            "/cfg/a",
            "/w/a",
        ]));
    }
    expected.extend(strings(&["--", "/opt/bin/codex"]));
    assert_eq!(result.args, expected);
    assert!(result.home_inserted);
}

#[test]
fn no_matching_bind_means_no_home_or_overlays() {
    for before in [
        vec!["--sandbox-policy-cwd=/w"],
        vec!["--bind", "/w", "/w"],
        vec!["--sandbox-policy-cwd=/w/./", "--bind", "/w/./", "/w/./"],
        vec!["--sandbox-policy-cwd=/w", "--bind", "/other", "/w"],
    ] {
        let args = codex(&before, &[]);
        let output = transform::transform(
            &args,
            &Inputs {
                home: OsStr::new("/missing"),
                config_files: &strings(&["/cfg/file"]),
                windows_mounts: &[],
            },
        )
        .unwrap();
        assert_eq!(output.args, args);
        assert!(!output.home_inserted);
    }
}

#[test]
fn raw_argument_and_workspace_bytes_survive() {
    let raw = OsString::from_vec(b"/w\xff".to_vec());
    let mut args = codex(&[], &[]);
    args.extend(["--sandbox-policy-cwd".into(), raw.clone(), raw.clone()]);
    let output = rewrite(&args);
    assert_eq!(output.args, args);
    assert_eq!(output.workspace, Some(raw));
    let mut command = codex(&[], &[]);
    command[3] = OsString::from_vec(b"/\xff/codex".to_vec());
    assert!(transform::is_codex(&command));
}

#[test]
fn home_join_preserves_redundant_slashes() {
    let args = codex(&["--sandbox-policy-cwd=/w", "--bind", "/w", "/w"], &[]);
    let output = transform::transform(
        &args,
        &Inputs {
            home: OsStr::new("/home//u/"),
            config_files: &[],
            windows_mounts: &[],
        },
    )
    .unwrap();
    assert_eq!(
        &output.args[3..7],
        strings(&["--tmpfs", "/home//u/", "--dir", "/home//u/.codex"])
    );
}

#[test]
fn reference_consumes_malformed_bind_operands_even_across_separator() {
    let args = codex(&["--bind", "/w"], &[]);
    // --bind consumes '--' as its destination; the reference never sees a separator.
    let output = transform::transform(
        &args,
        &Inputs {
            home: OsStr::new("/h"),
            config_files: &[],
            windows_mounts: &strings(&["/mnt/c"]),
        },
    )
    .unwrap();
    assert_eq!(output.args, args);
}

#[test]
fn mount_parser_exact_options_octal_dedup_and_ordering() {
    let contents = "bad\nx /mnt/c 9p rw,aname=drvfs 0 0\nx /mnt/c 9p aname=drvfs 0 0\n\
        x /a/deep 9p rw;aname=drvfs;foo 0 0\nx /z 9p aname=drvfs 0 0\n\
        x /mnt/d\\040space 9p aname=drvfs 0 0\nx /mnt/e\\134path 9p aname=drvfs 0 0\n\
        x /wrong ext4 aname=drvfs 0 0\nx /wrong2 9p prefixaname=drvfs 0 0\n\
        x /wrong3 9p aname=drvfs-extra 0 0\nx /unicode\\377 9p aname=drvfs 0 0\n\
        x /literal\\08x 9p aname=drvfs 0 0\n";
    assert_eq!(
        mounts::parse(contents),
        strings(&[
            "/literal\\08x",
            "/unicodeÿ",
            "/z",
            "/a/deep",
            "/mnt/c",
            "/mnt/d space",
            "/mnt/e\\path"
        ])
    );
}

#[test]
fn mount_table_read_errors_are_propagated() {
    let error = mounts::read_file("/definitely/missing/bwrap-wrapper-mounts").unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn windows_mounts_follow_all_mount_operations_before_separator() {
    let args = codex(&["--dev", "/dev"], &["--tmpfs", "/untouched"]);
    let mounts = mounts::parse("x /mnt/d 9p aname=drvfs\nx /c 9p aname=drvfs\n");
    let out = transform::transform(
        &args,
        &Inputs {
            home: OsStr::new("/h"),
            config_files: &[],
            windows_mounts: &mounts,
        },
    )
    .unwrap();
    assert_eq!(
        out.args,
        codex(
            &[
                "--dev-bind",
                "/dev",
                "/dev",
                "--tmpfs",
                "/c",
                "--tmpfs",
                "/mnt/d"
            ],
            &["--tmpfs", "/untouched"]
        )
    );
}

#[test]
fn configuration_home_fallback_and_relative_pathlib_semantics() {
    for xdg in [None, Some(OsStr::new(""))] {
        assert_eq!(
            config::directory(OsStr::new("/home/u"), xdg, OsStr::new("/run")),
            OsString::from("/home/u/.config/bwrap-wrapper/cwd")
        );
    }
    for (xdg, expected) in [
        ("../cfg/./", "/run/../cfg/bwrap-wrapper/cwd"),
        ("/cfg/../other", "/cfg/../other/bwrap-wrapper/cwd"),
        ("//cfg", "//cfg/bwrap-wrapper/cwd"),
    ] {
        assert_eq!(
            config::directory(OsStr::new("/h"), Some(OsStr::new(xdg)), OsStr::new("/run")),
            OsString::from(expected)
        );
    }
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "bwrap-wrapper-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn configuration_files_include_hidden_and_file_symlinks_only() {
    let temp = TempDir::new();
    for name in ["z", ".hidden", "A"] {
        std::fs::write(temp.0.join(name), "contents").unwrap();
    }
    std::fs::create_dir(temp.0.join("directory")).unwrap();
    std::fs::write(temp.0.join("directory/nested"), "excluded").unwrap();
    symlink("A", temp.0.join("linked-file")).unwrap();
    symlink("directory", temp.0.join("linked-dir")).unwrap();
    symlink("missing", temp.0.join("broken")).unwrap();
    assert_eq!(
        config::files(temp.0.as_os_str()).unwrap(),
        [".hidden", "A", "linked-file", "z"].map(|name| temp.0.join(name).into_os_string())
    );
    assert!(
        config::files(temp.0.join("missing").as_os_str())
            .unwrap()
            .is_empty()
    );
    assert!(
        config::files(temp.0.join("A").as_os_str())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn configuration_filename_bytes_use_python_surrogateescape_sort_order() {
    let temp = TempDir::new();
    let names = [
        OsString::from("ÿ"),
        OsString::from_vec(vec![0xff]),
        OsString::from("𐀀"),
    ];
    for name in &names {
        std::fs::write(temp.0.join(name), "contents").unwrap();
    }
    let files = config::files(temp.0.as_os_str()).unwrap();
    assert_eq!(files, names.map(|name| temp.0.join(name).into_os_string()));
    let args = codex(&["--sandbox-policy-cwd=/w", "--bind", "/w", "/w"], &[]);
    let out = transform::transform(
        &args,
        &Inputs {
            home: OsStr::new("/h"),
            config_files: &files,
            windows_mounts: &[],
        },
    )
    .unwrap();
    assert!(out.args.contains(&OsString::from_vec(b"/w/\xff".to_vec())));
}

#[test]
fn discovery_repeats_after_each_bind_and_mounts_come_last() {
    use std::cell::RefCell;
    let temp = TempDir::new();
    std::fs::write(temp.0.join(".first"), "first").unwrap();
    let events = RefCell::new(Vec::new());
    let mut calls = 0;
    let args = codex(
        &[
            "--sandbox-policy-cwd=/w",
            "--bind",
            "/w",
            "/w",
            "--bind",
            "/w",
            "/w",
        ],
        &[],
    );
    let output = transform::transform_with_discovery(
        &args,
        OsStr::new("/home/u"),
        || {
            events.borrow_mut().push("config");
            let files = config::files(temp.0.as_os_str())?;
            if calls == 0 {
                std::fs::remove_file(temp.0.join(".first"))?;
                std::fs::write(temp.0.join("second"), "second")?;
            }
            calls += 1;
            Ok::<_, std::io::Error>(files)
        },
        || {
            events.borrow_mut().push("mounts");
            Ok(strings(&["/mnt/c"]))
        },
    )
    .unwrap();
    assert_eq!(*events.borrow(), ["config", "config", "mounts"]);
    assert_eq!(output.args.iter().filter(|arg| *arg == "--dir").count(), 1);
    let first = output
        .args
        .iter()
        .position(|arg| *arg == "/w/.first")
        .unwrap();
    let second = output
        .args
        .iter()
        .position(|arg| *arg == "/w/second")
        .unwrap();
    assert!(first < second);
    assert_eq!(
        &output.args[second + 1..],
        strings(&["--tmpfs", "/mnt/c", "--", "/opt/bin/codex"])
    );
}

#[test]
fn discovery_is_lazy_and_errors_abort_the_rewrite() {
    let unneeded = || -> Result<Vec<OsString>, std::io::Error> { panic!("unexpected discovery") };
    let non_codex = strings(&[
        "--argv0=codex-linux-sandbox",
        "--dev",
        "/dev",
        "--",
        "/bin/codex",
    ]);
    assert_eq!(
        transform::transform_with_discovery(&non_codex, OsStr::new("/h"), unneeded, unneeded)
            .unwrap()
            .args,
        non_codex
    );
    let malformed = codex(&["--bind", "source"], &[]);
    assert_eq!(
        transform::transform_with_discovery(&malformed, OsStr::new("/h"), unneeded, unneeded)
            .unwrap()
            .args,
        malformed
    );
    let args = codex(&["--sandbox-policy-cwd=/w", "--bind", "/w", "/w"], &[]);
    let result = transform::transform_with_discovery(
        &args,
        OsStr::new("/h"),
        || Err(std::io::Error::other("configuration failure")),
        unneeded,
    );
    assert!(matches!(
        result,
        Err(transform::DiscoveryError::Discovery(_))
    ));
}

#[test]
fn empty_configuration_and_non_missing_metadata_errors() {
    let temp = TempDir::new();
    assert!(config::files(temp.0.as_os_str()).unwrap().is_empty());
    let long = "x".repeat(300);
    let error = config::files(temp.0.join(&long).as_os_str()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ENAMETOOLONG));
    symlink(&long, temp.0.join("bad-target")).unwrap();
    let error = config::files(temp.0.as_os_str()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ENAMETOOLONG));
}

#[test]
fn mount_parsing_handles_nested_paths_python_whitespace_and_cr_lines() {
    let table = "x /deep/a/b 9p aname=drvfs\rx /deep/a 9p aname=drvfs\nx /deep/a 9p aname=drvfs\n\
        x\u{1c}/z\u{1f}9p\u{1d}rw;aname=drvfs\n\
        x /deep\\057a/b/c 9p aname=drvfs\nx /escaped\\040space 9p aname=drvfs\n";
    assert_eq!(
        mounts::parse(table),
        strings(&[
            "/escaped space",
            "/z",
            "/deep/a",
            "/deep/a/b",
            "/deep/a/b/c"
        ])
    );
}

fn current_sandbox() -> Vec<OsString> {
    include_str!("fixtures/current-sandbox.txt")
        .lines()
        .map(OsString::from)
        .collect()
}

#[test]
fn current_helper_transforms_mounts_and_preserves_command_tail() {
    let args = current_sandbox();
    let out = transform::transform(
        &args,
        &Inputs {
            home: OsStr::new("/home/u"),
            config_files: &strings(&["/cfg/AGENTS.md"]),
            windows_mounts: &strings(&["/mnt/c"]),
        },
    )
    .unwrap();
    assert!(out.is_codex);
    assert!(out.home_inserted);
    assert_eq!(out.workspace, Some("/work/project".into()));
    let mut expected = strings(&["--ro-bind", "/", "/", "--dev-bind", "/dev", "/dev"]);
    expected.extend(home_mounts());
    expected.extend(strings(&[
        "--bind",
        "/work/project",
        "/work/project",
        "--ro-bind",
        "/cfg/AGENTS.md",
        "/work/project/AGENTS.md",
        "--ro-bind",
        "/work/other/.git",
        "/work/other/.git",
        "--bind",
        "/work/project",
        "/work/project",
        "--ro-bind",
        "/cfg/AGENTS.md",
        "/work/project/AGENTS.md",
        "--tmpfs",
        "/mnt/c",
    ]));
    let separator = args.iter().position(|a| a == "--").unwrap();
    expected.extend_from_slice(&args[separator..]);
    assert_eq!(out.args, expected);
}

#[test]
fn current_helper_detection_requires_exact_command_position() {
    for command in [
        "codex-linux-sandbox",
        "./codex-linux-sandbox",
        "/opt/codex-linux-sandbox",
    ] {
        assert!(rewrite(&strings(&["--", command])).is_codex);
    }
    for args in [
        strings(&["/opt/codex-linux-sandbox"]),
        strings(&["--"]),
        strings(&["--", ""]),
        strings(&["--", "/opt/codex-linux-sandbox/"]),
        strings(&["--", "/opt/codex-linux-sandbox-other"]),
        strings(&[
            "--",
            "/bin/sh",
            "-c",
            "codex-linux-sandbox",
            "--sandbox-policy-cwd=relative",
        ]),
        strings(&["--", "--", "/opt/codex-linux-sandbox"]),
    ] {
        let out = rewrite(&args);
        assert!(!out.is_codex, "{args:?}");
        assert_eq!(out.args, args);
    }
}

#[test]
fn current_helper_workspace_is_bounded_by_nested_separator() {
    for options in [
        vec![],
        vec!["--sandbox-policy-cwd"],
        vec!["--sandbox-policy-cwd="],
        vec!["--sandbox-policy-cwd", ""],
    ] {
        let mut args = strings(&["--", "/opt/codex-linux-sandbox"]);
        args.extend(strings(&options));
        args.extend(strings(&["--", "/bin/sh", "--sandbox-policy-cwd=/wrong"]));
        let out = rewrite(&args);
        assert_eq!(out.workspace, None);
        assert!(!out.home_inserted);
        assert_eq!(out.args, args);
    }
    for option in [
        "--sandbox-policy-cwd=/work/a/../project",
        "--sandbox-policy-cwd=//work/project",
    ] {
        let args = strings(&["--", "/opt/codex-linux-sandbox", option]);
        assert!(rewrite(&args).workspace.is_some());
    }
    for options in [
        vec!["--sandbox-policy-cwd=relative"],
        vec!["--sandbox-policy-cwd", "relative"],
    ] {
        let mut args = strings(&["--", "/opt/codex-linux-sandbox"]);
        args.extend(strings(&options));
        assert_eq!(
            transform::workspace(&args),
            Err(transform::RelativeWorkspace)
        );
    }
}
