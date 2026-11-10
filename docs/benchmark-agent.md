# Agent benchmark: Code Atlas vs grep

Script: `scripts/agent_benchmark.py`. Two headless Claude Code agents (`claude -p`, same default
model, same project `CLAUDE.md`) get the same requirement questions:

- **atlas**: only `mcp__<server>__atlas_*` (search, context, outline, grep, read, batch);
  `Bash`, `Read`, `Grep`, `Glob` disallowed; `--strict-mcp-config` with the atlas server only;
- **grep**: `rg`/`grep`/`sed`/`find`/`cat`/`ls`/`head`/`wc` via Bash plus `Read`, `Grep`, `Glob`;
  no MCP servers.

The requirement text is part of the prompt, so neither agent reads the spec. Each run saves the
raw `stream-json` events and a summary: turns, tool calls, time, cost, token usage, result bytes
per tool, final answer.

```sh
python3 scripts/agent_benchmark.py --project ../slotor \
  --spec openspec/specs/aggregatorua/spec.md --ids AU-001,AU-004,AU-005,AU-007,AU-008,AU-012 \
  --mcp-config atlas-only.json --server atlas_slotor --out results/
```

## Run 2026-11-10 (slotor, local image 0.1.33)

| Spec, 6 requirements | Agent | Tool calls | Time | Cost | Fresh tokens¹ | Cache reads² | Result bytes |
|---|---|---|---|---|---|---|---|
| aggregatorua (AU-001/004/005/007/008/012) | atlas | **24** | **178 s** | **$1.53** | 123 087 | **1.80 M** | 190 551 |
| | grep | 46 | 231 s | $1.87 | **110 944** | 3.58 M | 187 649 |
| bonus-lifecycle (BL-004/005/011/014/015/021) | atlas | **11** | **94 s** | **$0.68** | **58 547** | **0.52 M** | 50 747 |
| | grep | 32 | 145 s | $0.88 | 59 766 | 1.11 M | 86 324 |

¹ input + cache creation + output. ² context re-read on every turn; grows with the number of
turns, which is why fewer calls matter more than smaller results.

Atlas was 18–23% cheaper and 23–35% faster with half or a third of the calls; fresh tokens were
about equal (+11% on aggregatorua, −2% on bonus-lifecycle). Spot check: both agents listed all 12
callers of `Freespin::createFreeSpin` (the place where the earlier grep run lost 4) and found no
callers of the `updateLimits` implementations.

Where atlas spent its bytes (aggregatorua): `atlas_read` 29 calls / 144 KB (85% of result bytes,
mostly whole files `1–200`), `atlas_grep` 14 / 26 KB, `atlas_outline` 20 / 16 KB, `atlas_search`
2 / 4 KB; 20 of 24 calls were `atlas_batch`.

## Previous owner run (image 0.1.31, aggregatorua)

Atlas 117 142 tokens / 36 calls / 162 s vs grep 68 674 / 11 / 99 s; the grep agent had no built-in
Grep/Glob tools. The token metric of that run is not the same as the columns above.

## Limits

One run per agent and spec; answers differ in wording, so quality was spot-checked, not scored.
Costs depend on the model and cache behaviour of the day.
