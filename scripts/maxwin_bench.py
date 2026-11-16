#!/usr/bin/env python3
"""MaxWin agent series (bench/gold/maxwin.yaml): atlas_only vs grep_only, N runs.

Moves report*.md aside for the duration (contamination guard). Scores final
answers against must_find tokens/paths. Records stream-json + summary JSON.

Usage:
  python3 scripts/maxwin_bench.py --project ~/Projects/slotor777/slotor \\
      --out results/maxwin-$(date +%Y%m%d) --n 5 --modes atlas,grep
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GOLD = ROOT / "bench" / "gold" / "maxwin.yaml"

TASK = """Задача: найти ошибку в логике MaxWin (максимальный выигрыш) для фриспинов. Ошибка проявляется на проде, причина неизвестна. Ты работаешь на локальной копии проекта, доступа к проду, его логам и БД нет. Это только расследование: код не меняй, файлы не создавай, вопросов не задавай, допущения перечисли явно.
В итоговом ответе дай:
1. как устроена логика MaxWin для фриспинов (file:line, цепочка вызовов);
2. найденные ошибки по убыванию вероятности: file:line, суть, сценарий на проде, последствия, уверенность;
3. что проверить на проде (логи, SQL);
4. предлагаемое исправление (описанием).
"""

ATLAS_NOTE = (
    "Для кода используй ТОЛЬКО mcp__atlas_slotor__*. "
    "Grep/Glob/Read/Bash не используй; исключение: один раз Read гайда "
    "atlas-navigation/SKILL.md или .claude/skills/atlas-navigation/SKILL.md."
)
GREP_NOTE = "Используй ТОЛЬКО Grep / Glob / Read / Bash. Никаких MCP-инструментов."

ATLAS_TOOLS = [
    "mcp__atlas_slotor__atlas_search",
    "mcp__atlas_slotor__atlas_context",
    "mcp__atlas_slotor__atlas_outline",
    "mcp__atlas_slotor__atlas_grep",
    "mcp__atlas_slotor__atlas_read",
    "mcp__atlas_slotor__atlas_batch",
]
GREP_TOOLS = [
    "Bash(rg:*)",
    "Bash(grep:*)",
    "Bash(sed:*)",
    "Bash(find:*)",
    "Bash(cat:*)",
    "Bash(ls:*)",
    "Bash(head:*)",
    "Bash(wc:*)",
    "Bash(git:*)",
    "Read",
    "Grep",
    "Glob",
]
DENIED = ["Edit", "Write", "NotebookEdit", "Task", "Agent", "WebFetch", "WebSearch"]


def load_gold() -> dict:
    try:
        import yaml
    except ImportError:
        sys.exit("pip install pyyaml")
    return yaml.safe_load(GOLD.read_text())


def score(answer: str, gold: dict) -> dict[str, bool]:
    text = answer.lower()
    out: dict[str, bool] = {}
    for fact in gold.get("must_find", []):
        fid = fact["id"]
        paths_ok = True
        if fact.get("paths"):
            paths_ok = any(p.lower() in text for p in fact["paths"])
        tokens = [t.lower() for t in fact.get("tokens_any", [])]
        tokens_ok = (not tokens) or any(t in text for t in tokens)
        # api_bonus_type: path alone is weak; require bonus_type OR BONUS_COD concept
        if fid == "api_bonus_type":
            tokens_ok = any(
                t in text
                for t in [
                    "bonus_type",
                    "bonus_cod",
                    "missing:[event, bonus_type]",
                    "missing:[bonus_type",
                ]
            )
            paths_ok = "mybonuscontroller" in text or "api/modules/v1" in text
        out[fid] = paths_ok and tokens_ok
    return out


def hide_reports(project: Path) -> Path | None:
    reports = list(project.glob("report*.md"))
    if not reports:
        return None
    stash = project / ".atlas" / f"_bench_reports_{int(time.time())}"
    stash.mkdir(parents=True, exist_ok=True)
    for p in reports:
        shutil.move(str(p), stash / p.name)
    return stash


def restore_reports(stash: Path | None) -> None:
    if not stash or not stash.exists():
        return
    project = stash.parent.parent if stash.parent.name == ".atlas" else stash.parent
    # stash is project/.atlas/_bench_reports_TS → project is parents[1]
    project = stash.parents[1]
    for p in stash.iterdir():
        shutil.move(str(p), project / p.name)
    stash.rmdir()


def write_atlas_mcp(project: Path, out_dir: Path) -> Path:
    token = None
    env = project / ".env"
    if env.exists():
        m = re.search(r"^ATLAS_AGENT_TOKEN=(.*)$", env.read_text(), re.M)
        if m:
            token = m.group(1).strip().strip('"').strip("'")
    if not token:
        sys.exit("ATLAS_AGENT_TOKEN missing in project .env")
    cfg = {
        "mcpServers": {
            "atlas_slotor": {
                "type": "http",
                "url": "http://127.0.0.1:8085/mcp",
                "headers": {"Authorization": f"Bearer {token}"},
            }
        }
    }
    path = out_dir / "atlas-only.mcp.json"
    path.write_text(json.dumps(cfg, indent=2))
    return path


def run_one(
    *,
    mode: str,
    run_id: int,
    project: Path,
    out_dir: Path,
    mcp: Path,
    model: str | None,
) -> dict:
    prompt = TASK + "\n" + (ATLAS_NOTE if mode == "atlas" else GREP_NOTE)
    cmd = [
        "claude",
        "-p",
        prompt,
        "--output-format",
        "stream-json",
        "--verbose",
        "--dangerously-skip-permissions",
    ]
    if model:
        cmd += ["--model", model]
    if mode == "atlas":
        cmd += [
            "--strict-mcp-config",
            "--mcp-config",
            str(mcp),
            "--allowedTools",
            ",".join(ATLAS_TOOLS + ["Read"]),
            "--disallowedTools",
            ",".join(["Bash", "Grep", "Glob", *DENIED]),
        ]
    else:
        cmd += [
            "--strict-mcp-config",
            "--mcp-config",
            str(out_dir / "empty.mcp.json"),
            "--allowedTools",
            ",".join(GREP_TOOLS),
            "--disallowedTools",
            ",".join(["mcp__*", *DENIED]),
        ]
    stream = out_dir / f"{mode}_{run_id}.jsonl"
    meta_path = out_dir / f"{mode}_{run_id}.summary.json"
    t0 = time.time()
    proc = subprocess.run(
        cmd,
        cwd=str(project),
        capture_output=True,
        text=True,
        timeout=1200,
    )
    elapsed = time.time() - t0
    stream.write_text(proc.stdout)
    if proc.stderr:
        (out_dir / f"{mode}_{run_id}.stderr.txt").write_text(proc.stderr)
    events = []
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    answer = ""
    usage = {}
    cost = None
    tool_calls = 0
    atlas_calls = 0
    for ev in events:
        if ev.get("type") == "assistant":
            msg = ev.get("message") or {}
            for block in msg.get("content") or []:
                if isinstance(block, dict) and block.get("type") == "tool_use":
                    tool_calls += 1
                    name = block.get("name") or ""
                    if "atlas_" in name:
                        atlas_calls += 1
                if isinstance(block, dict) and block.get("type") == "text":
                    answer = block.get("text") or answer
        if ev.get("type") == "result":
            answer = ev.get("result") or answer
            usage = ev.get("usage") or {}
            cost = ev.get("total_cost_usd")
    gold = load_gold()
    hits = score(str(answer), gold)
    summary = {
        "mode": mode,
        "run": run_id,
        "exit": proc.returncode,
        "elapsed_s": round(elapsed, 1),
        "cost_usd": cost,
        "usage": usage,
        "tool_calls": tool_calls,
        "atlas_calls": atlas_calls,
        "atlas_share": (atlas_calls / tool_calls) if tool_calls else 0.0,
        "must_find": hits,
        "answer_head": str(answer)[:500],
        "stream": str(stream.name),
    }
    meta_path.write_text(json.dumps(summary, indent=2, ensure_ascii=False))
    print(
        f"{mode}#{run_id}: exit={proc.returncode} {elapsed:.0f}s "
        f"hits={hits} atlas_share={summary['atlas_share']:.0%} cost={cost}",
        flush=True,
    )
    return summary


def aggregate(rows: list[dict], gold: dict) -> dict:
    by_mode: dict[str, list[dict]] = {}
    for r in rows:
        by_mode.setdefault(r["mode"], []).append(r)
    report: dict = {"modes": {}, "gate": {}}
    for mode, items in by_mode.items():
        n = len(items)
        facts = {}
        for fact in gold.get("must_find", []):
            fid = fact["id"]
            ok = sum(1 for i in items if i["must_find"].get(fid))
            facts[fid] = {"ok": ok, "n": n, "rate": f"{ok}/{n}"}
        report["modes"][mode] = {
            "n": n,
            "facts": facts,
            "median_cost": sorted(
                [i["cost_usd"] for i in items if i["cost_usd"] is not None]
            ),
            "atlas_share_min": min((i["atlas_share"] for i in items), default=0),
        }
    atlas = report["modes"].get("atlas", {}).get("facts", {})
    grep = report["modes"].get("grep", {}).get("facts", {})
    api_a = atlas.get("api_bonus_type", {})
    api_g = grep.get("api_bonus_type", {})
    report["gate"] = {
        "api_bonus_type_atlas_ge_4_of_5": (api_a.get("ok") or 0) >= 4
        and (api_a.get("n") or 0) >= 5,
        "bgp_atlas_ge_4_of_5": (atlas.get("bgp_is_ended", {}).get("ok") or 0) >= 4,
        "atlas_api_recall_ge_grep": (api_a.get("ok") or 0) >= (api_g.get("ok") or 0)
        if grep
        else None,
        "atlas_share_ok": report["modes"].get("atlas", {}).get("atlas_share_min", 0)
        >= 0.7,
    }
    return report


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--n", type=int, default=5)
    ap.add_argument("--modes", default="atlas,grep")
    ap.add_argument("--model", default="opus")
    ap.add_argument("--start", type=int, default=1, help="first run id")
    args = ap.parse_args()
    project = args.project.resolve()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    (out / "empty.mcp.json").write_text(json.dumps({"mcpServers": {}}))
    mcp = write_atlas_mcp(project, out)
    freeze = {
        "slotor_commit": subprocess.check_output(
            ["git", "-C", str(project), "rev-parse", "HEAD"], text=True
        ).strip(),
        "atlas_health": subprocess.check_output(
            ["curl", "-s", "http://127.0.0.1:8085/health"], text=True
        ).strip(),
        "gold": str(GOLD),
        "model": args.model,
        "n": args.n,
    }
    (out / "freeze.json").write_text(json.dumps(freeze, indent=2))
    stash = hide_reports(project)
    rows = []
    try:
        modes = [m.strip() for m in args.modes.split(",") if m.strip()]
        for mode in modes:
            for i in range(args.start, args.start + args.n):
                rows.append(
                    run_one(
                        mode=mode,
                        run_id=i,
                        project=project,
                        out_dir=out,
                        mcp=mcp,
                        model=args.model,
                    )
                )
    finally:
        restore_reports(stash)
    gold = load_gold()
    report = aggregate(rows, gold)
    report["freeze"] = freeze
    report["runs"] = rows
    (out / "report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False))
    print(json.dumps(report["gate"], indent=2))
    print(json.dumps(report["modes"], indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
