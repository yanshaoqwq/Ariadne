#!/usr/bin/env python3
"""JSON-line stdio 驱动器：按桌面端的真实方式驱动 ariadne-ipc。

为什么不用 `ariadne-ipc call`：那是**一次调用一个进程**。而主密码 /
明文许可这类保护状态是**进程内内存**（core/src/config/secrets.rs），
用单次调用模式验不到「同一会话内多步操作」，也验不到「重启后状态丢失」。
桌面端的 sidecar 是长驻进程，只有 stdio 模式才等价。

用法：
  driver = Sidecar(project_root, app_state_root)
  driver.start()
  driver.call("get_app_status")
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import threading
import time

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BINARY = os.path.join(REPO_ROOT, "target", "debug", "ariadne-ipc")


class SidecarError(RuntimeError):
    """后端返回 ok:false 时抛出，携带完整错误体便于取证。"""

    def __init__(self, method: str, payload: dict):
        self.method = method
        self.payload = payload
        super().__init__(f"{method} failed: {payload.get('error')}")


class Sidecar:
    """长驻 ariadne-ipc stdio 进程。

    响应**按 request_id 配对**而不是按顺序读：后端最多 8 个并发 worker
    （MAX_CONCURRENT_IPC_REQUESTS），响应乱序是正常的，按顺序读会张冠李戴。
    """

    def __init__(self, project_root: str, app_state_root: str, env_extra: dict | None = None):
        self.project_root = project_root
        self.app_state_root = app_state_root
        self.env_extra = env_extra or {}
        self.process: subprocess.Popen | None = None
        self._seq = 0
        self._pending: dict[str, dict] = {}
        self._lock = threading.Lock()
        self._reader_thread: threading.Thread | None = None
        self._stderr: list[str] = []

    def start(self) -> None:
        env = dict(os.environ)
        env["ARIADNE_PROJECT_ROOT"] = self.project_root
        env["ARIADNE_APP_STATE_ROOT"] = self.app_state_root
        env.update(self.env_extra)
        self.process = subprocess.Popen(
            [BINARY, "stdio"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            bufsize=0,
        )
        self._reader_thread = threading.Thread(target=self._pump, daemon=True)
        self._reader_thread.start()
        threading.Thread(target=self._pump_stderr, daemon=True).start()

    def _pump(self) -> None:
        assert self.process is not None and self.process.stdout is not None
        for raw in self.process.stdout:
            line = raw.decode("utf-8", errors="replace").strip()
            if not line:
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                # 坏行不能终止 pump——这正是 IPC BOM 那条 P0 的成因
                # （前端遇坏行就停止读取，之后所有响应都收不到）。
                self._stderr.append(f"[unparseable line] {line[:200]}")
                continue
            request_id = message.get("request_id")
            if request_id is None:
                self._stderr.append(f"[no request_id] {line[:200]}")
                continue
            with self._lock:
                self._pending[request_id] = message

    def _pump_stderr(self) -> None:
        assert self.process is not None and self.process.stderr is not None
        for raw in self.process.stderr:
            self._stderr.append(raw.decode("utf-8", errors="replace").rstrip())

    def stderr_text(self) -> str:
        return "\n".join(self._stderr)

    def call(self, method: str, params=None, timeout: float = 60.0, raise_on_error: bool = True):
        """发一次调用并等它的响应。返回 data；失败按 raise_on_error 决定抛不抛。"""
        assert self.process is not None and self.process.stdin is not None
        with self._lock:
            self._seq += 1
            request_id = f"req-{self._seq}"
        envelope = {"request_id": request_id, "method": method, "params": params or {}}
        payload = (json.dumps(envelope, ensure_ascii=False) + "\n").encode("utf-8")
        self.process.stdin.write(payload)
        self.process.stdin.flush()

        deadline = time.time() + timeout
        while time.time() < deadline:
            with self._lock:
                if request_id in self._pending:
                    message = self._pending.pop(request_id)
                    break
            if self.process.poll() is not None:
                raise RuntimeError(
                    f"sidecar exited (code {self.process.returncode}) while waiting for {method}\n"
                    f"stderr:\n{self.stderr_text()}"
                )
            time.sleep(0.01)
        else:
            raise TimeoutError(f"{method} timed out after {timeout}s")

        if not message.get("ok"):
            if raise_on_error:
                raise SidecarError(method, message)
            return message
        return message.get("data")

    def stop(self) -> None:
        if self.process is None:
            return
        try:
            if self.process.stdin:
                self.process.stdin.close()
            self.process.wait(timeout=5)
        except (subprocess.TimeoutExpired, OSError):
            self.process.kill()
        self.process = None


def pretty(value) -> str:
    return json.dumps(value, ensure_ascii=False, indent=2)
