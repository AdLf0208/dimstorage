# DimStorage for Horizon — slice 1

The **dimensional chest**: a 54-slot inventory shared by every chest on the same public
frequency (default `public / 1`), opened through a vanilla 9×6 container menu and persisted in the
world.

- Upstream: DimStorage 9.1.1, `Edivad99/DimStorage`, AGPL-3.0-only.
- Native package version **0.2.0** (supersedes the retired `.hnmod` 0.1.0 and closed PR #804),
  built against the Horizon SDK nightly `0b14f8a` / mixins `e59da90` for 26.3.
- Class/tier: content · machines.
- **Local-only** until the maintainer approves publishing (ARCHITECTURE.md §8). The licence
  allows redistribution under AGPL-3.0 with the complete corresponding source.

## Layout

| Path | What it is |
|---|---|
| `horizon-mod.toml` | the manifest and the only identity: `api = "0.2"`, `side = "both"`, the SDK pin under `[native]` |
| `Cargo.toml` | one crate; its only dependency is `horizon-mod-api` from `.horizon/sdk/` |
| `src/lib.rs` | the one `#[mod_api::horizon_mod] impl Hooks` |
| `src/support.rs` | the game glue: the chest's block entity Part, the store file, break/explosion accounting, the commands |
| `src/runtime.rs` | the pure truth (frequency, 54-slot store, three-way merge, store file codec), **byte-identical to 0.1.0**, with its 16 tests |
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
then saves and relaunches. `SELFTEST=1` flips the sharing assertion and must fail. The script now
expects a **launcher-built** game (`GAME=`, optional `CONTROL_GAME=`); no such build exists on this
machine (`horizon-launcher` is not installed), so it has not been re-run since 0.1.0 — see
`notes.md`.
