# Task 25 evidence: wind, current and complex rocks (Linux, 2026-09-28)

- Review sheets and the user's approval: [round 7](../../visual-review/round-07/README.md), [result](../../visual-review/round-07/RESULT.md). Approved baseline: [baseline-25](../../visual-review/baseline-25/README.md).
- `paint-bench-rtx-linux-2026-09-28.txt`, `paint-bench-intel-linux-2026-09-28.txt`: `pigment-prose paint-bench` defaults (12 sample seeds, 16:9) on the RTX 4070 Ti and the Intel RPL-S iGPU. `paint-bench-stress-rtx-linux-2026-09-28.txt`: the synthetic coverage worst case.
- `qualify-linux-2026-09-28.txt`: `scripts/qualify.sh` from a clean worktree at `983c0f3`: 17 PASS, 1 SKIPPED (no software adapter installed).
- Hardware suite: [gpu-tests-linux-2026-09-28-task25.txt](../gpu-tests-linux-2026-09-28-task25.txt), PASS on the RTX 4070 Ti, including all 47 approved images repainting to their baseline-25 hashes.
