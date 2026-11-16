# MaxWin series (cycle-2 gate) — 2026-11-14

Image: `memory-tools:local-r1r2` (post R1–R7 + proof). Model: `opus`.  
Slotor: `67e57a437…`. Gold: `bench/gold/maxwin.yaml`.  
Scripts: `scripts/maxwin_bench.py`. Raw: `atlas_*.jsonl`, `grep_*.jsonl`.

Baseline before fixes (`report_3` / `report_3_v2`): Atlas found API `bonus_type` bug **0/2**.

## Recall (must_find)

| Fact | Atlas | grep |
|---|---|---|
| `api_bonus_type` (MyBonusController without BONUS_COD) | **5/5** | 5/5 |
| `bgp_is_ended` | **5/5** | 5/5 |
| `create_freespin_bind_loop` | **5/5** | 5/5 |

## Cost / time (median)

| | Atlas | grep |
|---|---|---|
| Cost | **$2.00** | $2.44 (−18%) |
| Time | **392 s** | 470 s (−17%) |
| Atlas tool share (min) | **87%** | 0% |

Total spend ≈ $22.4 (atlas $10.05 + grep $12.35).

## Gates

- api_bonus_type Atlas ≥4/5: **PASS** (5/5)
- bgp Atlas ≥4/5: **PASS**
- Atlas recall ≥ grep on api_bonus_type: **PASS** (5=5)
- Atlas share ≥70%: **PASS** (87%)

## Verdict

**MaxWin product gate: PASS.** The pre-fix failure mode (Atlas 0/2 on API `bonus_type`) is closed at 5/5 after R2b/`called_by` missing args. Atlas matches grep on all three gold facts and is cheaper/faster on this series.
