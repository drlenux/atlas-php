#!/usr/bin/env python3
"""Check Atlas MCP against bench/gold/*.yaml atlas_outline must_contain (no LLM).

Env:
  ATLAS_MCP_URL   default http://127.0.0.1:8085/mcp
  ATLAS_AGENT_TOKEN  required
"""
from __future__ import annotations

import json
import os
import sys
import urllib.request
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None

ROOT = Path(__file__).resolve().parents[1]
GOLD = ROOT / "bench" / "gold"


def mcp_call(url: str, token: str, name: str, arguments: dict) -> str:
    body = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }
    ).encode()
    req = urllib.request.Request(
        url,
        data=body,
        headers={
            "Authorization": f"Bearer {token}",
            "content-type": "application/json",
            "accept": "application/json, text/event-stream",
            "mcp-protocol-version": "2025-11-25",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        raw = resp.read().decode("utf-8", "replace")
    if raw.lstrip().startswith("event:") or "\ndata:" in raw[:80]:
        chunks = [ln[5:].lstrip() for ln in raw.splitlines() if ln.startswith("data:")]
        raw = "\n".join(chunks)
    data = json.loads(raw, strict=False)
    if "error" in data:
        raise SystemExit(f"MCP error: {data['error']}")
    parts = [
        c.get("text", "")
        for c in data.get("result", {}).get("content", [])
        if c.get("type") == "text"
    ]
    return "\n".join(parts)


def load_gold(path: Path) -> dict:
    text = path.read_text()
    if yaml is not None:
        return yaml.safe_load(text)
    # Minimal fallback: only files with atlas_outline blocks we parse loosely via json? require pyyaml
    raise SystemExit("install PyYAML: pip install pyyaml")


def main() -> int:
    token = os.environ.get("ATLAS_AGENT_TOKEN")
    if not token:
        print("ATLAS_AGENT_TOKEN required", file=sys.stderr)
        return 2
    url = os.environ.get("ATLAS_MCP_URL", "http://127.0.0.1:8085/mcp")
    failed = 0
    checked = 0
    for path in sorted(GOLD.glob("*.yaml")):
        spec = load_gold(path)
        for fact in spec.get("must_find", []):
            outline = fact.get("atlas_outline")
            if not outline:
                continue
            checked += 1
            text = mcp_call(
                url,
                token,
                "atlas_outline",
                {
                    "entity_id": outline["entity"],
                    "sections": [outline.get("section", "called_by")],
                },
            )
            missing = [s for s in outline.get("must_contain", []) if s not in text]
            if missing:
                failed += 1
                print(f"FAIL {path.name} / {fact['id']}: missing {missing}")
                print(text[:600])
            else:
                print(f"PASS {path.name} / {fact['id']}")
    if checked == 0:
        print("no atlas_outline facts found")
        return 1
    print(f"{checked - failed}/{checked} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
