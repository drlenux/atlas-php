#!/usr/bin/env python3
"""Atlas vs grep agent benchmark (see docs/benchmark-agent.md).

Runs two headless Claude Code agents on the same requirement questions in a project:
an atlas-only agent (Code Atlas MCP tools) and a grep-only agent (rg/grep/sed/Read).
Records tokens, cost, turns, time and result bytes per tool from `stream-json` output.

Usage:
  agent_benchmark.py --project DIR --spec openspec/specs/aggregatorua/spec.md \
      --ids AU-001,AU-004 --mcp-config atlas.json --server atlas_slotor --out results/
"""
import argparse
import json
import re
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path

ATLAS_TOOLS = ["atlas_search", "atlas_context", "atlas_outline", "atlas_grep", "atlas_read", "atlas_batch"]
GREP_TOOLS = [
    "Bash(rg:*)", "Bash(grep:*)", "Bash(sed:*)", "Bash(find:*)", "Bash(cat:*)",
    "Bash(ls:*)", "Bash(head:*)", "Bash(wc:*)", "Read", "Grep", "Glob",
]
ALWAYS_DENIED = ["Edit", "Write", "NotebookEdit", "Task", "Agent", "WebFetch", "WebSearch"]

PROMPT = """You are analysing the code of this repository (read-only, do not edit anything).
For each requirement below, find where it is implemented and answer briefly:
- entry points (controller actions, console commands, jobs) with file:line;
- the key methods that implement the behaviour, with file:line;
- who calls the key method (complete list of callers);
- whether the code satisfies the requirement: yes / no / partly, with one line of evidence.
Do not guess: say "not found" when the code does not show it.

{tools_note}

Requirements:

{requirements}
"""


def requirement_blocks(spec: Path, ids: list[str]) -> str:
    """Text of each `### Requirement: <ID> …` section for the selected ids."""
    text = spec.read_text(encoding="utf-8")
    parts = re.split(r"(?m)^### Requirement: ", text)
    selected = []
    for part in parts[1:]:
        head = part.split(maxsplit=1)[0]
        if head in ids:
            body = re.split(r"(?m)^## ", part)[0]
            selected.append("### " + body.strip())
    missing = set(ids) - {b.split()[1] for b in selected}
    if missing:
        sys.exit(f"requirements not found in {spec}: {sorted(missing)}")
    return "\n\n".join(selected)


def agent_command(kind: str, prompt: str, mcp_config: str, server: str) -> list[str]:
    command = ["claude", "-p", prompt, "--output-format", "stream-json", "--verbose",
               "--strict-mcp-config"]
    if kind == "atlas":
        command += ["--mcp-config", mcp_config,
                    "--allowedTools", *[f"mcp__{server}__{t}" for t in ATLAS_TOOLS],
                    "--disallowedTools", "Bash", "Read", "Grep", "Glob", *ALWAYS_DENIED]
    else:
        command += ["--allowedTools", *GREP_TOOLS, "--disallowedTools", *ALWAYS_DENIED]
    return command


def summarize(events: list[dict]) -> dict:
    """Tool calls, result bytes per tool and the final usage from stream-json events."""
    names: dict[str, str] = {}
    calls: Counter = Counter()
    result_bytes: Counter = Counter()
    final: dict = {}
    for event in events:
        message = event.get("message")
        message = message if isinstance(message, dict) else {}
        content = message.get("content")
        content = content if isinstance(content, list) else []
        if event.get("type") == "assistant":
            for block in filter(lambda b: isinstance(b, dict), content):
                if block.get("type") == "tool_use":
                    tool = block["name"].split("__")[-1]
                    names[block["id"]] = tool
                    calls[tool] += 1
        elif event.get("type") == "user":
            for block in filter(lambda b: isinstance(b, dict), content):
                if block.get("type") == "tool_result":
                    tool = names.get(block.get("tool_use_id"), "?")
                    payload = block.get("content")
                    if isinstance(payload, list):
                        payload = "".join(p.get("text", "") for p in payload if isinstance(p, dict))
                    result_bytes[tool] += len(str(payload or "").encode())
        elif event.get("type") == "result":
            final = event
    usage = final.get("usage") or {}
    return {
        "turns": final.get("num_turns"),
        "duration_s": round((final.get("duration_ms") or 0) / 1000, 1),
        "cost_usd": final.get("total_cost_usd"),
        "input_tokens": usage.get("input_tokens", 0),
        "cache_creation_tokens": usage.get("cache_creation_input_tokens", 0),
        "cache_read_tokens": usage.get("cache_read_input_tokens", 0),
        "output_tokens": usage.get("output_tokens", 0),
        "total_tokens": sum(usage.get(k, 0) for k in (
            "input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens", "output_tokens")),
        "tool_calls": sum(calls.values()),
        "calls_per_tool": dict(calls),
        "result_bytes_per_tool": dict(result_bytes),
        "answer": final.get("result", ""),
        "is_error": final.get("is_error"),
    }


def run(kind: str, args: argparse.Namespace, requirements: str) -> dict:
    note = ("Use only the Code Atlas tools (atlas_*)." if kind == "atlas"
            else "Use only shell search (rg/grep/sed/find) and file reads.")
    prompt = PROMPT.format(tools_note=note, requirements=requirements)
    started = time.time()
    completed = subprocess.run(agent_command(kind, prompt, args.mcp_config, args.server),
                               cwd=args.project, capture_output=True, text=True, timeout=args.timeout)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    stem = f"{Path(args.spec).parent.name}-{kind}"
    # Raw events first: a parsing problem must not lose a paid run.
    (out / f"{stem}.events.jsonl").write_text(completed.stdout, encoding="utf-8")
    events = [json.loads(line) for line in completed.stdout.splitlines() if line.startswith("{")]
    summary = summarize(events)
    summary["wall_s"] = round(time.time() - started, 1)
    summary["exit_code"] = completed.returncode
    (out / f"{stem}.summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    return summary


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--project", required=True)
    parser.add_argument("--spec", required=True, help="spec path relative to --project")
    parser.add_argument("--ids", required=True, help="comma-separated requirement ids")
    parser.add_argument("--mcp-config", required=True, help="MCP config with the atlas server only")
    parser.add_argument("--server", default="atlas", help="MCP server name in that config")
    parser.add_argument("--agents", default="atlas,grep")
    parser.add_argument("--out", required=True)
    parser.add_argument("--timeout", type=int, default=1800)
    args = parser.parse_args()
    requirements = requirement_blocks(Path(args.project) / args.spec, args.ids.split(","))
    for kind in args.agents.split(","):
        summary = run(kind, args, requirements)
        brief = {k: v for k, v in summary.items() if k != "answer"}
        print(kind, json.dumps(brief, ensure_ascii=False))


if __name__ == "__main__":
    main()
