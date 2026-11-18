# DimStorage — a Horizon port

**DimStorage** is a Minecraft mod. DimStorage add a technologic version of ender chest system with owner, frequence selection and lock system. This repository is a **port of it to [Horizon](https://github.com/horizon-rust/horizon)**, the Rust engine for Minecraft Java 26.3. It carries the complete source of the port plus the content files it ships, and installs as a single `.hmod` package.

Upstream: [Edivad99/DimStorage](https://github.com/Edivad99/DimStorage) — licence **AGPL-3.0-only**.

| | |
|---|---|
| Upstream mod | DimStorage ([source](https://github.com/Edivad99/DimStorage)) |
| Ported to | Horizon 0b14f8a, Minecraft Java 26.3, mod API `0.2` |
| Package | `dimstorage.hmod` (install it with the Horizon launcher) |
| Licence | **AGPL-3.0-only** — the upstream mod's licence; see [Licence and credit](#licence-and-credit) |

## What this port implements

- **Store** (`runtime.rs`): `Store<ItemStack>` keyed by `Frequency`; 54 slots;
- **Merge** (`sync`): the first tick after a chest loads adopts the store,
- **Menu**: 54 slots reachable from every side, shown as
- **Break/explosion**: commit the chest's last change and empty its slots before
- **Commands** (`/dimstorage channel|info|put|take`): the port-owned fallback
- **The store loads on the first server tick, not on an open phase.** There is no
- **The store's item lines are hex network NBT** (`i <slot> hex:<bytes>`). `Slot::encode`/`decode`
- **No log lines.** A native mod may not add a crate and the SDK exports no logger, so a corrupt

## Not yet ported

- **Frequency is public-only** in normal play. The store and block entity keep
- **Custom screen is replaced by the vanilla 9×6 menu.** The carried
- **No custom renderer or lid animation.** The chest is the carried cube
- **Tank/tablet content is omitted.** Only the chest and the component items
- **No item frequency component.** A broken chest drops a plain item; the
- **Map colour is 33 (metal), not the exact Java `MapColor.METAL` object;**
- **`Store` is a mirrored copy, not a shared object.** Each chest carries a
- **A crash can lose up to one second of store changes** (debounced write).

## Installing

1. Install the [Horizon launcher](https://github.com/horizon-rust/horizon/releases) (0.2.70 or newer).
2. Add `dimstorage.hmod` to the instance's mods.
3. The package is pinned to a specific Horizon build; the launcher checks that pin.

## Licence and credit

DimStorage was created by its upstream authors; the code, models and assets in the original mod
remain theirs and are used here under **AGPL-3.0-only**. The Rust in this repository is a clean-room
rewrite for Horizon, not a translation of the original source.

- `LICENSE` — the upstream licence text.
- `LICENSE-NOTICE.md` — what this port carries and where it came from.
- The upstream project: https://github.com/Edivad99/DimStorage

## Technical details

```
horizon-mod.toml   package manifest (id, version, api, [native] pin)
Cargo.toml         the crate; depends on `horizon-mod-api` from the fetched SDK
src/               lib.rs (the one `impl Hooks`), runtime.rs (pure logic + tests), glue
assets/ data/      the content the package ships
tools/             content generators and the e2e scenario
```

- Build: `cargo horizon-mod build` (packages; it does not compile against the game).
- Test: `cargo test`, `cargo clippy --all-targets -- -D warnings`.
- In-game: `ports/tools/launcher-game.sh run dimstorage` in the campaign tree, or install the
  `.hmod` and play; the harness scenario is `tools/e2e.sh`.

This repository is generated from the campaign's port at `ports/dimstorage/`; the port's own
`notes.md` is the full record (decisions, traps, evidence).
