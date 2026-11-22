import unittest
from unittest.mock import Mock

from bwrap_wrapper.transform import transform

HOME = "/home/test"
WORK = "/work/project"
GENERATED = HOME + "/.codex/tmp/arg0-linux/session"
HELPER = "codex-linux-sandbox"


def codex(options=(), helper_options=(), tail=(), *, legacy=False, helper=None):
    if legacy:
        return [*options, "--argv0", HELPER, "--", "/usr/bin/codex", *helper_options, *tail]
    return [*options, "--", helper or f"/bin/{HELPER}", *helper_options, *tail]


class TransformTests(unittest.TestCase):
    def rewrite(self, args):
        return transform(args, HOME, Mock(return_value=[]), Mock(return_value=[]))

    def test_rewrite_order_and_fresh_overlays(self):
        bind = ["--bind", WORK, WORK]
        close = ["--", "/bin/sh", "--dev", "/dev", "--sandbox-policy-cwd=relative"]
        policy = ["--sandbox-policy-cwd", "/work/./sub/../project/"]
        command = ["--", GENERATED + "/" + HELPER, *policy, *close]
        args = [
            "--dev", "/dev", "--dev", "/other",
            "--ro-bind", WORK + "/.git", WORK + "/.git",
            "--perms", "000", "--tmpfs", "/tmp/.aws", "--remount-ro", "/tmp/.aws",
            *bind,
            "--ro-bind", "/tmp/.aws/", "/tmp/.aws/",
            *bind,
            "--tmpfs", WORK + "/.agents", "--remount-ro", WORK + "/.agents",
            *command,
        ]
        calls = []

        def overlays():
            calls.append("overlays")
            return [f"/config/overlay{len(calls)}"]

        def mounts():
            calls.append("mounts")
            return ["/mnt/c", "/mnt/c/nested"]

        result = transform(args, HOME, overlays, mounts, ["/extra/a", "/extra/b"])
        self.assertEqual(calls, ["overlays", "overlays", "mounts"])
        self.assertEqual(result.workspace, WORK)
        self.assertTrue(result.is_codex and result.home_inserted)
        self.assertEqual(result.args, [
            "--dev-bind", "/dev", "/dev", "--dev", "/other",
            "--tmpfs", HOME, "--dir", HOME + "/.codex",
            "--ro-bind-try", HOME + "/.codex/packages", HOME + "/.codex/packages",
            "--bind-try", HOME + "/.codex/memories", HOME + "/.codex/memories",
            "--ro-bind-try", HOME + "/.codex/AGENTS.md", HOME + "/.codex/AGENTS.md",
            "--dir", HOME + "/.codex/tmp", "--dir", HOME + "/.codex/tmp/arg0-linux",
            "--ro-bind", GENERATED, GENERATED,
            *bind, "--ro-bind", "/config/overlay1", WORK + "/overlay1",
            "--ro-bind", "/tmp/.aws/", "/tmp/.aws/",
            *bind, "--ro-bind", "/config/overlay2", WORK + "/overlay2",
            "--tmpfs", "/mnt/c", "--tmpfs", "/mnt/c/nested",
            "--ro-bind-try", "/extra/a", "/extra/a", "--ro-bind-try", "/extra/b", "/extra/b",
            *command,
        ])

    def test_detectors_and_workspace_scopes(self):
        policy = "--sandbox-policy-cwd"
        for args, expected in [
            (codex([policy, "/outer"], [policy, "//current/./"], ["--", policy, "/nested"]),
             "//current"),
            (codex([policy, "/outer"], [policy, "/legacy"], legacy=True), "/outer"),
            (codex(tail=["--", policy, "/nested"], legacy=True), "/nested"),
            (codex(tail=["--", policy, "/nested"]), None),
            (codex(helper_options=[policy], helper=HELPER), None),
            (codex(helper_options=[policy + "=", policy, "/ignored"]), None),
            (codex(helper_options=[policy + "=/a/../../b"]), "/b"),
            (codex(helper_options=[policy + "=///a"]), "/a"),
        ]:
            with self.subTest(args=args):
                result = self.rewrite(args)
                self.assertTrue(result.is_codex)
                self.assertEqual(result.workspace, expected)
                self.assertEqual(result.project_tmp_index, None if expected is None else 0)
        for args in [
            ["--dev", "/dev"],
            ["--", "/bin/" + HELPER + "/"],
            ["--argv0", "--", HELPER],
            ["--argv0", HELPER, "--", "codex"],
            ["--argv0", HELPER, "--", "/bin/codex/"],
            ["--", "/bin/other", "--", HELPER],
            ["--"],
        ]:
            with self.subTest(args=args):
                unused = Mock(side_effect=AssertionError)
                result = transform(args, HOME, unused, unused, ["/ignored"])
                self.assertFalse(result.is_codex)
                self.assertEqual(result.args, args)
                self.assertIsNone(result.project_tmp_index)
        for value in ("relative", ".", "../work", "--other"):
            with self.subTest(value=value):
                with self.assertRaisesRegex(ValueError, "must be absolute"):
                    self.rewrite(codex(helper_options=[policy, value]))

    def test_project_tmp_precedes_existing_child_mounts(self):
        workspace = "/tmp/project"
        mask = ["--perms", "000", "--tmpfs", "/tmp/daemon", "--remount-ro", "/tmp/daemon"]
        root = ["--ro-bind", "/", "/"]
        policy = ["--sandbox-policy-cwd", workspace]
        for legacy in (False, True):
            with self.subTest(legacy=legacy):
                args = codex([*root, *mask, "--bind", workspace, workspace,
                              "--bind", "/tmp", "/tmp", "--bind-try", "/tmp", "/tmp"],
                             policy, legacy=legacy)
                result = self.rewrite(args)
                self.assertEqual(result.project_tmp_index, len(root))
                self.assertEqual(result.args[:len(root) + len(mask)], root + mask)
                self.assertEqual(result.args[-(len(policy) + 2):], args[-(len(policy) + 2):])
                self.assertNotIn("/tmp", result.args)
                self.assertTrue(result.home_inserted)

    def test_project_tmp_follows_the_root_mount_without_a_host_tmp_bind(self):
        for root in (["--tmpfs", "/"], ["--bind", "/", "/"]):
            with self.subTest(root=root):
                args = codex([*root, "--dir", "/tmp/subdir"], ["--sandbox-policy-cwd=" + WORK])
                result = self.rewrite(args)
                self.assertEqual(result.project_tmp_index, len(root))
                self.assertEqual(result.args, args)

    def test_project_tmp_follows_the_read_only_tmp_bootstrap_mount(self):
        bootstrap = ["--tmpfs", "/", "--ro-bind", "/tmp", "/tmp"]
        args = codex([*bootstrap, "--bind", "/tmp", "/tmp"],
                     ["--sandbox-policy-cwd=" + WORK])
        result = self.rewrite(args)
        self.assertEqual(result.project_tmp_index, len(bootstrap))
        self.assertEqual(result.args, bootstrap + args[len(bootstrap) + 3:])

    def test_exact_protection_groups_and_literal_operands(self):
        for base in (WORK, "/tmp"):
            for name in (".git", ".agents", ".codex", ".aws"):
                target = base + "/" + name
                mask = ["--tmpfs", target, "--remount-ro", target]
                for group in (mask, ["--perms", "000", *mask], ["--ro-bind", target, target]):
                    for size in range(1, len(group) + 1):
                        with self.subTest(group=group, size=size):
                            args = codex(group[:size], ["--sandbox-policy-cwd=" + WORK])
                            expected = args if size < len(group) else args[size:]
                            self.assertEqual(self.rewrite(args).args, expected)
        retained = [
            "--bind", WORK + "/", WORK + "/",
            "--ro-bind", WORK + "/./.git", WORK + "/.git",
            "--perms", "--tmpfs", WORK + "/.git", "--remount-ro", WORK + "/.git",
            "--ro-bind", "/tmp/nested/.aws", "/tmp/nested/.aws",
        ]
        args = codex(retained, ["--sandbox-policy-cwd=" + WORK])
        result = self.rewrite(args)
        self.assertEqual(result.args, args)
        self.assertFalse(result.home_inserted)
        args = codex(["--ro-bind", "/tmp/.git", "/tmp/.git"])
        self.assertEqual(self.rewrite(args).args, args[3:])

    def test_malformed_bind_consumes_operands(self):
        unused = Mock(side_effect=AssertionError)
        args = ["--argv0", HELPER, "--bind", "/w", "--", "/bin/codex"]
        self.assertEqual(transform(args, HOME, unused, unused).args, args)
        args = codex(["--bind"], tail=["--dev", "/dev", "--", "--dev", "/dev"], legacy=True)
        output = transform(args, HOME, unused, lambda: ["/mnt/c"])
        self.assertEqual(output.args, [
            "--bind", "--argv0", HELPER, "--tmpfs", "/mnt/c",
            "--", "/usr/bin/codex", "--dev", "/dev", "--", "--dev", "/dev",
        ])
        args = ["--argv0", HELPER, "--bind", "/w", "--", "/bin/codex", "--dev", "/dev",
                "--", "command"]
        self.assertEqual(transform(args, HOME, unused, lambda: ["/mnt/c"]).args, [
            *args[:6], "--dev-bind", "/dev", "/dev", "--tmpfs", "/mnt/c", "--", "command",
        ])

    def test_overlay_error_precedes_mount_reader(self):
        args = codex(["--bind", WORK, WORK], ["--sandbox-policy-cwd=" + WORK])
        overlays = Mock(side_effect=OSError("overlay unavailable"))
        with self.assertRaisesRegex(OSError, "overlay unavailable"):
            transform(args, HOME, overlays, Mock(side_effect=AssertionError))
        mounts = Mock(side_effect=OSError("mount table unavailable"))
        with self.assertRaisesRegex(OSError, "mount table unavailable"):
            transform(codex(), HOME, Mock(side_effect=AssertionError), mounts)

    def test_generated_helper_path_is_matched_lexically(self):
        for helper in [
            GENERATED + "/./" + HELPER,
            GENERATED + "/../other/" + HELPER,
            HOME + "/.codex/tmp/arg0-linux//" + HELPER,
            HOME + "/.codex/tmp/arg0-linux/../" + HELPER,
            "/other/.codex/tmp/arg0-linux/session/" + HELPER,
        ]:
            with self.subTest(helper=helper):
                result = self.rewrite(codex(
                    ["--bind", WORK, WORK], ["--sandbox-policy-cwd=" + WORK], helper=helper,
                ))
                self.assertTrue(result.home_inserted)
                self.assertNotIn(HOME + "/.codex/tmp", result.args)
                self.assertIn(helper, result.args)
