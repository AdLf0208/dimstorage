# DimStorage for Horizon — the chest and the tank

The **dimensional chest**: a 54-slot inventory shared by every chest on the same public
frequency (default `public / 1`), opened through a vanilla 9×6 container menu and persisted in the
world. The **dimensional tank**: one 16,000 mB fluid per frequency, shared by every tank on its
channel, filled and drained with buckets, also persisted; since **0.2.4** every transfer commits
to the channel's one balance before it reports success, a drained channel cannot be restored by a
stale saved tank, and a fill that does not fit is refused whole before anything moves.

- Upstream: DimStorage 9.1.1, `Edivad99/DimStorage`, AGPL-3.0-only.
- Native package version **0.2.4** (supersedes 0.2.3, 0.2.0/0.2.1 and the retired `.hnmod` 0.1.0),
  built against the Horizon SDK nightly `0a875da` / mixins `e59da90` for 26.3.
- Class/tier: content · machines.
- **Local-only** until the maintainer approves publishing (ARCHITECTURE.md §8). The licence
  allows redistribution under AGPL-3.0 with the complete corresponding source.

## Layout

| Path | What it is |
|---|---|
| `horizon-mod.toml` | the manifest and the only identity: `api = "0.2"`, `side = "both"`, the SDK pin under `[native]` |
| `Cargo.toml` | one crate; its only dependency is `horizon-mod-api` from `.horizon/sdk/` |
| `src/lib.rs` | the one `#[mod_api::horizon_mod] impl Hooks` |
| `src/support.rs` | the game glue: the chest's and the tank's block entity Parts, the store files, break/explosion accounting, the bucket path, the commands |
| `src/runtime.rs` | the pure truth (frequency, the 54-slot store, the merged store, the tank store, the two file codecs), with its 41 tests |
| `assets/dimstorage/`, `data/dimstorage/` | the carried content (block, items, models, textures, recipes, loot, tags, advancements, lang, creative tab) |
| `FINDINGS.md` | the upstream behaviour analysis |
| `notes.md` | scope, hook map, gaps, the migration record and the verification evidence |
| `tools/e2e.sh` | the in-game scenario, kept for a launcher-built game |
| `LICENSE`, `LICENSE-NOTICE.md` | the AGPL text and the provenance notice, **inside** the package |

## Build and verify

```sh
# every compile runs inside a campaign build-slot lease, and the command runs in the port
BUILD_LOCAL=1 ports/tools/build-slot.sh run --job dimstorage-sdk --port ports/dimstorage --step test -- \
    bash -c 'cd ports/dimstorage && CARGO_INCREMENTAL=0 cargo test --offline'
BUILD_LOCAL=1 ports/tools/build-slot.sh run --job dimstorage-sdk --port ports/dimstorage --step check -- \
    bash -c 'cd ports/dimstorage && CARGO_INCREMENTAL=0 cargo clippy --all-targets --offline -- -D warnings'
BUILD_LOCAL=1 ports/tools/build-slot.sh run --job dimstorage-sdk --port ports/dimstorage --step build -- \
    bash -c 'cd ports/dimstorage && rm -f Cargo.lock && cargo horizon-mod build'

cargo horizon-mod check dimstorage.hmod
cargo horizon-mod fits dimstorage.hmod --build nightly
```

`cargo fmt --check` (never `--all`: the fetched SDK is a path dependency), `cargo test` and
`cargo clippy --all-targets -- -D warnings` are the compile gate; `cargo horizon-mod build` packs,
it does not compile. The built `.hmod` is copied from `target/horizon-mod/` to this directory —
that file is the deliverable.

The in-game scenario in `tools/e2e.sh` places four chests in an Overworld arena at y=100: two share
`public / 1`, one is on `public / 2`, one on `public / 3`; a vanilla barrel is the differential
control. It deposits 8 diamonds through the menu, withdraws one, checks isolation and the barrel,
then caps the tank channel and refuses the 17th bucket, drains a bucket with the clock frozen,
removes the sibling tank before its next tick and relaunches to prove the channel kept exactly the
drained amount (read from the block's own compound and the store), all before a final relaunch.
`SELFTEST=1|tank|capacity|transfer` flips one assertion each and must fail. The script expects a
**launcher-built** game (`GAME=`, optional `CONTROL_GAME=`), which this machine provides:
`ports/tools/launcher-game.sh build dimstorage` then `run --step e2e`; see `notes.md`.
