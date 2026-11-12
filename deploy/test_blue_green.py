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
        root = Path(self.directory.name)
        (root / ".env.mysql").write_text("MYSQL_URL=test\n")
        env = root / "deployment.env"
        env.write_text(
            f"DEPLOY_ROOT={root}\n"
            "KIRO_IMAGE=test:latest\n"
            "KIRO_VERIFY_MODE=none\n"
            "KIRO_DRAIN_SECONDS=1\n"
        )
        self.args = argparse.Namespace(env=str(env), action="deploy", no_build=True)
        self.deploy = module.Deploy(self.args)
        self.deploy.check_caddy_persistence = lambda: None
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
             patch.object(self.deploy, "upstream", return_value=self.old["address"]), \
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
             patch.object(self.deploy, "upstream", return_value=self.old["address"]), \
             patch.object(self.deploy, "info_for", side_effect=lambda name: self.old if name == "kiro-rs" else None), \
             patch.object(self.deploy, "build_image"), \
             patch.object(module, "run", return_value=type("Result", (), {"stdout": "new"})()), \
             patch.object(self.deploy, "start_candidate", side_effect=start), \
             patch.object(self.deploy, "check_ready", return_value={"ready": True}), \
             patch.object(self.deploy, "verify", side_effect=verify), \
             patch.object(self.deploy, "set_upstream", side_effect=set_upstream):
            with self.assertRaises(RuntimeError):
                self.deploy.deploy()
        self.assertEqual(switches, [self.new["address"], self.old["address"]])
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

    def test_proxy_requires_dedicated_id(self):
        self.assertEqual(self.deploy.proxy_path(), "/id/kiro_rs_active")

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
             patch.object(self.deploy, "upstream", return_value=self.old["address"]), \
             patch.object(self.deploy, "check_caddy_persistence"), \
             patch.object(self.deploy, "backup", side_effect=RuntimeError("backup failed")), \
             patch.object(self.deploy, "set_upstream") as switch:
            with self.assertRaises(RuntimeError):
                self.deploy.deploy()
            switch.assert_not_called()

    def test_rollback_requires_previous_ready(self):
        state = {"active": self.new, "phase": "complete", "previous": self.old}
        with patch.object(self.deploy, "active_state", return_value=state):
            with self.assertRaises(RuntimeError):
                self.deploy.rollback()


if __name__ == "__main__":
    unittest.main()
