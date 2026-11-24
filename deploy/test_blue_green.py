import argparse
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("blue_green", Path(__file__).with_name("blue_green.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        (root / ".env.mysql").write_text("MYSQL_URL=test\n")
        env = root / "deployment.env"
        env.write_text(
            f"DEPLOY_ROOT={root}\n"
            "KIRO_IMAGE=test:latest\n"
            "KIRO_VERIFY_MODE=none\n"
            "KIRO_DRAIN_SECONDS=1\n"
            f"CADDYFILE={root / 'Caddyfile'}\n"
            f"KIRO_CADDY_UPSTREAM_FILE={root / 'kiro-rs-upstream.caddy'}\n"
            "KIRO_PUBLIC_URL=https://public.example:6091\n"
            "KIRO_CADDY_RELOAD_CMD=caddy reload --config {caddyfile} --adapter caddyfile --force\n"
        )
        self.caddyfile = root / "Caddyfile"
        self.fragment = root / "kiro-rs-upstream.caddy"
        self.caddyfile.write_text(
            "site {\n    reverse_proxy {\n"
            f"        import {self.fragment}\n"
            "        flush_interval -1\n    }\n}\n"
        )
        self.fragment.write_text("to 127.0.0.1:16091\n")
        self.args = argparse.Namespace(env=str(env), action="deploy", no_build=True)
        self.deploy = module.Deploy(self.args)
        self.deploy.check_caddy_config = lambda: None
        self.deploy.backup = lambda: None
        self.old = {
            "name": "kiro-rs", "color": "blue", "local_port": 16091,
            "address": "172.18.0.2:6091", "image": "old", "running": True,
        }
        self.new = {
            "name": "kiro-rs-green", "color": "green", "local_port": 16092,
            "address": "172.18.0.3:6091", "image": "new", "running": True,
        }

    def test_failed_candidate_does_not_switch(self):
        state = {"active": self.old, "retired": []}
        with patch.object(self.deploy, "active_state", return_value=state), \
             patch.object(self.deploy, "upstream", return_value="127.0.0.1:16091"), \
             patch.object(self.deploy, "info_for", side_effect=lambda name: self.old if name == "kiro-rs" else None), \
             patch.object(self.deploy, "build_image"), \
             patch.object(module, "run", return_value=type("Result", (), {"stdout": "new"})()), \
             patch.object(self.deploy, "start_candidate", side_effect=RuntimeError("start failed")), \
             patch.object(self.deploy, "set_upstream") as switch:
            with self.assertRaises(RuntimeError):
                self.deploy.deploy()
            switch.assert_not_called()
        self.assertEqual(state["phase"], "candidate_failed")

    def test_failed_post_switch_restores_old_upstream(self):
        state = {"active": self.old, "retired": []}
        switches = []
        def start(candidate):
            candidate.update(self.new)
        def set_upstream(address):
            switches.append(address)
        def verify(instance):
            if instance["name"] == self.new["name"] and len(switches) > 0:
                raise RuntimeError("bad upstream")
        with patch.object(self.deploy, "active_state", return_value=state), \
             patch.object(self.deploy, "upstream", return_value="127.0.0.1:16091"), \
             patch.object(self.deploy, "info_for", side_effect=lambda name: self.old if name == "kiro-rs" else None), \
             patch.object(self.deploy, "build_image"), \
             patch.object(module, "run", return_value=type("Result", (), {"stdout": "new"})()), \
             patch.object(self.deploy, "start_candidate", side_effect=start), \
             patch.object(self.deploy, "check_ready", return_value={"ready": True}), \
             patch.object(self.deploy, "verify", side_effect=verify), \
             patch.object(self.deploy, "set_upstream", side_effect=set_upstream):
            with self.assertRaises(RuntimeError):
                self.deploy.deploy()
        self.assertEqual(switches, ["127.0.0.1:16092", "127.0.0.1:16091"])
        self.assertEqual(state["phase"], "failed")

    def test_public_confirm_timeout_restores_old_upstream(self):
        state = {"active": self.old, "retired": []}
        switches = []
        def start(candidate):
            candidate.update(self.new)
        def set_upstream(address):
            switches.append(address)
            self.fragment.write_text(f"to {address}\n")
            if address == "127.0.0.1:16092":
                raise RuntimeError("public instance timeout")
        with patch.object(self.deploy, "active_state", return_value=state), \
             patch.object(self.deploy, "info_for", side_effect=lambda name: self.old if name == "kiro-rs" else None), \
             patch.object(self.deploy, "build_image"), \
             patch.object(module, "run", return_value=type("Result", (), {"stdout": "new"})()), \
             patch.object(self.deploy, "start_candidate", side_effect=start), \
             patch.object(self.deploy, "check_ready", return_value={"ready": True}), \
             patch.object(self.deploy, "verify"), \
             patch.object(self.deploy, "set_upstream", side_effect=set_upstream):
            with self.assertRaises(RuntimeError):
                self.deploy.deploy()
        self.assertEqual(switches, ["127.0.0.1:16092", "127.0.0.1:16091"])
        self.assertEqual(state["phase"], "failed")

    def test_failed_container_stop_preserves_retired_instance(self):
        result = type("Result", (), {"returncode": 1, "stderr": "cannot stop"})()
        with patch.object(module, "run", return_value=result), \
             patch.object(self.deploy, "info_for", return_value={"running": True}):
            with self.assertRaises(RuntimeError):
                self.deploy.retire(self.old)

    def test_missing_previous_refuses_rollback(self):
        with patch.object(self.deploy, "active_state", return_value={"active": self.old}):
            with self.assertRaises(RuntimeError):
                self.deploy.rollback()

    def test_upstream_reads_fragment(self):
        self.assertEqual(self.deploy.upstream(), "127.0.0.1:16091")
        self.assertEqual(self.deploy.color_for("127.0.0.1:16092"), "green")

    def test_missing_fragment_asks_for_init(self):
        self.fragment.unlink()
        with self.assertRaisesRegex(RuntimeError, "初始化"):
            self.deploy.upstream()

    def test_upstream_mismatch_blocks_deploy(self):
        state = {"active": self.old, "retired": []}
        with patch.object(self.deploy, "active_state", return_value=state), \
             patch.object(self.deploy, "upstream", return_value="127.0.0.1:16092"), \
             patch.object(self.deploy, "backup") as backup, \
             patch.object(self.deploy, "set_upstream") as switch:
            with self.assertRaisesRegex(RuntimeError, "不一致"):
                self.deploy.deploy()
            switch.assert_not_called()
            backup.assert_not_called()

    def test_stopped_instance_can_be_cleaned_without_ip(self):
        state = {"active": self.new, "phase": "awaiting_drain", "retired": [self.old], "previous": self.old}
        with patch.object(self.deploy, "active_state", return_value=state), \
             patch.object(self.deploy, "retire", return_value=True), \
             patch.object(self.deploy, "remove") as remove:
            self.deploy.cleanup()
        remove.assert_called_once_with(self.old["name"])
        self.assertEqual(state["phase"], "complete")
        self.assertNotIn("previous", state)

    def test_backup_failure_blocks_switch(self):
        state = {"active": self.old, "retired": []}
        with patch.object(self.deploy, "active_state", return_value=state), \
             patch.object(self.deploy, "upstream", return_value="127.0.0.1:16091"), \
             patch.object(self.deploy, "check_caddy_config"), \
             patch.object(self.deploy, "backup", side_effect=RuntimeError("backup failed")), \
             patch.object(self.deploy, "set_upstream") as switch:
            with self.assertRaises(RuntimeError):
                self.deploy.deploy()
            switch.assert_not_called()

    def result(self, returncode=0, stdout="", stderr=""):
        return type("Result", (), {"returncode": returncode, "stdout": stdout, "stderr": stderr})()

    def health(self, instance):
        return 200, {}, json.dumps({"ready": True, "instance": instance})

    def test_set_upstream_switches_after_validate_and_reload(self):
        commands = []
        def fake_run(args, **kwargs):
            commands.append(args)
            return self.result()
        with patch.object(module, "run", side_effect=fake_run), \
             patch.object(module, "request", return_value=self.health("green")):
            self.deploy.set_upstream("127.0.0.1:16092")
        self.assertEqual(self.fragment.read_text(), "to 127.0.0.1:16092\n")
        self.assertEqual([command[1] for command in commands], ["validate", "reload"])
        self.assertEqual(commands[1][-1], "--force")

    def test_set_upstream_validate_failure_restores_without_reload(self):
        commands = []
        def fake_run(args, **kwargs):
            commands.append(args)
            return self.result(1, stderr="bad config")
        with patch.object(module, "run", side_effect=fake_run), \
             patch.object(module, "request") as request:
            with self.assertRaisesRegex(RuntimeError, "validate"):
                self.deploy.set_upstream("127.0.0.1:16092")
        self.assertEqual(self.fragment.read_text(), "to 127.0.0.1:16091\n")
        self.assertEqual([command[1] for command in commands], ["validate"])
        request.assert_not_called()

    def test_set_upstream_reload_failure_restores_and_reloads_again(self):
        commands = []
        fragments = []
        def fake_run(args, **kwargs):
            commands.append(args)
            if args[1] == "reload":
                fragments.append(self.fragment.read_text())
                return self.result(1 if len(fragments) == 1 else 0, stderr="reload failed")
            return self.result()
        with patch.object(module, "run", side_effect=fake_run), \
             patch.object(module, "request") as request:
            with self.assertRaisesRegex(RuntimeError, "已回写旧 upstream"):
                self.deploy.set_upstream("127.0.0.1:16092")
        self.assertEqual(self.fragment.read_text(), "to 127.0.0.1:16091\n")
        self.assertEqual([command[1] for command in commands], ["validate", "reload", "reload"])
        self.assertEqual(fragments, ["to 127.0.0.1:16092\n", "to 127.0.0.1:16091\n"])
        request.assert_not_called()

    def test_set_upstream_times_out_when_public_instance_does_not_change(self):
        clock = iter(range(0, 100))
        def fake_request(url, **kwargs):
            return self.health("green" if url.startswith("http://127.0.0.1:16092") else "blue")
        with patch.object(module, "run", return_value=self.result()), \
             patch.object(module, "request", side_effect=fake_request) as request, \
             patch.object(module.time, "monotonic", side_effect=lambda: next(clock)), \
             patch.object(module.time, "sleep"):
            with self.assertRaisesRegex(RuntimeError, "15 秒"):
                self.deploy.set_upstream("127.0.0.1:16092")
        public = [call for call in request.call_args_list if call.args[0].startswith("https://public.example")]
        self.assertEqual(public[0].args[0], "https://public.example:6091/health/ready")
        self.assertGreater(len(public), 1)

    def test_set_upstream_rejects_non_blue_green_address(self):
        with patch.object(module, "run") as run:
            with self.assertRaises(RuntimeError):
                self.deploy.set_upstream("172.18.0.3:6091")
            run.assert_not_called()
        self.assertEqual(self.fragment.read_text(), "to 127.0.0.1:16091\n")

    def test_check_caddy_config_requires_import(self):
        self.caddyfile.write_text("site {\n    reverse_proxy 127.0.0.1:16091\n}\n")
        deploy = module.Deploy(self.args)
        with patch.object(module, "run", return_value=self.result()) as run:
            with self.assertRaisesRegex(RuntimeError, "import"):
                deploy.check_caddy_config()
            run.assert_not_called()

    def test_check_caddy_config_validates(self):
        deploy = module.Deploy(self.args)
        with patch.object(module, "run", return_value=self.result()) as run:
            deploy.check_caddy_config()
        self.assertEqual(run.call_args.args[0][:2], ["caddy", "validate"])

    def test_rollback_requires_previous_ready(self):
        state = {"active": self.new, "phase": "complete", "previous": self.old}
        with patch.object(self.deploy, "active_state", return_value=state):
            with self.assertRaises(RuntimeError):
                self.deploy.rollback()


if __name__ == "__main__":
    unittest.main()
