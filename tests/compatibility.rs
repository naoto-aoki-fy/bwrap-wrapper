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
        "--ro-bind-try",
        "/home/u/.codex/packages",
        "/home/u/.codex/packages",
        "--bind-try",
        "/home/u/.codex/memories",
        "/home/u/.codex/memories",
        "--ro-bind-try",
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

fn protection_groups(path: &str) -> [Vec<OsString>; 3] {
    [
        strings(&["--ro-bind", path, path]),
        strings(&["--perms", "555", "--tmpfs", path, "--remount-ro", path]),
        strings(&["--tmpfs", path, "--remount-ro", path]),
    ]
}

fn sandbox_invocation(
    before: &[OsString],
    after: &[OsString],
    current_helper: bool,
) -> Vec<OsString> {
    let mut args = if current_helper {
        vec![]
    } else {
        strings(&["--argv0", "codex-linux-sandbox"])
    };
    args.extend_from_slice(before);
    args.extend(strings(&[
        "--",
        if current_helper {
            "/opt/bin/codex-linux-sandbox"
        } else {
            "/opt/bin/codex"
        },
    ]));
    args.extend_from_slice(after);
    args
}

#[test]
fn all_workspace_protections_removed_for_both_invocations_and_workspace_formats() {
    for current_helper in [false, true] {
        for policy in [
            strings(&["--sandbox-policy-cwd", "/work/./a/../project/"]),
            strings(&["--sandbox-policy-cwd=/work/./a/../project/"]),
        ] {
            for name in [".git", ".agents", ".codex", ".aws"] {
                for group in protection_groups(&format!("/work/project/{name}")) {
                    let mut tail = policy.clone();
                    tail.extend(strings(&[
                        "--permission-profile",
                        r#"{ "permissions": ["workspace-write"], "note": "a\\b" }"#,
                        "--",
                        "/bin/sh",
                        "-c",
                        "printf '%s' \"$HOME\"",
                    ]));
                    tail.extend_from_slice(&group);
                    tail.push(OsString::from_vec(b"raw\xff".to_vec()));
                    let args = sandbox_invocation(&group, &tail, current_helper);
                    let out = rewrite(&args);
                    assert_eq!(out.workspace, Some("/work/project".into()));
                    assert_eq!(
                        out.args,
                        sandbox_invocation(&[], &tail, current_helper),
                        "{args:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn tmp_protections_removed_with_or_without_workspace_and_preserve_command_tail() {
    for current_helper in [false, true] {
        for policy in [
            vec![],
            strings(&["--sandbox-policy-cwd="]),
            strings(&["--sandbox-policy-cwd", "/work/project"]),
            strings(&["--sandbox-policy-cwd=/tmp"]),
        ] {
            for name in [".git", ".agents", ".codex", ".aws"] {
                for group in protection_groups(&format!("/tmp/{name}")) {
                    let mut tail = policy.clone();
                    tail.extend(strings(&[
                        "--permission-profile",
                        r#"{ "permissions": ["workspace-write"], "note": "a\\b" }"#,
                    ]));
                    // Include identical groups in both the helper options and
                    // nested command arguments; everything after the first -- stays.
                    tail.extend_from_slice(&group);
                    tail.extend(strings(&["--", "/bin/sh", "-c", "echo test"]));
                    tail.extend_from_slice(&group);
                    tail.push(OsString::from_vec(b"raw\xff".to_vec()));
                    let args = sandbox_invocation(&group, &tail, current_helper);
                    let out = rewrite(&args);
                    assert_eq!(
                        out.args,
                        sandbox_invocation(&[], &tail, current_helper),
                        "{args:?}"
                    );
                    assert!(!out.home_inserted);
                }
            }
        }
    }
}

#[test]
fn mixed_adjacent_and_repeated_protections_preserve_other_transformations() {
    let mut before = strings(&["--dev", "/dev", "--bind", "/work/project", "/work/project"]);
    let mut retained = strings(&["--bind", "/tmp", "/tmp"]);
    before.extend_from_slice(&retained);
    for suffix in ["1000", "2048", "arbitrary"] {
        for base in ["/work/project", "/tmp"] {
            for name in [".git", ".agents", ".codex", ".aws"] {
                for group in protection_groups(&format!("{base}/{name}")) {
                    before.extend(group);
                }
            }
            let daemon = format!("/tmp/codex-daemon-{suffix}");
            let mask = strings(&[
                "--perms",
                "000",
                "--tmpfs",
                &daemon,
                "--remount-ro",
                &daemon,
            ]);
            before.extend_from_slice(&mask);
            retained.extend(mask);
        }
        let synthetic = format!("/tmp/codex-bwrap-synthetic-mount-targets-{suffix}");
        let bind = strings(&["--ro-bind", &synthetic, &synthetic]);
        before.extend_from_slice(&bind);
        retained.extend(bind);
    }
    let mut unrelated = vec![];
    for path in [
        "/other/.git",
        "/other/.agents",
        "/other/.codex",
        "/other/.aws",
        "/tmp/codex-mount-targets",
    ] {
        for group in protection_groups(path) {
            unrelated.extend(group);
        }
    }
    unrelated.extend(strings(&["--perms", "700", "--dir", "/unrelated"]));
    before.extend_from_slice(&unrelated);
    retained.extend(unrelated);
    let tail = strings(&["--sandbox-policy-cwd", "/work/project"]);
    let args = sandbox_invocation(&before, &tail, true);
    let out = transform::transform(
        &args,
        &Inputs {
            home: OsStr::new("/home/u"),
            config_files: &strings(&["/cfg/AGENTS.md"]),
            windows_mounts: &strings(&["/mnt/c"]),
        },
    )
    .unwrap();
    let mut expected = strings(&["--dev-bind", "/dev", "/dev"]);
    expected.extend(home_mounts());
    expected.extend(strings(&[
        "--bind",
        "/work/project",
        "/work/project",
        "--ro-bind",
        "/cfg/AGENTS.md",
        "/work/project/AGENTS.md",
    ]));
    expected.extend(retained);
    expected.extend(strings(&["--tmpfs", "/mnt/c"]));
    assert_eq!(out.args, sandbox_invocation(&expected, &tail, true));
    assert!(out.home_inserted);
}

#[test]
fn protection_matching_preserves_near_matches_and_different_operands() {
    for (base, name) in ["/work/project", "/tmp"]
        .into_iter()
        .flat_map(|base| [".git", ".agents", ".codex", ".aws"].map(|name| (base, name)))
    {
        let target = format!("{base}/{name}");
        for path in [
            format!("/other/{name}"),
            format!("{target}-backup"),
            format!("{target}/child"),
            format!("{target}/"),
            format!("{base}/subdir/{name}"),
            format!("{base}/./{name}"),
            format!("{base}//{name}"),
            format!("{base}/subdir/../{name}"),
            format!("/{base}/{name}"),
            format!("/var/tmp/{name}"),
            format!("$TMPDIR/{name}"),
        ] {
            for group in protection_groups(&path) {
                let args = sandbox_invocation(
                    &group,
                    &strings(&["--sandbox-policy-cwd=/work/./project/"]),
                    true,
                );
                assert_eq!(rewrite(&args).args, args);
            }
        }
        for group in [
            strings(&["--ro-bind", "/other", &target]),
            strings(&["--ro-bind", &target, "/other"]),
            strings(&["--ro-bind-try", &target, &target]),
            strings(&["--bind", &target, &target]),
            strings(&["--tmpfs", &target, "--remount-ro", "/other"]),
            strings(&["--tmpfs", "/other", "--remount-ro", &target]),
            strings(&[
                "--perms",
                "555",
                "--tmpfs",
                &target,
                "--remount-ro",
                "/other",
            ]),
            strings(&["--perms", "700", "--dir", &target]),
        ] {
            let args = sandbox_invocation(
                &group,
                &strings(&["--sandbox-policy-cwd=/work/project"]),
                true,
            );
            assert_eq!(rewrite(&args).args, args);
        }
    }
}

#[test]
fn incomplete_protection_groups_are_preserved_without_crossing_separator() {
    for target in ["/work/project", "/tmp"]
        .into_iter()
        .flat_map(|base| [".git", ".agents", ".codex", ".aws"].map(|name| format!("{base}/{name}")))
    {
        for group in protection_groups(&target) {
            for len in 1..group.len() {
                let args = sandbox_invocation(
                    &group[..len],
                    &strings(&["--sandbox-policy-cwd=/work/project"]),
                    true,
                );
                assert_eq!(rewrite(&args).args, args, "{args:?}");
            }
        }
        let missing_mode = sandbox_invocation(
            &strings(&["--perms", "--tmpfs", &target, "--remount-ro", &target]),
            &strings(&["--sandbox-policy-cwd=/work/project"]),
            true,
        );
        assert_eq!(rewrite(&missing_mode).args, missing_mode);
    }
    let args = codex(
        &[
            "--sandbox-policy-cwd=/work/project",
            "--perms",
            "555",
            "--tmpfs",
            "/work/project/.aws",
            "--dir",
            "/unrelated",
            "--remount-ro",
            "/work/project/.aws",
        ],
        &[],
    );
    assert_eq!(rewrite(&args).args, args);
}

#[test]
fn workspace_protections_require_codex_and_a_valid_workspace() {
    let groups: Vec<_> = [".git", ".agents", ".codex", ".aws"]
        .into_iter()
        .flat_map(|name| protection_groups(&format!("/work/project/{name}")))
        .flatten()
        .collect();
    let tmp_groups: Vec<_> = [".git", ".agents", ".codex", ".aws"]
        .into_iter()
        .flat_map(|name| protection_groups(&format!("/tmp/{name}")))
        .flatten()
        .collect();
    let mut non_codex = groups.clone();
    non_codex.extend_from_slice(&tmp_groups);
    non_codex.extend(strings(&[
        "--",
        "/bin/true",
        "--sandbox-policy-cwd=/work/project",
    ]));
    assert_eq!(rewrite(&non_codex).args, non_codex);
    for current_helper in [false, true] {
        for policy in [vec![], strings(&["--sandbox-policy-cwd="])] {
            let args = sandbox_invocation(&groups, &policy, current_helper);
            assert_eq!(rewrite(&args).args, args);
        }
        let mut invalid_workspace_groups = groups.clone();
        invalid_workspace_groups.extend_from_slice(&tmp_groups);
        let args = sandbox_invocation(
            &invalid_workspace_groups,
            &strings(&["--sandbox-policy-cwd=relative"]),
            current_helper,
        );
        assert_eq!(
            transform::transform(
                &args,
                &Inputs {
                    home: OsStr::new("/home/u"),
                    config_files: &[],
                    windows_mounts: &[],
                },
            ),
            Err(transform::RelativeWorkspace)
        );
    }
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
        "--dir",
        "/home/u/.codex/tmp",
        "--dir",
        "/home/u/.codex/tmp/arg0-linux",
        "--ro-bind",
        "/home/u/.codex/tmp/arg0-linux/example",
        "/home/u/.codex/tmp/arg0-linux/example",
    ]));
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

#[test]
fn optional_home_mounts_do_not_depend_on_source_presence_or_create_sources() {
    for present in 0..8 {
        let temp = TempDir::new();
        let home = temp.0.join("home");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        for (bit, suffix) in [(1, "packages"), (2, "memories"), (4, "AGENTS.md")] {
            if present & bit != 0 {
                let path = home.join(".codex").join(suffix);
                if bit == 4 {
                    std::fs::write(path, "agents").unwrap();
                } else {
                    std::fs::create_dir(path).unwrap();
                }
            }
        }
        let args = codex(&["--sandbox-policy-cwd=/w", "--bind", "/w", "/w"], &[]);
        let output = transform::transform(
            &args,
            &Inputs {
                home: home.as_os_str(),
                config_files: &[],
                windows_mounts: &[],
            },
        )
        .unwrap();
        let expected = home_mounts()
            .into_iter()
            .map(|arg| {
                OsString::from(
                    arg.to_str()
                        .unwrap()
                        .replace("/home/u", home.to_str().unwrap()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(&output.args[3..16], expected);
        for (bit, suffix) in [(1, "packages"), (2, "memories"), (4, "AGENTS.md")] {
            assert_eq!(
                home.join(".codex").join(suffix).exists(),
                present & bit != 0
            );
        }
    }
}

#[test]
fn generated_helper_mount_is_narrow_and_ordered() {
    for name in ["codex-arg0gX9c7R", "another-generated-name"] {
        let dir = format!("/home/u/.codex/tmp/arg0-linux/{name}");
        let helper = format!("{dir}/codex-linux-sandbox");
        let args = strings(&[
            "--bind",
            "/work",
            "/work",
            "--",
            &helper,
            "--sandbox-policy-cwd",
            "/work",
        ]);
        let out = rewrite(&args);
        let mut expected = home_mounts();
        expected.extend(strings(&[
            "--dir",
            "/home/u/.codex/tmp",
            "--dir",
            "/home/u/.codex/tmp/arg0-linux",
            "--ro-bind",
            &dir,
            &dir,
        ]));
        expected.extend(args.clone());
        assert_eq!(out.args, expected);
        let no_isolation = rewrite(&args[3..]);
        assert_eq!(no_isolation.args, args[3..]);
    }
}

#[test]
fn unexpected_helper_paths_never_add_directory_access() {
    for helper in [
        "codex-linux-sandbox",
        "/home/u/.codex/tmp/arg0-linux/codex-linux-sandbox",
        "/home/u/.codex/tmp/arg0-linux/./codex-linux-sandbox",
        "/home/u/.codex/tmp/arg0-linux/../codex-linux-sandbox",
        "/home/u/.codex/tmp/arg0-linux/a/../b/codex-linux-sandbox",
        "/home/u/.codex/tmp/arg0-linux//codex-linux-sandbox",
        "/home/u/.codex/tmp/arg0-linux/a/b/codex-linux-sandbox",
        "/home/other/.codex/tmp/arg0-linux/a/codex-linux-sandbox",
        "/home/u/.codex/tmp/other/a/codex-linux-sandbox",
    ] {
        let args = strings(&[
            "--bind",
            "/work",
            "/work",
            "--",
            helper,
            "--sandbox-policy-cwd",
            "/work",
        ]);
        let mut expected = home_mounts();
        expected.extend(args.clone());
        assert_eq!(rewrite(&args).args, expected, "{helper}");
    }
    let missing = strings(&["--bind", "/work", "/work", "--"]);
    assert_eq!(rewrite(&missing).args, missing);
}
