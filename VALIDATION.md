# DimStorage — validation record

The current package is **0.2.4** at pin **`0a875da91dbb0fdcb7ba8c42c11341aa229af51a`** + mixins
`e59da9094f0c2fdda6c627fe9eff22dec6a906f0` (Nightly 2026-11-20, launcher 0.2.92). The slice's
record, behaviour, red tests, gaps, coverage table and upstream-defect notes are in
[`notes.md`](notes.md) under `## Slice 3 — the tank-transfer fix (0.2.4)`. This file is the ladder
summary the orchestrator reads.

## Artifact

| Field | Value |
|---|---|
| File | `dimstorage.hmod` (port root) |
| Size | **86,836 B** |
| SHA-256 | `02cc3b1c937686ba6a4e8f1929ca470f6d91fe5cecdc597fbd20c58a5b6333ed` |
| Declared version | `0.2.4` (`horizon-mod.toml`, api `0.2`, side `both`, capabilities `registries`) |
| Votes | `native-mod`, format `native-1/0a875da…/e59da90…`, for 26.3 |
| Entries | 64 — 5 root documents, 3 `src/`, 31 `assets/dimstorage/`, 1 `data/c/`, 21 `data/dimstorage/`, 2 `data/minecraft/`, `uses.json`; **no `Cargo.lock`** |
| Reproducible | two builds after `rm -f Cargo.lock` are `cmp`-identical |

## Ladder (all inside a build-slot lease, `CARGO_INCREMENTAL=0`, no `--target-dir`)

| Step | Command | Result | Evidence |
|---|---|---|---|
| red (TDD) | `cargo test --offline` on the four new tests | **4 failed** for their stated reasons (see `notes.md`) | `../_shared/evidence/dimstorage/sdk-test-transfer-red.log` |
| tests | `cargo test --offline` | **45 passed**, 0 failed (41 `runtime::`, 4 `support::`) | `../_shared/evidence/dimstorage/sdk-test.log` |
| lints | `cargo clippy --all-targets --offline -- -D warnings` | exit 0, 0 warnings | `../_shared/evidence/dimstorage/sdk-clippy.log` |
| format | `cargo fmt --check` | `fmt_exit=0` | `../_shared/evidence/dimstorage/sdk-fmt.log` |
| build ×2 | `rm -f Cargo.lock && cargo horizon-mod build` | `cmp`-identical, 86,836 B | `sdk-build.log`, `sdk-build-second.log` |
| registry check | `cargo horizon-mod check dimstorage.hmod` | exit 0 | `sdk-check.log` |
| fits (pin) | `cargo horizon-mod fits dimstorage.hmod --build 0a875da` | `0a875da  made for` | `sdk-fits.log` |
| fits (other) | `cargo horizon-mod fits dimstorage.hmod --build 0b14f8a` | `0b14f8a  fits` | `sdk-fits-other.log` |
| package gate | `python3 ports/tools/verify-sdk-package.py dimstorage` | **1/1** | `sdk-verify-0.2.4.log` |
| carried-data gate | `python3 ports/tools/check_261_dialect.py .` | exit 0 (unchanged data) | — |

Every log prints the artifact sha256 it was run against (`sdk-check`, `sdk-fits`, `sdk-fits-other`,
`sdk-build*`, `sdk-verify-0.2.4`) or the source hashes plus the pin (`sdk-test`, `sdk-clippy`,
`sdk-fmt`).

## In-game leg

`ports/tools/launcher-game.sh build dimstorage` → `Compiling dimstorage (1 of 1)` (the manifest was
bumped, so no stale native reuse; `../_shared/evidence/dimstorage-e2e/launcher-build-0.2.4.log`),
then
`build-slot.sh run --job W1-dimstorage --port ports/dimstorage --step e2e -- bash tools/e2e.sh`.

| Leg | Result | Log |
|---|---|---|
| scenario | **`PASS` + `EXIT:0`** | `../_shared/evidence/dimstorage-e2e/e2e-launcher.log` |
| control (the same script on the Nightly, **no** package) | `EXIT:1` at the craft | `e2e-launcher-control.log` |
| `SELFTEST=1` (chest sharing inverted) | `EXIT:1` | `e2e-launcher-selftest.log` |
| `SELFTEST=tank` (tank sharing inverted) | `EXIT:1` | `e2e-launcher-selftest-tank.log` |
| `SELFTEST=capacity` (refused fill inverted) | `EXIT:1` | `e2e-launcher-selftest-capacity.log` |
| `SELFTEST=transfer` (drain committed inverted) | `EXIT:1` | `e2e-launcher-selftest-transfer.log` |
| `check-e2e-logs.py dimstorage-e2e --strict` | **6/6** | `check-logs-0.2.4.log` |

The scenario covers the tank recipe crafted in a crafting table (result id and count, grid emptied),
the crafted item placing the block, a bucket filling a channel shared by its tanks, a lava bucket
staying on its own channel, an empty bucket draining the channel, the 16,000 mB cap refusing the
17th bucket **before it moves** (the bucket and the tank's own compound are unchanged), a
**frozen-clock drain** through one tank with the sibling removed before its next tick and a
relaunch that keeps exactly 15,000 mB in both the block's own compound and the store (no
resurrection, no loss), a survival break dropping the tank into a hopper with the channel's fluid
intact, both loot tables rolling their item, both store files written, and both stores surviving a
relaunch — beside the slice-1 chest scenario (menu deposit/withdraw, channel isolation,
vanilla-barrel differential).

## Known limits at this pin

- The tank has no comparator output, no screen, no waterlogging, no auto-eject and no lock
  (`notes.md` coverage table), and a re-placed chest/tank starts on `public / 1` (no item frequency
  component yet).
- A tank's own bucket interaction commits to the channel at the access; a foreign capability
  transfer (a pipe) lands in the tank's saved compound and is committed by the next tick, by a
  break/explosion, or by the persisted `synced` delta after a restart. A `/setblock` deletion
  bypasses the break hook (recorded deviation).
- A creative break drops nothing (this is why the drop leg plays in survival); a hopper under a chest
  pulls from the shared channel, which is the mod's intended behaviour.
- `tanks.dat` now carries `e <frequency>` lines for known empty channels; a downlevel (0.2.3)
  binary skips them, so the marker protection is forward-only.
