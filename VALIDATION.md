# DimStorage — validation record

The current package is **0.2.3** at pin **`0a875da91dbb0fdcb7ba8c42c11341aa229af51a`** + mixins
`e59da9094f0c2fdda6c627fe9eff22dec6a906f0` (Nightly 2026-11-20, launcher 0.2.92). The slice's
record, behaviour, gaps, coverage table and upstream-defect notes are in [`notes.md`](notes.md)
under `## Slice 2 — the dimensional tank (0.2.3)`. This file is the ladder summary the
orchestrator reads.

## Artifact

| Field | Value |
|---|---|
| File | `dimstorage.hmod` (port root) |
| Size | **81,237 B** |
| SHA-256 | `0ef5637cb09c57f989f275c2cc067006f0403e382ef540e94fd12192913d741c` |
| Declared version | `0.2.3` (`horizon-mod.toml`, api `0.2`, side `both`, capabilities `registries`) |
| Votes | `native-mod`, format `native-1/0a875da…/e59da90…`, for 26.3 |
| Entries | 64 — 5 root documents, 3 `src/`, 31 `assets/dimstorage/`, 1 `data/c/`, 21 `data/dimstorage/`, 2 `data/minecraft/`, `uses.json`; **no `Cargo.lock`** |
| Reproducible | two builds after `rm -f Cargo.lock` are `cmp`-identical |

## Ladder (all inside a build-slot lease, `CARGO_INCREMENTAL=0`, no `--target-dir`)

| Step | Command | Result | Evidence |
|---|---|---|---|
| tests | `cargo test --offline` | **40 passed**, 0 failed | `../_shared/evidence/dimstorage/sdk-test.log` |
| lints | `cargo clippy --all-targets --offline -- -D warnings` | exit 0, 0 warnings | `../_shared/evidence/dimstorage/sdk-clippy.log` |
| format | `cargo fmt --check` | `fmt_exit=0` | `../_shared/evidence/dimstorage/sdk-fmt.log` |
| build ×2 | `rm -f Cargo.lock && cargo horizon-mod build` | `cmp`-identical, 81,237 B | `sdk-build.log`, `sdk-build-second.log` |
| registry check | `cargo horizon-mod check dimstorage.hmod` | exit 0 | `sdk-check.log` |
| fits (pin) | `cargo horizon-mod fits dimstorage.hmod --build 0a875da` | `0a875da  made for` | `sdk-fits.log` |
| fits (other) | `cargo horizon-mod fits dimstorage.hmod --build 0b14f8a` | `0b14f8a  fits` | `sdk-fits-other.log` |
| package gate | `python3 ports/tools/verify-sdk-package.py dimstorage` | **1/1** | prints the package sha above |
| carried-data gate | `python3 ports/tools/check_261_dialect.py .` | exit 0 | every recipe/loot file in the 26.3 shape |

Every log prints the artifact sha256 it was run against (`sdk-check`, `sdk-fits`, `sdk-fits-other`,
`sdk-build*`) or the source hashes plus the pin (`sdk-test`, `sdk-clippy`).

## In-game leg

`ports/tools/launcher-game.sh build dimstorage` → `Compiling dimstorage (1 of 1)` (the manifest was
bumped, so no stale native reuse), then
`build-slot.sh run --job e2e-dimstorage --port ports/dimstorage --step e2e -- bash tools/e2e.sh`.

| Leg | Result | Log |
|---|---|---|
| scenario | **`PASS` + `EXIT:0`** | `../_shared/evidence/dimstorage-e2e/e2e-launcher.log` |
| control (same Nightly, **no** package) | `EXIT:1` at the craft | `e2e-launcher-control.log` |
| `SELFTEST=1` (chest sharing inverted) | `EXIT:1` | `e2e-launcher-selftest.log` |
| `SELFTEST=tank` (tank sharing inverted) | `EXIT:1` | `e2e-launcher-selftest-tank.log` |
| `check-e2e-logs.py dimstorage-e2e --strict` | **4/4** | — |

The scenario covers: the tank recipe crafted in a crafting table (result id and count, grid emptied),
the crafted item placing the block, a bucket filling a channel shared by its tanks, a lava bucket
staying on its own channel, an empty bucket draining the channel, the 16,000 mB cap refusing the
17th bucket, a survival break dropping the tank into a hopper with the channel's fluid intact, both
loot tables rolling their item, both store files written, and both stores surviving a relaunch —
beside the slice-1 chest scenario (menu deposit/withdraw, channel isolation, vanilla-barrel
differential).

## Known limits at this pin

- The tank has no comparator output, no screen, no waterlogging, no auto-eject and no lock
  (`notes.md` coverage table), and a re-placed chest/tank starts on `public / 1` (no item frequency
  component yet).
- The tank's fluid is a mirrored copy of the channel store, merged once per block-entity tick; a
  tank's own bucket interaction syncs first, a pipe's write lands on the next tick.
- A creative break drops nothing (this is why the drop leg plays in survival); a hopper under a chest
  pulls from the shared channel, which is the mod's intended behaviour.
