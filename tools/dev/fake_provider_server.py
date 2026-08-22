#!/usr/bin/env python3
"""本地假 provider 服务：OpenAI 兼容，用于全流程真实跑通验证。

为什么要真起一个 HTTP 服务而不是 mock provider：
mock 会掩盖一整类缺陷（见 CLAUDE.md 方法论）。跨进程 + 真实 socket + 真实
JSON 编解码这条路上的问题（BOM、超时、鉴权头、响应形状不符）只有真实服务
才复现得出来。

端点（按 core/src/providers/http.rs 的实际请求构造）：
  GET  /models             -> fetch_provider_models / test_provider_draft
  POST /chat/completions   -> OpenAiCompatible 协议的 complete()
  POST /embeddings         -> HttpEmbeddingProvider
  POST /rerank             -> reranker

用法：
  python3 tools/dev/fake_provider_server.py --port 8127 --log /tmp/provider.jsonl
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# 每次请求都追加一行到日志，便于事后核对「后端到底发了什么」——
# 这是判断「工作流真的调了 LLM」而不是「只是没报错」的唯一硬证据。
LOG_LOCK = threading.Lock()
LOG_PATH: str | None = None
REQUEST_COUNTER = {"n": 0}


def record(entry: dict) -> None:
    """把一次请求落盘。日志是「真的发生了出站请求」的取证材料。"""
    if LOG_PATH is None:
        return
    with LOG_LOCK:
        with open(LOG_PATH, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, ensure_ascii=False) + "\n")


MODEL_CATALOG = [
    {"id": "fake-writer-large", "object": "model", "owned_by": "ariadne-dev"},
    {"id": "fake-writer-small", "object": "model", "owned_by": "ariadne-dev"},
    {"id": "fake-embed-3", "object": "model", "owned_by": "ariadne-dev"},
]


def pick_tool_call(tools: list, messages: list) -> dict | None:
    """决定这一轮要不要调工具、调哪个。

    工作流的写作节点是 tool-use 驱动的：不返回 tool_calls，节点就只拿到一段
    文本，产不出 patch，也就验不到写回闭环。所以只要请求里带了 tools，
    第一轮就必须真的挑一个调。

    但**不能每轮都调**——tool 结果回填后若再返回 tool_call，会无限循环。
    判据用「消息里已经出现过 tool 结果」：出现过就收尾出文本。

    ⚠️ **优先挑写工具**。取 `tools[0]` 会挑到 `*-find`（只读），于是正文永远
    落不了盘 —— 那时看到的「跑成功但没产出」是**取样器的假象**，不是产品缺陷。
    我第一次就误判过一次，所以这里必须显式偏向写工具。
    """
    if not tools:
        return None
    already_used_tool = any(
        message.get("role") == "tool" or message.get("tool_call_id")
        for message in messages
    )
    if already_used_tool:
        return None
    write_suffixes = ("-insert-lines", "-replace-lines", "-rewrite-file")
    for tool in tools:
        name = (tool.get("function") or {}).get("name") or ""
        if name.endswith(write_suffixes):
            return tool
    return tools[0]


# 中文正文样本。用中文是刻意的：UTF-8 多字节边界是本项目反复出问题的地方
# （CLAUDE.md §3），拿 ASCII 跑通不算跑通。
SAMPLE_PROSE = "沈砚把灯芯拨亮了一点，窗外的雪就显出形状来。\n他知道那封信迟早要写。\n"


def synthesize_arguments(schema: dict, tool_name: str) -> dict:
    """按工具的 JSON Schema 合成一份合法参数。

    不能硬编码某个工具的参数：工具表是后端按节点类型动态给的，硬编码等于
    只测到我猜中的那一个。这里遍历 required 字段按 type 填值，
    未知类型退回字符串——让后端的校验器来当裁判。
    """
    properties = schema.get("properties") or {}
    required = schema.get("required") or list(properties.keys())
    arguments: dict = {}
    for field in required:
        spec = properties.get(field) or {}
        arguments[field] = synthesize_value(field, spec, tool_name)
    return arguments


def synthesize_value(field: str, spec: dict, tool_name: str):
    """给单个字段造值。enum 优先——它是唯一能保证过校验的选择。"""
    if "enum" in spec and spec["enum"]:
        return spec["enum"][0]
    field_type = spec.get("type")
    if isinstance(field_type, list):
        field_type = next((item for item in field_type if item != "null"), "string")
    if field_type == "integer":
        # 行号类字段给 0：after_line = 0 是合法值（空文件写入 / 首行前插入，U123）。
        return 0
    if field_type == "number":
        return 0
    if field_type == "boolean":
        return False
    if field_type == "array":
        item_spec = spec.get("items") or {}
        return [synthesize_value(field, item_spec, tool_name)]
    if field_type == "object":
        return synthesize_arguments(spec, tool_name)
    return synthesize_text_for(field)


def synthesize_text_for(field: str) -> str:
    """按字段名猜内容。正文类字段要给真正的中文段落，否则验不到 UTF-8 路径。"""
    lowered = field.lower()
    if any(key in lowered for key in ("content", "text", "lines", "body", "prose")):
        return SAMPLE_PROSE
    if "path" in lowered or "document" in lowered or "file" in lowered:
        return "planning/global.md"
    if "name" in lowered or "title" in lowered:
        return "沈砚"
    if "query" in lowered or "keyword" in lowered:
        return "沈砚"
    return "自动化验证占位值"


class FakeProviderHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # noqa: D102 - 静音默认 stderr 噪声
        pass

    def _send_json(self, payload: dict, status: int = 200) -> None:
        # 显式 utf-8 编码 + Content-Length：HTTP/1.1 keep-alive 下缺长度会让
        # 客户端一直等，表现成「超时」而不是「格式错」，极难定位。
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_json(self) -> dict:
        length = int(self.headers.get("Content-Length") or 0)
        if length == 0:
            return {}
        raw = self.rfile.read(length)
        try:
            return json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            record({"kind": "bad_request_body", "error": str(error), "raw": repr(raw[:200])})
            return {}

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler 约定
        path = self.path.split("?", 1)[0].rstrip("/")
        auth = self.headers.get("Authorization")
        record({"kind": "GET", "path": path, "has_auth": bool(auth)})
        if path.endswith("/models") or path == "/models":
            self._send_json({"object": "list", "data": MODEL_CATALOG})
            return
        self._send_json({"error": {"message": f"unknown path {path}"}}, status=404)

    def do_POST(self) -> None:  # noqa: N802
        path = self.path.split("?", 1)[0].rstrip("/")
        payload = self._read_json()
        REQUEST_COUNTER["n"] += 1
        auth = self.headers.get("Authorization") or self.headers.get("x-api-key")
        record({
            "kind": "POST",
            "seq": REQUEST_COUNTER["n"],
            "path": path,
            "has_auth": bool(auth),
            "model": payload.get("model"),
            "stream": payload.get("stream"),
            "tool_names": [
                (tool.get("function") or {}).get("name")
                for tool in (payload.get("tools") or [])
            ],
            "message_roles": [m.get("role") for m in (payload.get("messages") or [])],
            "request": payload,
        })
        if path.endswith("/chat/completions"):
            self._send_json(self._chat_completion(payload))
            return
        if path.endswith("/embeddings"):
            self._send_json(self._embeddings(payload))
            return
        if path.endswith("/rerank"):
            self._send_json(self._rerank(payload))
            return
        self._send_json({"error": {"message": f"unknown path {path}"}}, status=404)

    def _chat_completion(self, payload: dict) -> dict:
        """构造 OpenAI chat 响应。

        形状按 core/src/providers/http.rs 的 openai_chat_response 反推：
        choices[0].message.{content,tool_calls}、finish_reason、usage.{prompt,completion}_tokens。
        usage 必须给——它是成本记账与预算闸的输入，缺了预算路径就没被验到。
        """
        tools = payload.get("tools") or []
        messages = payload.get("messages") or []
        chosen = pick_tool_call(tools, messages)
        message: dict = {"role": "assistant", "content": None}
        finish_reason = "stop"
        if chosen is not None:
            function = chosen.get("function") or {}
            name = function.get("name") or "unknown_tool"
            arguments = synthesize_arguments(function.get("parameters") or {}, name)
            message["content"] = ""
            message["tool_calls"] = [{
                "id": f"call_{REQUEST_COUNTER['n']}",
                "type": "function",
                # arguments 是 JSON **字符串**而不是对象：后端用
                # serde_json::from_str 解析，给对象会直接报 invalid JSON arguments。
                "function": {"name": name, "arguments": json.dumps(arguments, ensure_ascii=False)},
            }]
            finish_reason = "tool_calls"
        else:
            message["content"] = SAMPLE_PROSE

        return {
            "id": f"chatcmpl-fake-{REQUEST_COUNTER['n']}",
            "object": "chat.completion",
            "created": 1756000000,
            "model": payload.get("model") or "fake-writer-large",
            "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
            "usage": {"prompt_tokens": 128, "completion_tokens": 64, "total_tokens": 192},
        }

    def _embeddings(self, payload: dict) -> dict:
        raw_input = payload.get("input")
        items = raw_input if isinstance(raw_input, list) else [raw_input]
        # 维度取 8 只为省事，但必须**每条输入都回一条**——数量不匹配是
        # 向量库写入静默错位的经典来源。
        return {
            "object": "list",
            "model": payload.get("model") or "fake-embed-3",
            "data": [
                {"object": "embedding", "index": index, "embedding": [0.01 * (index + 1)] * 8}
                for index, _ in enumerate(items)
            ],
            "usage": {"prompt_tokens": 8 * len(items), "total_tokens": 8 * len(items)},
        }

    def _rerank(self, payload: dict) -> dict:
        documents = payload.get("documents") or []
        return {
            "results": [
                {"index": index, "relevance_score": 1.0 - index * 0.1}
                for index, _ in enumerate(documents)
            ]
        }


def main() -> int:
    global LOG_PATH
    parser = argparse.ArgumentParser(description="本地假 provider 服务")
    parser.add_argument("--port", type=int, default=8127)
    parser.add_argument("--log", default=None, help="请求日志 JSONL 路径")
    args = parser.parse_args()
    LOG_PATH = args.log
    server = ThreadingHTTPServer(("127.0.0.1", args.port), FakeProviderHandler)
    print(f"fake provider listening on http://127.0.0.1:{args.port}/v1", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
