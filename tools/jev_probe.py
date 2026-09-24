#!/usr/bin/env python3
"""Jev 探针 —— IF 项目的判定后端实测与回归工具。

用途（对应 docs/07 §6 的校准与回归流程）：
  * 探活：确认端点、凭据、模型版本可用
  * 单发：把一份 state + questions 打给 Jev，打印原始结果
  * 回归：对同一批问题重复 N 次，统计概率稳定性

用法：
  python tools/jev_probe.py --ping
  python tools/jev_probe.py --file batch.json
  python tools/jev_probe.py --file batch.json --repeat 5

batch.json 形如：
  { "state": "…或 {…} 或 […]", "questions": { "q1": { "type": "noul", "instructions": "…" } } }

凭据：优先读环境变量 JEV_KEY，其次读仓库根 jevkey 的 `key:` 行。
密钥不会被打印或写入任何文件。
"""
from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ENDPOINT = "https://openrouter.ai/api/alpha/decisions"
DEFAULT_MODEL = "typesafe/jev-1.13"
REPO_ROOT = Path(__file__).resolve().parent.parent


def load_credentials() -> tuple[str, str]:
    key = os.environ.get("JEV_KEY", "").strip()
    model = os.environ.get("JEV_MODEL", "").strip() or DEFAULT_MODEL
    if key:
        return key, model
    path = REPO_ROOT / "jevkey"
    if not path.exists():
        sys.exit("找不到凭据：设置 JEV_KEY 环境变量，或在仓库根放置 jevkey 文件。")
    text = path.read_text(encoding="utf-8")
    m = re.search(r"^key:\s*(\S+)", text, re.M)
    if not m:
        sys.exit("jevkey 中缺少 `key:` 行。")
    mm = re.search(r"^model:\s*(\S+)", text, re.M)
    if mm:
        model = mm.group(1)
    return m.group(1), model


def ask(state, questions, key: str, model: str, timeout: float = 120.0):
    body = {"model": model, "state": state, "questions": questions}
    req = urllib.request.Request(
        ENDPOINT,
        data=json.dumps(body, ensure_ascii=False).encode("utf-8"),
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
        method="POST",
    )
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw, status = resp.read().decode("utf-8"), resp.status
    except urllib.error.HTTPError as exc:
        raw, status = exc.read().decode("utf-8"), exc.code
    latency_ms = (time.time() - t0) * 1000
    try:
        return status, latency_ms, json.loads(raw)
    except json.JSONDecodeError:
        return status, latency_ms, {"_raw": raw}


def err_text(payload) -> str:
    message = payload.get("error", {}).get("message", payload)
    return str(message).replace("\n", " ")[:400]


def cmd_ping(key: str, model: str) -> int:
    status, latency_ms, payload = ask(
        "ping",
        {"ping": {"type": "noul", "instructions": "这句话是真的吗？"}},
        key,
        model,
    )
    if status != 200:
        print(f"[FAIL] HTTP {status}  {err_text(payload)}")
        return 1
    print(f"[OK] {latency_ms:.0f} ms  解析版本={payload.get('model')}")
    print(f"     usage={json.dumps(payload.get('usage'), ensure_ascii=False)}")
    return 0


def summarize(payload) -> dict:
    out = {}
    for qid, ans in payload.get("answers", {}).items():
        kind = ans.get("type")
        if kind == "noul":
            out[qid] = ans.get("noul")
        elif kind == "choice":
            out[qid] = {"choice": ans.get("choice"), "conf": ans.get("confidence")}
        elif kind == "score":
            out[qid] = {"score": ans.get("score"), "conf": ans.get("confidence")}
        else:
            out[qid] = ans
    return out


def cmd_batch(key: str, model: str, path: Path, repeat: int) -> int:
    spec = json.loads(path.read_text(encoding="utf-8"))
    state = spec["state"]
    questions = spec["questions"]
    runs = []
    for i in range(repeat):
        status, latency_ms, payload = ask(state, questions, key, model)
        if status != 200:
            print(f"第 {i + 1} 轮 HTTP {status}  {err_text(payload)}")
            return 1
        usage = payload.get("usage", {})
        print(
            f"第 {i + 1} 轮  {latency_ms:.0f} ms  in={usage.get('input_tokens')} "
            f"cost=${usage.get('cost'):.8f}  版本={payload.get('model')}"
        )
        for qid, val in summarize(payload).items():
            print(f"    {qid}: {json.dumps(val, ensure_ascii=False)}")
        runs.append((summarize(payload), latency_ms))

    if repeat > 1:
        print("\n稳定性（仅统计 noul 类）:")
        ids = [qid for qid, v in runs[0][0].items() if isinstance(v, (int, float))]
        for qid in ids:
            series = [r[0][qid] for r in runs]
            print(
                f"    {qid}: {series}  均值={statistics.mean(series):.3f}  "
                f"极差={max(series) - min(series):.3f}"
            )
        lat = [r[1] for r in runs]
        print(f"    延迟: min={min(lat):.0f}ms max={max(lat):.0f}ms")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Jev 判定后端探针")
    parser.add_argument("--file", type=Path, help="含 state 与 questions 的 JSON 文件")
    parser.add_argument("--repeat", type=int, default=1, help="重复次数，用于稳定性统计")
    parser.add_argument("--ping", action="store_true", help="最小探活")
    parser.add_argument("--model", default="", help="覆盖模型 ID")
    args = parser.parse_args()

    key, model = load_credentials()
    if args.model:
        model = args.model
    if args.ping:
        return cmd_ping(key, model)
    if not args.file:
        parser.error("需要 --file 或 --ping")
    return cmd_batch(key, model, args.file, max(1, args.repeat))


if __name__ == "__main__":
    raise SystemExit(main())
