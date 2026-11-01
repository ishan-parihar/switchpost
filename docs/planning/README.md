# Planning index

Each file here is a dated or live planning record. `AGENTS.md` §0.6 is the
authority on which document answers which question; this page only lists them.

| File | Status | What it covers |
|---|---|---|
| [`PLAN_PARITY_DEPTH_SINGLEUSER_v25.md`](PLAN_PARITY_DEPTH_SINGLEUSER_v25.md) | live | Tier rationale (§1) and parity phases. Read before parity or feature work. |
| [`TARGETING_AND_WRITER_DISCIPLINE.md`](TARGETING_AND_WRITER_DISCIPLINE.md) | live | Targeting, collisions, and the single-writer rule (G-08). Why one long-lived process owns the database. |
| [`VENDOR_PINS.md`](VENDOR_PINS.md) | live (CI gate) | Pinned transitive dependencies, enforced by `scripts/vendor-pull.sh --check`. |
| [`OUTLINE-2026-10-19-v2.md`](OUTLINE-2026-10-19-v2.md) | live queue per AGENTS §0.6 | P0 gate, token efficiency, eval loop, credential-blocked live passes. **Its verdicts were true on 2026-10-19; all six had landed by 2026-10-20. Read it with the re-audit below.** |
| [`REAUDIT_OUTLINE-2026-10-19-v2_2026-10-20.md`](REAUDIT_OUTLINE-2026-10-19-v2_2026-10-20.md) | dated-frozen audit | Every claim in the outline above re-measured against the tree, with the measurement command for each. Its finding: B1, B2, B4, C1, C2 and C3 are ALL done, and the "D-test redness" priority inversion is gone. Nothing in its B/C waves survives — an executor agent should not work any of them. |
| [`OUTLINE-2026-10-19-v3.md`](OUTLINE-2026-10-19-v3.md) | archived | Superseded by v2 (the live queue per AGENTS §0.6). Archived at `docs/archive/OUTLINE-2026-10-19-v3.md`. |
| [`OUTLINE-2026-10-19.md`](OUTLINE-2026-10-19.md) | archived | Earlier 2026-10-19 outline, superseded by v2. Archived at `docs/archive/OUTLINE-2026-10-19.md`. |
| [`OUTLINE-2026-10-17.md`](OUTLINE-2026-10-17.md) | archived | Replaced by the 2026-10-19 v2 outline. Its Wave A–C items live in `TODO.md`. Archived at `docs/archive/OUTLINE-2026-10-17.md`. |
| [`OUTLINE-2026-10-16.md`](OUTLINE-2026-10-16.md) | archived (superseded). Archived at `docs/archive/OUTLINE-2026-10-16.md` | Executed predecessor outline (`d5b9bbd..7873f84`). |
| [`DECISIONS_2026-10-19.md`](DECISIONS_2026-10-19.md) | open decisions | Decision points with recommendations. Nothing in it is applied until the user approves it. |
| [`EXECUTION_2026-10-19.md`](EXECUTION_2026-10-19.md) | record | Cycle 3 execution log: what landed, gate results, decisions for the user, and what did not land. |
| [`AUDIT_2026-10-14.md`](../audit/AUDIT_2026-10-14.md) | history (moved to `docs/audit/`) | End-to-end audit: P0 bugs, cookie and API matrix, phased plan. |
| [`SESSION_2026-10-19.md`](SESSION_2026-10-19.md) | post-mortem | Session self-reflection: what worked, every failure with its evidence, and the carry-forward checklist. Read before a delegated or browser-driven session — it records the harness mistakes that produced 48 false-positive probe failures. |
| [`PHASES_P0-P9.md`](PHASES_P0-P9.md) | live | The P0–P9 phase plan the objective names, with each phase's acceptance criteria and the register-row mapping. The register in `TODO.md` carries live status; this file owns the plan (AGENTS rule 1). |
| [`GAPS_2026-10-15.md`](GAPS_2026-10-15.md) | archived (superseded). Archived at `docs/archive/GAPS_2026-10-15.md` | What the 13 audit commits did not close, before the register existed. |
| [`PLAN_UI_PARITY_LAUNCHES_CALENDAR_2026-10-30.md`](PLAN_UI_PARITY_LAUNCHES_CALENDAR_2026-10-30.md) | **landed 2026-11-01** | The 7-phase UI parity plan (chip, cell cap, one-row header, composer preview, 80px rail, per-channel dates, measurement gate). All seven phases are in the tree and every acceptance number was re-measured on the running app — the top blockquote records the three defects the phase verdicts had missed. |

The live work register is `TODO.md` at the repository root.
