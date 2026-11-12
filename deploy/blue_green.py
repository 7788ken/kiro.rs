#!/usr/bin/env python3
"""Zero-downtime blue/green deployment for kiro-rs behind Caddy."""

import argparse
import fcntl
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


def load_env(path):
    values = {}
    for number, line in enumerate(Path(path).read_text().splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not key.strip().replace("_", "").isalnum() or not key.strip()[0].isalpha():
            raise RuntimeError(f"deployment.env:{number}: invalid setting")
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values


def run(args, *, capture=True, check=True, env=None):
    return subprocess.run(
        args,
        check=check,
        text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        env=env,
    )


def request(url, *, method="GET", body=None, headers=None, timeout=10):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={"Content-Type": "application/json", **(headers or {})},
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return response.status, dict(response.headers), response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, dict(error.headers), error.read().decode()


class Deploy:
    def __init__(self, args):
        env_file = Path(args.env).resolve()
        self.env = load_env(env_file)
        self.root = Path(self.env.get("DEPLOY_ROOT", env_file.parent)).resolve()
        self.image = self.env["KIRO_IMAGE"]
        self.network = self.env.get("KIRO_NETWORK", "deploy_new-api-network")
        self.container_port = self.env.get("KIRO_CONTAINER_PORT", "6091")
        self.local_ports = {
            "blue": int(self.env.get("KIRO_BLUE_PORT", "16091")),
            "green": int(self.env.get("KIRO_GREEN_PORT", "16092")),
        }
        self.caddy = self.env.get("CADDY_ADMIN", "http://127.0.0.1:2019")
        self.site_port = self.env.get("KIRO_SITE_PORT", "6091")
        self.public_url = self.env.get("KIRO_PUBLIC_URL", "")
        self.readiness_seconds = int(self.env.get("KIRO_READINESS_SECONDS", "90"))
        self.drain_seconds = int(self.env.get("KIRO_DRAIN_SECONDS", "3600"))
        self.stop_grace_seconds = int(self.env.get("KIRO_STOP_GRACE_SECONDS", "3600"))
        self.state_dir = Path(self.env.get("KIRO_STATE_DIR", str(self.root / "deploy-state")))
        self.state_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.state_file = self.state_dir / "active.json"
        self.lock_handle = None
        self.action = args.action
        self.build = not args.no_build
        self.verify_key = self.env.get("KIRO_VERIFY_API_KEY", "")
        self.verify_model = self.env.get("KIRO_VERIFY_MODEL", "claude-sonnet-4-5")
        self.verify_path = self.env.get("KIRO_VERIFY_PATH", "/cc/v1/messages")
        self.verify_timeout = int(self.env.get("KIRO_VERIFY_TIMEOUT", "90"))
        self.verify_mode = self.env.get("KIRO_VERIFY_MODE", "required")
        self.verify_command = self.env.get("KIRO_VERIFY_COMMAND", "")
        self.mysql_container = self.env.get("KIRO_MYSQL_CONTAINER", "")
        self.mysql_database = self.env.get("KIRO_MYSQL_DATABASE", "")
        self.backup_enabled = self.env.get("KIRO_BACKUP", "1") != "0"
        self.tls_verify = self.env.get("KIRO_TLS_VERIFY", "1") != "0"

    def backup(self):
        if not self.backup_enabled:
            raise RuntimeError("正式蓝绿升级必须启用备份，不能用 KIRO_BACKUP=0 跳过")
        directory = self.root / "config" / "backups" / time.strftime("blue-green-%Y%m%d-%H%M%S")
        directory.mkdir(mode=0o700, parents=True, exist_ok=False)
        for name in ("config.json", "credentials.json"):
            source = self.root / "config" / name
            if not source.is_file():
                raise RuntimeError(f"备份所需配置不存在: {source}")
            target = directory / name
            target.write_bytes(source.read_bytes())
            target.chmod(0o600)
        if self.mysql_container and self.mysql_database:
            dump = run([
                "docker", "exec", self.mysql_container, "sh", "-c",
                'exec mysqldump -uroot -p"$MYSQL_ROOT_PASSWORD" --single-transaction --quick --routines --triggers '
                + self.mysql_database,
            ])
            path = directory / "kiro_rs.sql"
            path.write_text(dump.stdout)
            path.chmod(0o600)
        self.info(f"升级前备份完成: {directory}")

    def check_caddy_persistence(self):
        service = run(["systemctl", "is-active", "caddy-api"], check=False)
        if service.returncode or service.stdout.strip() != "active":
            raise RuntimeError("必须启用 caddy-api.service，确保重启后恢复最新 upstream")
        command = run(["systemctl", "show", "caddy-api", "--property=ExecStart", "--value"])
        if "--resume" not in command.stdout:
            raise RuntimeError("caddy-api 未使用 --resume，管理 API 切流状态不能持久化")
        self.info("Caddy 管理 API 持久化检查通过")

    def check_https(self, url):
        command = ["curl", "-fsS", "--max-time", "10", "-o", "/dev/null", "-w", "%{http_code}"]
        if not self.tls_verify:
            command.append("-k")
        command.append(url)
        result = run(command, check=False)
        if result.returncode or result.stdout.strip() != "200":
            raise RuntimeError("公网管理页 HTTPS 验收失败")

    def info(self, message):
        print(f"[{time.strftime('%H:%M:%S')}] {message}", flush=True)

    def acquire_lock(self):
        self.lock_handle = (self.state_dir / "deploy.lock").open("a+")
        try:
            fcntl.flock(self.lock_handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError("另一个部署任务正在运行，拒绝并发切流")

    def info_for(self, name):
        result = run(["docker", "inspect", name], check=False)
        if result.returncode:
            return None
        data = json.loads(result.stdout)[0]
        networks = data["NetworkSettings"]["Networks"]
        if self.network not in networks:
            raise RuntimeError(f"实例 {name} 不在网络 {self.network}")
        address = networks[self.network].get("IPAddress")
        if not data["State"].get("Running", False):
            address = address or "stopped"
        if not address:
            raise RuntimeError(f"实例 {name} 没有容器 IP")
        bindings = data["HostConfig"].get("PortBindings") or {}
        local = bindings.get(f"{self.container_port}/tcp") or []
        local_port = int(local[0]["HostPort"]) if local else None
        if any(item.get("HostIp") != "127.0.0.1" for item in local):
            raise RuntimeError(f"实例 {name} 的内部端口不得暴露到公网")
        return {
            "name": name,
            "running": data["State"].get("Running", False),
            "image": data["Image"],
            "address": f"{address}:{self.container_port}",
            "local_port": local_port,
        }

    def load_state(self):
        if not self.state_file.exists():
            return None
        state = json.loads(self.state_file.read_text())
        if not self.info_for(state["active"]["name"]):
            raise RuntimeError("状态中的活跃实例不存在，请人工确认，禁止猜测切流")
        return state

    def save_state(self, state):
        temporary = self.state_file.with_suffix(".tmp")
        temporary.write_text(json.dumps(state, indent=2, ensure_ascii=False) + "\n")
        os.chmod(temporary, 0o600)
        temporary.replace(self.state_file)

    def active_state(self):
        state = self.load_state()
        if state:
            return state
        current = self.info_for("kiro-rs")
        if not current or not current["running"]:
            raise RuntimeError("没有找到运行中的旧实例，拒绝直接替换服务")
        local_port = current["local_port"]
        color = next((color for color, port in self.local_ports.items() if port == local_port), None)
        if color is None:
            raise RuntimeError("旧实例的内部端口不在蓝绿端口配置中")
        state = {
            "version": 1,
            "active": {"name": "kiro-rs", "color": color, **current},
            "retired": [],
        }
        self.save_state(state)
        return state

    def proxy_path(self):
        return "/id/kiro_rs_active"

    def upstream(self):
        path = self.proxy_path()
        status, _, body = request(f"{self.caddy}{path}/upstreams")
        if status != 200:
            raise RuntimeError("读取 Caddy upstream 失败")
        upstreams = json.loads(body)
        if len(upstreams) != 1:
            raise RuntimeError("kiro-rs 必须只有一个活跃 upstream")
        return upstreams[0]["dial"]

    def set_upstream(self, address):
        path = self.proxy_path()
        url = f"{self.caddy}{path}/upstreams"
        status, headers, body = request(url)
        etag = headers.get("Etag") or headers.get("ETag")
        if status != 200 or not etag:
            raise RuntimeError("读取 upstream ETag 失败，拒绝切流")
        status, _, body = request(
            url,
            method="PATCH",
            body=[{"dial": address}],
            headers={"If-Match": etag},
        )
        if status != 200:
            raise RuntimeError(f"Caddy 条件切流失败 HTTP {status}: {body[:300]}")
        if self.upstream() != address:
            raise RuntimeError("Caddy 切流后 upstream 与预期不符")
        self.info(f"Caddy 活跃 upstream: {address}")

    def check_ready(self, instance):
        if not instance.get("local_port"):
            raise RuntimeError("候选实例没有本机检查端口")
        deadline = time.monotonic() + self.readiness_seconds
        while time.monotonic() < deadline:
            info = self.info_for(instance["name"])
            if not info or not info["running"]:
                logs = run(["docker", "logs", "--tail", "80", instance["name"]], check=False)
                raise RuntimeError(f"候选实例退出: {logs.stdout}{logs.stderr}")
            try:
                status, _, body = request(
                    f"http://127.0.0.1:{instance['local_port']}/health/ready", timeout=3
                )
                health = json.loads(body)
                if status == 200 and health.get("ready") and health.get("activeRequests") == 0:
                    return health
            except (OSError, ValueError):
                pass
            time.sleep(1)
        raise RuntimeError("候选实例在限定时间内未通过就绪检查")

    def verify(self, instance):
        port = instance["local_port"]
        for path in ("/health/ready", "/health/live", "/admin"):
            status, _, _ = request(f"http://127.0.0.1:{port}{path}", timeout=10)
            if status != 200:
                raise RuntimeError(f"候选 {path} 返回 HTTP {status}")
        if not self.verify_key and self.verify_mode == "required":
            raise RuntimeError("没有配置 KIRO_VERIFY_API_KEY，拒绝跳过真实上游验收")
        if self.verify_key:
            payload = json.dumps({
                "model": self.verify_model,
                "max_tokens": 32,
                "stream": True,
                "messages": [{"role": "user", "content": "Reply with exactly OK."}],
            })
            result = run([
                "curl", "-fsS", "--max-time", str(self.verify_timeout),
                f"http://127.0.0.1:{port}{self.verify_path}",
                "-H", f"x-api-key: {self.verify_key}",
                "-H", "anthropic-version: 2023-06-01",
                "-H", "content-type: application/json", "--data", payload,
            ], check=False)
            if result.returncode or "OK" not in result.stdout:
                raise RuntimeError(f"候选真实流式请求失败: {result.stderr[:300]}")
            self.info("候选真实流式请求通过，响应为 OK")
        if self.verify_command:
            env = os.environ.copy()
            env["KIRO_CANDIDATE_PORT"] = str(port)
            run(["/bin/sh", "-c", self.verify_command], capture=False, env=env)
        if self.mysql_container and self.mysql_database:
            result = run([
                "docker", "exec", self.mysql_container,
                "sh", "-c",
                'export MYSQL_PWD="$MYSQL_ROOT_PASSWORD"; mysql -uroot -NBe "SELECT COUNT(*) FROM '
                + self.mysql_database + '.credit_transactions WHERE status IN (\\"pending\\", \\"processing\\");"',
            ])
            if result.stdout.strip() != "0":
                raise RuntimeError("存在未完成的账务预授权，暂停切换")

    def build_image(self):
        if not self.build:
            return
        self.info(f"构建候选镜像: {self.image}")
        command = ["docker", "compose"]
        files = self.env.get("KIRO_COMPOSE_FILES", "docker-compose.yml,docker-compose.deploy.yml")
        for path in files.split(","):
            command.extend(["-f", str(self.root / path.strip())])
        command.extend(["build", "kiro-rs"])
        run(command, capture=False)

    def start_candidate(self, candidate):
        if self.info_for(candidate["name"]):
            raise RuntimeError(f"候选容器 {candidate['name']} 已存在，请先排查残留部署")
        command = [
            "docker", "run", "-d", "--name", candidate["name"],
            "--network", self.network, "--restart", "no",
            "--stop-timeout", str(self.stop_grace_seconds),
            "--env-file", str(self.root / ".env.mysql"),
            "-e", f"KIRO_INSTANCE={candidate['color']}",
            "-p", f"127.0.0.1:{candidate['local_port']}:{self.container_port}",
            "-v", f"{self.root}/config:/app/config",
            "--label", "kiro-rs.managed=blue-green", self.image,
        ]
        result = run(command, check=False)
        if result.returncode:
            raise RuntimeError(f"启动候选实例失败: {result.stderr[:500]}")
        info = self.info_for(candidate["name"])
        candidate.update(info)
        self.info(f"候选实例已启动: {candidate['name']}，容器地址 {candidate['address']}")

    def candidate(self, state):
        active = state["active"]
        color = "green" if active["color"] == "blue" else "blue"
        return {
            "name": f"kiro-rs-{color}",
            "color": color,
            "local_port": self.local_ports[color],
        }

    def retire(self, old):
        deadline = time.monotonic() + self.drain_seconds
        self.info(f"旧实例 {old['name']} 开始优雅排空，保留在途流式响应")
        result = run([
            "timeout", "--signal=TERM", "10s",
            "docker", "stop", "--time", str(self.stop_grace_seconds), old["name"],
        ], check=False)
        if result.returncode == 124:
            self.info("停止监听请求尚未完成，旧实例仍在处理存量连接，继续观察")
        elif result.returncode and (self.info_for(old["name"]) or {}).get("running"):
            raise RuntimeError(f"旧实例停止请求失败，保留实例: {result.stderr[:300]}")
        while time.monotonic() < deadline:
            info = self.info_for(old["name"])
            if not info or not info["running"]:
                self.info(f"旧实例 {old['name']} 已完成优雅退出")
                return True
            time.sleep(min(5, max(1, self.drain_seconds)))
        self.info("旧实例仍有在途响应，保留容器；稍后执行 cleanup 继续排空")
        return False

    def remove(self, name):
        info = self.info_for(name)
        if info and info["running"]:
            raise RuntimeError(f"拒绝删除仍在运行的实例 {name}")
        if info:
            run(["docker", "rm", name])

    def status(self):
        state = self.active_state()
        print(json.dumps(state, indent=2, ensure_ascii=False))
        print(f"Caddy upstream: {self.upstream()}")

    def deploy(self):
        state = self.active_state()
        active = state["active"]
        current_upstream = self.upstream()
        expected = active.get("address")
        if current_upstream != expected:
            if active["name"] == "kiro-rs" and current_upstream == f"127.0.0.1:{active['local_port']}":
                self.info("首次迁移：将 Caddy 从本机端口切到旧容器 IP，旧进程不重启")
                self.set_upstream(expected)
            else:
                raise RuntimeError(f"Caddy upstream {current_upstream} 与活跃实例 {expected} 不一致")
        candidate = state.get("candidate", self.candidate(state))
        self.check_caddy_persistence()
        self.backup()
        expected_name = self.candidate(state)["name"]
        if candidate.get("name") != expected_name:
            raise RuntimeError("状态中的候选与当前蓝绿颜色不一致，请先人工恢复")
        existing = self.info_for(candidate["name"])
        if existing and state.get("phase") not in {"candidate_starting", "candidate_failed"}:
            raise RuntimeError(f"候选容器 {candidate['name']} 已存在，请先检查上次部署状态")
        if existing and candidate.get("image") and existing["image"] != candidate["image"]:
            raise RuntimeError("已有候选镜像与状态不一致，禁止覆盖")
        self.build_image()
        current_image = run(["docker", "image", "inspect", self.image, "--format", "{{.Id}}"]).stdout.strip()
        if existing and candidate.get("image") and candidate["image"] != current_image:
            raise RuntimeError("候选不是当前构建镜像，请先保留日志并清理旧候选")
        candidate["image"] = current_image
        state["candidate"] = candidate
        state["phase"] = "candidate_starting"
        self.save_state(state)
        switched = False
        try:
            if existing:
                if not existing["running"]:
                    raise RuntimeError("上次候选已停止，请先保留日志并清理候选容器")
                candidate.update(existing)
                self.info("复用本任务已启动的候选实例，重新执行验收")
            else:
                self.start_candidate(candidate)
            self.info(f"候选就绪: {json.dumps(self.check_ready(candidate), ensure_ascii=False)}")
            self.verify(candidate)
            self.set_upstream(candidate["address"])
            switched = True
            state["phase"] = "verifying_active"
            self.save_state(state)
            self.verify(candidate)
            if self.public_url:
                self.check_https(f"{self.public_url}/admin")
            run(["docker", "update", "--restart", "unless-stopped", candidate["name"]])
            state["active"] = candidate
            state["previous"] = active
            state.pop("candidate", None)
            state["phase"] = "draining"
            self.save_state(state)
            if self.retire(active):
                self.remove(active["name"])
                state["phase"] = "complete"
                state["retired"] = []
                state.pop("previous", None)
                self.save_state(state)
                self.info(f"部署完成: {candidate['name']} / {candidate['image']}")
            else:
                state["phase"] = "awaiting_drain"
                state["retired"] = [active]
                self.save_state(state)
                self.info("新版本已接管流量；旧版本继续排空，状态为 awaiting_drain")
        except Exception as error:
            if switched:
                self.set_upstream(active["address"])
                self.verify(active)
                self.info("切流后验收失败，已恢复旧实例流量")
            state["phase"] = "failed" if switched else "candidate_failed"
            state["failed_candidate"] = candidate
            self.save_state(state)
            raise RuntimeError(f"部署失败，旧实例保持服务；候选保留用于排查: {error}") from error

    def rollback(self):
        state = self.active_state()
        previous = state.get("previous")
        if not previous or not self.info_for(previous["name"]):
            raise RuntimeError("没有可恢复的上一个实例")
        if state.get("phase") not in {"draining", "awaiting_drain", "failed"}:
            raise RuntimeError("当前部署阶段不支持自动回滚，请先确认活跃实例与流量状态")
        current = state["active"]
        self.check_ready(previous)
        self.verify(previous)
        self.set_upstream(previous["address"])
        state["active"] = previous
        state["previous"] = current
        state["phase"] = "draining_rollback"
        self.save_state(state)
        if self.retire(current):
            self.remove(current["name"])
            state["retired"] = []
            state["phase"] = "rolled_back"
            state.pop("previous", None)
            self.save_state(state)
        else:
            state["phase"] = "awaiting_drain"
            state["retired"] = [current]
            self.save_state(state)
        self.info("已切回上一个实例，当前版本继续验收")

    def cleanup(self):
        state = self.active_state()
        for instance in state.get("retired", []):
            if self.retire(instance):
                self.remove(instance["name"])
        state["retired"] = []
        if state.get("phase") == "awaiting_drain":
            state["phase"] = "complete"
            state.pop("previous", None)
        self.save_state(state)
        self.info("排空清理完成，未终止仍在运行的实例")

    def execute(self):
        self.acquire_lock()
        if self.action == "status":
            self.status()
        elif self.action == "deploy":
            self.deploy()
        elif self.action == "rollback":
            self.rollback()
        elif self.action == "cleanup":
            self.cleanup()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["deploy", "status", "rollback", "cleanup"])
    parser.add_argument("--env", default="/opt/kiro-rs/deployment.env")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    try:
        Deploy(args).execute()
    except Exception as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
