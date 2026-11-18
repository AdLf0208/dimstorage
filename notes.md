# DimStorage — port notes

Ported from **DimStorage 9.1.1** (`Edivad99/DimStorage`, **AGPL-3.0-only**),
tag `9.1.1`, commit `0c7cd55151b73dd7a69fb1b02c6259e980c46c1d`, Minecraft Java
1.21.1 NeoForge. The exact ATM10 jar is `DimStorage-1.21.1-9.1.1.jar`
(SHA-1 `b2b89b7c…b9b`, SHA-256 `430e3454…91db`), provenance in
[`SOURCE.md`](../_shared/carry-work/dimstorage/SOURCE.md). Upstream source was
read for behaviour only; this is a clean-room Rust rewrite.

**LOCAL-ONLY** until the maintainer approves publishing (ARCHITECTURE.md §8).
AGPL-3.0 permits redistribution under the same licence with the complete
corresponding source; it is not a publishing bar.

Status: **slice 1 implemented and verified in-game** — the dimensional chest,
its 54-slot remote inventory, the menu, and persistence across a relaunch.
Tank, tablet, owner/lock/frequency screen, custom renderer and the remaining
cosmetics are deferred (see Gaps). **Package 0.2.0** republishes the same
behaviour as one flat SDK `.hmod` (supersedes the closed `.hnmod` PR #804); see
`## Migration to the SDK format` at the end for the file map, the hook map, the
0.2.0 ladder and the one new gap (no launcher, so no 0.2.0 in-game run).

## Class + tier and approach

**Class:** content · **tier:** machines (`ports/ledger.csv`, source-map row
`dimstorage`). This is a **machines** agent port, not a data carry: a block
entity, a container/menu, item-slot reconciliation and world-backed state.
The content half is the exact carried chest/component assets and recipes.

## Behaviour (slice 1)

Upstream keeps one `DimChestStorage` (54 slots, 6×9) per `Frequency`
(`Optional<GameProfile>` + integer channel, default public/1) in a
world-global `SavedData`; every chest with that frequency opens the same
`Container`, and it is persisted with the world. Horizon gives every mod block
entity its own slots, so this port keeps the store as the truth and **mirrors**
it into each chest once per block-entity tick:

- **Store** (`runtime.rs`): `Store<ItemStack>` keyed by `Frequency`; 54 slots;
  deposit tops up equal stacks then uses empty slots; withdrawal splits a slot;
  the capacity is 54 stacks. It persists as
  `<world>/dimstorage/inventories.dat` (`f public,1` plus `i <slot> …` item
  lines, atomic `.tmp` + rename, at most once a second and on close/stop). The
  0.1.0 lines were SNBT; the SDK package writes the stack's network NBT in hex
  (`i 0 hex:…`) because the codec takes no host — see the migration section.
- **Merge** (`sync`): the first tick after a chest loads adopts the store,
  unless the frequency is unknown and the chest has items (then the chest
  bootstraps the store, so a lost store file does not wipe the chest).
  Afterwards it is a slot-wise three-way merge with the chest's last snapshot:
  a local change is written to the store, a remote change is adopted, and a
  slot changed on both sides keeps the local value.
- **Menu**: 54 slots reachable from every side, shown as
  `minecraft:generic_9x6`; hoppers, comparators and the normal click/shift-click
  path use the vanilla menu engine. The block is a `barrel` template with the
  upstream metal properties; the carried blockstate covers `facing`/`open`.
- **Break/explosion**: commit the chest's last change and empty its slots before
  removal, so the store's only copy does not spill on the ground; the chest item
  drops through the normal loot table (no copy of the inventory).
- **Commands** (`/dimstorage channel|info|put|take`): the port-owned fallback
  for the deferred frequency/owner screen and for headless checks. `channel`
  queues the new frequency for the chest's next tick and commits the old one;
  `put`/`take` exercise the same runtime insert/extract model.

## Hooks (the 0.1.0 record)

This is the retired `.hnmod` map, kept for the historical evidence. The 0.2.0
mapping — where each of these went — is the helper→hook table in
`## Migration to the SDK format`.

| Hook (API) | Helper | What it does |
|---|---|---|
| `block_entity` (15) | `block_entity.rs` | Create the chest's block entity. |
| `block_entity_ticks` (15) | `ticks.rs` | The chest ticks. |
| `block_entity_container` (16/17) | `container.rs` | 54 slots, all faces, 9×6 menu. |
| `block_entity_tick` (17) | `tick.rs` | Apply a queued frequency, then merge with the store. |
| `destroy_block` (14) | `destroy.rs` | Commit and empty (no spill). |
| `explosion_detonate` (22) | `explosion.rs` | Same commit/empty for a blasted chest. |
| `world_lifecycle` (3) | `world.rs` | Load/save the store with the world. |
| `server_tick` (4) | `server_tick.rs` | Debounced store write. |
| `server_stopping` (44) | `stopping.rs` | Flush the store. |
| `register_commands` (8) | `register.rs` | `/dimstorage channel|info|put|take`. |
| `perform_command` (8) | `perform.rs` | The command bodies. |

All 11 injections resolve against the pin's stamped `native-hooks.json`
(432 hooks / 174 helpers); no pending fragment is named.

## Slice boundary

In: one `dimstorage:dimensional_chest` block + item, its three component items
and four recipes, the 54-slot public-frequency store, the menu, comparator /
hopper reach through the container, break/explosion accounting, the store file,
and the command fallback.

Out (next slices): `dimensional_tank` (fluid store/capability), the
`dimensional_tablet` (remote item access, autocollect), the owner/lock/frequency
GUI, the custom chest renderer/lid animation and rotation display, auto-eject,
upgrades/size tiers, the config file, and the item `dimstorage:frequency`
component (so a broken chest re-places on the default channel).

## Gaps / deviations

1. **Frequency is public-only** in normal play. The store and block entity keep
   an optional owner, but there is no owner/lock screen and no item component;
   `/dimstorage channel` is the current way to change a chest's channel.
2. **Custom screen is replaced by the vanilla 9×6 menu.** The carried
   `dimstorage:textures/gui/dimchest.png` and the frequency/owner/lock widgets
   are not used in slice 1.
3. **No custom renderer or lid animation.** The chest is the carried cube
   model/textures; upstream's animated `BlockEntityRenderer` and `rot` display
   are not reproduced.
4. **Tank/tablet content is omitted.** Only the chest and the component items
   its recipe tree needs are carried; the tank/tablet recipes, advancements and
   assets are removed rather than shipping inert blocks.
5. **No item frequency component.** A broken chest drops a plain item; the
   store itself survives. A later slice can add the component and carry the
   channel through placement.
6. **Map colour is 33 (metal), not the exact Java `MapColor.METAL` object;**
   the block uses the barrel's state shape (facing/open). In 0.2.0 the block
   entity type is declared in code (`Registrar::block_entity_type`), not
   inherited from the template.
7. **`Store` is a mirrored copy, not a shared object.** Each chest carries a
   snapshot and reconciles per tick; a hopper/menu write that happens inside
   the same tick as a remote write can tie-break to the local chest. This is
   the same trade-off the EnderStorage port records.
8. **A crash can lose up to one second of store changes** (debounced write).
9. **The menu title is the block's name**; the frequency is in
   `/dimstorage info` and the action-bar feedback, not the title.
10. **No cross-dimension-specific test.** The store is world-global in memory
    and by key, so two chests in different dimensions on one channel share by
    construction; the e2e exercises one dimension.

## Upstream source analysis

- Repository: <https://github.com/Edivad99/DimStorage> (AGPL-3.0-only), tag
  `9.1.1`, commit `0c7cd55151b73dd7a69fb1b02c6259e980c46c1d`.
- Files read: `api/Frequency.java`, `api/AbstractDimStorage.java`,
  `manager/DimStorageManager.java`, `storage/DimChestStorage.java`,
  `plugin/DimChestPlugin.java`, `blockentities/BlockEntityDimChest.java`,
  `blockentities/BlockEntityFrequencyOwner.java`, `blocks/DimChest.java`,
  `blocks/DimBlockBase.java`, `menu/DimChestMenu.java`,
  `menu/DimStorageMenu.java`, `items/ItemDimBase.java`,
  `items/components/DimStorageComponents.java`, `setup/Registration.java`,
  `setup/Config.java`, `datagen/Recipes.java`, the blockstate/model/lang
  resources and the jar's recipes/loot/tags.
- Behaviours extracted: 54 shared slots per frequency; default public/1;
  owner optional; saved in a world `SavedData`; menu 54 + 36 slots; comparator
  from fullness; break removes the block entity without spilling; metal block
  strength 3.5 + correct tool; tank/tablet and the frequency/owner/lock screens
  are separate features.
- Rewrite differences: mirrored per-tick copy instead of a shared `Container`;
  its own line-based world file instead of the `SavedData` compound; vanilla
  9×6 menu instead of the custom screen; no renderer/animation; command
  fallback for the missing GUI.

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-port

## Verification

Pin **`1d63e0a5a37a7a708eb0b32b49bf0dfeb28ed8a9`**, mixins
**`5a64af938ec86b00d53d16a408aabc0038fb3e57`** (schema 6); checkouts
`ports/_shared/pins/horizon-1d63e0a` / `horizon-mixins-5a64af9`. Every staged
command ran through `ports/tools/build-slot.sh` with `CARGO_INCREMENTAL=0`;
no bare cargo, no `--target-dir`, no private slot directory. The pin's one
online `cargo fetch --locked` had already been done; the ladder used
`--locked --offline` afterwards.

### Ladder

| Step | Command (via the slot runner) | Result |
|---|---|---|
| runtime test | `cargo test --locked --offline` | **16 passed**, 0 failed |
| runtime clippy | `cargo clippy --all-targets --locked --offline -- -D warnings` | exit 0, 0 warnings |
| format | `cargo fmt --all -- --check` | exit 0 |
| stage | `python3 tools/horizon.py stage --horizon …/pins/horizon-1d63e0a` | exit 0, revision `1d63e0a5…`, **12 injections, 2 modules** |
| check | `python3 tools/horizon.py check …` | `cargo_exit: 0`, **0 warnings** (`target/horizon-check.log`) |
| embedded test | `python3 tools/horizon.py test …` (`--lib`) | `test result: ok. **16 passed**; 0 failed; 1240 filtered out` |
| build | `python3 tools/horizon.py build …` (fast) | `cargo_exit: 0` |
| e2e | `build-slot.sh e2e -- … -- tools/e2e.sh` | **PASS**, `EXIT:0` |
| selftest | `SELFTEST=1 … tools/e2e.sh` | **exit 1** at the deliberately wrong sharing assertion |
| embed ladder (this slice) | `cargo fmt --check`; `rustfmt --check mixins/dimstorage/embedded_content.rs`; `cargo clippy … -D warnings`; `cargo test`; `tools/horizon.py check/test/build` | 0 warnings, **16 passed**, `cargo_exit: 0` on all three |
| package | `tools/package.sh` (content `.hmod` → `data/` → `.hnmod` → native check) | `.hmod` 19,818 B; `.hnmod` 31,789 B; native check exit 0 |
| e2e, one file only | `EMBEDDED=1 CONTROL=1 … tools/e2e.sh` | **PASS**, `EXIT:0`; the control shows the ids unknown (`e2e-embedded.log`) |
| embed fail-capability | `EMBEDDED=1 SELFTEST=1 … tools/e2e.sh` | **exit 1** (`e2e-embedded-selftest.log`) |

Evidence: [`ports/_shared/evidence/dimstorage/`](../_shared/evidence/dimstorage/)
(`runtime-test.log`, `clippy.log`, `stage.log`, `check.log`,
`embedded-test.log`, `build.log`, `e2e.log`, `e2e-selftest.log`,
`hmod-load.log`, `package.log`, plus this slice's `embed-ladder.log`,
`embed-ladder-first-attempt.log` — the run where `tools/package.sh` was not yet
executable — `e2e-lease.log`, `e2e-embedded.log`, `e2e-embedded-selftest.log`).

### In-game scenario (quoted from `e2e.log`)

```
== launch (Overworld arena, content + native bundle) ==
== four chests start empty; channels 2 and 3 are separate ==
  info(0,100,0): DimStorage: public / 1 (0/54 slots, 0 items)
  info(8,100,0): DimStorage: public / 2 (0/54 slots, 0 items)
  info(16,100,0): DimStorage: public / 3 (0/54 slots, 0 items)
  empty(0,100,0): ok
  empty(4,100,0): ok
  empty(8,100,0): ok
  empty(16,100,0): ok
== deposit 8 diamonds through the 54-slot menu ==
menu slots: 54
  has(4,100,0) minecraft:diamond x8: ok
  has(0,100,0) minecraft:diamond x8: ok
  empty(8,100,0): ok
  empty(16,100,0): ok
== withdraw one diamond through the menu ==
  carried after take: 8
  carried after placing one: 7
  has(0,100,0) minecraft:diamond x7: ok
  has(4,100,0) minecraft:diamond x7: ok
  diamond in the inventory: 1
== a channel-2 chest is isolated ==
  has(8,100,0) minecraft:emerald x3: ok
  has(0,100,0) minecraft:emerald x0: ok
  has(8,100,0) minecraft:diamond x0: ok
  empty(16,100,0): ok
== a vanilla barrel never shares ==
  has(12,100,0) minecraft:stick x7: ok
  has(0,100,0) minecraft:stick x0: ok
  has(8,100,0) minecraft:stick x0: ok
  has(12,100,0) minecraft:diamond x0: ok
  has(12,100,0) minecraft:emerald x0: ok
== the store file exists and survives a relaunch ==
  store: f public,1 f public,2 f public,3
  has(0,100,0) minecraft:diamond x7: ok
  has(4,100,0) minecraft:diamond x7: ok
  has(8,100,0) minecraft:emerald x3: ok
  empty(16,100,0): ok
  info(0,100,0): DimStorage: public / 1 (1/54 slots, 7 items)
  info(8,100,0): DimStorage: public / 2 (1/54 slots, 3 items)
PASS: same-frequency chests shared 54 slots, deposits and withdrawals went
through the menu, a channel-2 chest stayed isolated, an empty channel-3 chest
stayed empty, a vanilla barrel never shared, and all three stores survived a
relaunch
EXIT:0
```

The e2e is fail-capable: `SELFTEST=1` asserts the wrong thing (the shared
chest is empty) and fails at `expected an empty container at 4,100,0; got
[{"count":8,"item":"minecraft:diamond"}]` with exit 1. The vanilla barrel is
the differential control (its sticks never appear in the store), and the
channel-2/channel-3 chests are the isolation controls.

### Packages and load check

| Artifact | Size | SHA-256 | Entries |
|---|---:|---|---:|
| `content/dimstorage.hmod` (content) | **19,818 B** | `ba6409aeac87168c8cc2f5715c8e885f21a96e1876da51105ed9761fbb612cc3` | 44 |
| `dimstorage.hnmod` (native, content embedded) | **31,789 B** | `a475afb1c31b326a9882d60aef18db907a1be765b6da858a088b8137ec4d1873` | 17 |

`cargo-horizon-mod check` reports `dimstorage 9.1.1 (mod, API 0.2, both)`,
content only. `tools/hmod-load.sh` on the shared unmodded `1d63e0a` game:
`PASS: dimstorage.hmod registered dimstorage:dimensional_chest on a real game,
and the same ids are unknown without the package` (`EXIT:0`); the negative
control's `setblock` replies `Unknown block type
'dimstorage:dimensional_chest'`. The `.hnmod` carries `horizon-native.json`
with `horizon: 1d63e0a5…`, `mixins: 5a64af93…`, schema 1, plus
`mixins/dimstorage/data/dimstorage-content.hmod`;
`cargo horizon-mod check --kind native-mod` reports `native-mod, format
native-1/1d63e0a5…/5a64af93…, for 26.3`, 31,789 B, the same SHA-256
(`../_shared/evidence/dimstorage/package.log`).

## Embedded content (AE2 pattern) — one listable file

The registry keeps **one file per project** (playbook §1.0), so a port with both
halves could not be listed faithfully. This bundle now carries its content,
exactly as the merged AE2 entry does:

- **`mixins/dimstorage/data/dimstorage-content.hmod`** is the built
  `content/dimstorage.hmod`, byte for byte (19,818 B,
  `ba6409aeac87168c8cc2f5715c8e885f21a96e1876da51105ed9761fbb612cc3`).
  `tools/package.sh` (new for this slice) builds the content mod, copies it in,
  and `tools/package.py` packs `data/**` as `mixins/<id>/data/…` with fixed zip
  timestamps, refusing a copy that is not byte-identical to
  `content/<id>.hmod`. It also now writes the `.hnmod` to the **port root**
  (it used to land in `target/`, which is disposable).
- **`mixins/dimstorage/embedded_content.rs`** with the `embedded-content`
  `[[injection]]` (`target = "horizon_client::game::mods::content_root"`,
  `kind = "HEAD"`, `cancellable = true`, bindings `root`/`enabled`/`dev`)
  compiles the file in with `native_data!` (API 41), writes it to
  `.horizon-cache/dimstorage-embedded-content.hmod` when the copy on disk
  differs, appends it to the dev-mod list and returns its `content_root`; the
  one-level `dev.contains(&file)` guard makes the recursive call run the vanilla
  body, as AE2 does. An unwritable cache logs a warning and `Continue`s.

Verified in game with **only the bundle** (`EMBEDDED=1`, no `--dev-mod`): the
whole scenario passes (54 shared slots, the menu deposit/withdraw, channels 2/3
isolated, the vanilla barrel sharing nothing, all three stores surviving a
relaunch) and the file the helper wrote is byte-identical to
`content/dimstorage.hmod`. The control (`CONTROL=1`) launches the same binary
with `.horizon-cache` replaced by a plain file: the chest item never arrives and
`setblock … dimstorage:dimensional_chest` leaves `minecraft:air`.

```
== control: the same game with no embedded content file ==
  without the embedded file: no chest item, the block stayed minecraft:air
  launch: no --dev-mod; the bundle carries its own content
  embedded content: 19818 B at /tmp/opencode/dimstorage-home.kj6gNn/.horizon-cache/dimstorage-embedded-content.hmod, identical to content/dimstorage.hmod
  store: f public,1 f public,2 f public,3
PASS: same-frequency chests shared 54 slots, … and all three stores survived a relaunch
EXIT:0
```

The same scenario with `SELFTEST=1` (still one file) ends `EXIT:1`, so the
one-file run is fail-capable. `check-e2e-logs.py dimstorage --strict` → 4/4;
`upstream_ready.py --copyleft dimstorage` → **READY**.

This makes the `.hnmod` the **single deliverable** of this port: one file
installs the dimensional chest, its models, textures, recipes and lang **and**
the block entity, the 54-slot container, the frequency store and `/dimstorage`.
It does not by itself publish it (licence approval and the AGPL reviewer note
are the publisher's step, ARCHITECTURE.md §8).

The template-only `tools/test_template.py` fixture is not a real-bundle test
(it hard-codes the example `break_block` hook and one injection); the
authoritative staging check is `tools/horizon.py check/test` above.

## Review repairs (PR #596, 2026-11-11)

Review: `ports/_shared/horizon-mods/reviews/pr-596-dimstorage.md` (CHANGES_REQUESTED, 0 bullet items).

The review's only actionable item is publication: the pinned artifact is not in the
registry release/staging path (**orchestrator** — no write access). Its other remark is a
reviewer limitation (they may not read AGPL source), not a defect: the port is
AGPL-3.0-only like the original, with the complete corresponding source in the repository.

This job re-checked the entry's text (link absolute, gaps listed, licence stated) and
repackaged the archive with the notice inside it (the MIT-style packager change applies to
every port in this batch): `dimstorage.hnmod` **55,853 B**
`84ec05de3be5c9f787531cfe1253967d77639dad9c84a14d3814ef1754cc185f`.

## Migration to the SDK format (0.2.0, 2026-11-18)

Horizon's mod-API merge (#16941, launcher ≥ 0.2.70, PR #17992) retired `.hnmod`: the registry
closed every `.hnmod` submission on 2026-11-18 with one reason, and an `.hnmod` no longer loads.
This port's former submission, **PR #804** (`dimstorage` 0.1.0), was closed by that purge. **0.2.0**
is the same behaviour republished as one flat SDK `.hmod`; the changelog text is below.

Pins: SDK nightly **`0b14f8acfd97fa2c750e42795249dcc981178ca4`** (2026-11-18, `package_format` 6,
`content_api` "0.2"), mixins **`e59da9094f0c2fdda6c627fe9eff22dec6a906f0`** (was `5a64af93…`),
launcher 0.2.78, `cargo-horizon-mod` from `horizon-rust/horizon@main`. The SDK is cached in-tree at
`../_shared/sdk/0b14f8a/` and fetched into `.horizon/sdk/` (gitignored) by the build. Licence
re-read from the jar for this record (`../_shared/evidence/dimstorage/sdk-licence.log`):
`DimStorage-1.21.1-9.1.1.jar` declares **GNU AGPLv3**, effective AGPL, rank 1, matching the ledger.

### The two `.hmod`s that were already beside the old layout

The port root held a **19,818 B** `dimstorage.hmod` (44 entries, no `LICENSE`) while `content/`
held a **31,827 B** one (45 entries, `LICENSE` inside). Measured before retiring either
(`../_shared/evidence/dimstorage/sdk-retired-artifacts.log`):

- the root file is byte-identical to `content/target/horizon-mod/dimstorage.hmod` — a **stale
  pre-notice build** of the 0.1.0 content half;
- `content/dimstorage.hmod` is byte-identical to `mixins/dimstorage/data/dimstorage-content.hmod`,
  the file the `.hnmod` actually embedded — the **0.1.0 content half** as shipped.

Neither is a 0.2.0 package. Both were moved to
`../_shared/evidence/dimstorage/retired-hnmod-0.1.0/` (`dimstorage-content-0.1.0-stale.hmod`,
`content-old/dimstorage.hmod`) before the first SDK build. The package root now carries `assets/`
and `data/` directly, so **one** `dimstorage.hmod` (built from this crate) is the deliverable.

### What moved, and where the retired tree is

| 0.1.0 path | Now | Evidence |
|---|---|---|
| `mixins/dimstorage/runtime.rs` | `src/runtime.rs`, **copied with `cp`, `cmp`-checked** | `retired-hnmod-0.1.0/mixins/dimstorage/runtime.rs` |
| `mixins/dimstorage/support.rs` + 12 hook helpers | `src/support.rs` (glue) + `src/lib.rs` (the one `Hooks`) | `retired-hnmod-0.1.0/mixins/dimstorage/` |
| `mixins/dimstorage/horizon-mixin.toml` (12 `[[injection]]`, 2 `[[module]]`) | no plan: `Registrar` declarations and typed `Hooks`/Part methods | `retired-hnmod-0.1.0/mixins/` |
| `mixins/dimstorage/data/dimstorage-content.hmod` + `embedded_content.rs` (AE2 pattern) | gone: the package carries `assets/` and `data/` itself | `retired-hnmod-0.1.0/mixins/` |
| `content/horizon-mod.toml`, `content/dimstorage.hmod` | the port root's `horizon-mod.toml`; one package | `retired-hnmod-0.1.0/content-old/` |
| port-root `dimstorage.hmod` (19,818 B, stale) | not needed; **stale content build** | `retired-hnmod-0.1.0/dimstorage-content-0.1.0-stale.hmod` |
| `dimstorage.hnmod` (55,853 B) | `dimstorage.hmod` (63,207 B, native + content) | `retired-hnmod-0.1.0/dimstorage.hnmod` |
| `compatibility.json`, `rust-toolchain.toml`, `horizon-native.json` | `[native]` in `horizon-mod.toml`; the pinned commit decides the toolchain | `retired-hnmod-0.1.0/` |
| `tools/horizon.py`, `package.py`, `package.sh`, `test_stage.py`, `test_template.py`, `notice.py`, `hmod-load.sh`, `tools/audit/`, `.github/` | deleted: staging, `.hnmod` packaging and the plugin-injection audit belong to the retired format | `retired-hnmod-0.1.0/tools/`, `retired-hnmod-0.1.0/.github/` |
| the old repository `AGENTS.md` (it says to open `compatibility.json` and run `tools/horizon.py`) | retired: it describes the retired format | `retired-hnmod-0.1.0/AGENTS.md` |
| `VALIDATION.md` (the 0.1.0 ladder) | retired; this section carries the 0.2.0 ladder | `retired-hnmod-0.1.0/VALIDATION-0.1.0.md` |
| `Cargo.lock` | not kept: it is packed and changes the artifact (trap 6) | not kept |

**Kept and updated:** `tools/e2e.sh` (the scenario and its assertions are format-independent; the
launch line now expects a launcher-built game), `notes.md`, `README.md`, `FINDINGS.md`, `LICENSE`,
and the new `LICENSE-NOTICE.md`.

### File map (0.2.0)

| Path | What it is |
|---|---|
| `horizon-mod.toml` | the only manifest and identity: `id = "dimstorage"`, `version = "0.2.0"`, `api = "0.2"`, `side = "both"`, `capabilities = ["registries"]`, `[native]` pinned to `0b14f8a`/`e59da90` |
| `Cargo.toml` | one crate, `edition = "2024"`; its only dependency is `horizon-mod-api` from `.horizon/sdk/` |
| `src/lib.rs` | the one `#[mod_api::horizon_mod] impl Hooks`: `register`, `register_commands`, `run_mod_command`, `break_block`, `explosion_detonate`, `world_lifecycle`, `server_tick`, `server_stopping` |
| `src/support.rs` | all the game glue: `StoredItem` (the store's slot), the core store, the store file, the chest's Part, break/explosion, the four commands. 6 unit tests |
| `src/runtime.rs` | the pure truth, **byte-identical** (`430c51cd…8037`), 16 tests |
| `assets/`, `data/` | the carried content, moved from `content/`: 43 files, every one unchanged (size + sha256 diff empty, `../_shared/evidence/dimstorage/sdk-content-carried.log`) |
| `LICENSE`, `LICENSE-NOTICE.md` | the AGPL-3.0 text and the provenance notice, **inside** the package |
| `tools/e2e.sh` | the in-game scenario, for a launcher-built game |
| `dimstorage.hmod` | the deliverable (63,207 B, sha256 `5cfee46f…0657`) |

### Helper → hook table (every 0.1.0 injection has a home; no gap)

Each former `[[injection]]` helper is now a method or a declaration; the helper file itself is gone.

| 0.1.0 injection (hook, API) | 0.2.0 |
|---|---|
| `embedded-content` (`content_root` HEAD, AE2 pattern) | gone — the flat `.hmod` carries `assets/` and `data/` |
| `[[module]] runtime` (`runtime.rs`) | `pub mod runtime;` — the same file, `src/runtime.rs` |
| `[[module]] state` (`support.rs`) | `src/support.rs` |
| `block-entity` (`block_entity` 15) | `support::Chest::block_entity` → `Some(Compound{frequency = "public,1"})` |
| `block-entity-ticks` (15) | `support::Chest::block_entity_ticks` → `event.ticks = true` |
| `block-entity-container` (16/17) | `Registrar::block_container(&chest, ModContainer { 54 slots, all six faces, "minecraft:generic_9x6" })` in `Hooks::register` |
| `block-entity-tick` (17) | `support::Chest::block_entity_tick` → `support::tick_chest` |
| `destroy-block` (`destroy_block` 14) | `Hooks::break_block` → `support::break_block` |
| `explosion-detonate` (22) | `Hooks::explosion_detonate` → `support::explosion_detonate` |
| `world-lifecycle` (3) | `Hooks::world_lifecycle` → `support::world_lifecycle` |
| `server-tick` (4) | `Hooks::server_tick` → `support::server_tick` |
| `stopping` (`server_stopping` 44) | `Hooks::server_stopping` → `support::server_stopping` |
| `register-commands` (8) | `Hooks::register_commands` → `support::declare_commands` |
| `perform-command` (8) | `Hooks::run_mod_command` → `support::run_command` (typed `CommandInvocation`, not a split line) |

The block entity **type** is a declaration, not a hook: `block_entity_type("dimensional_chest")
.blocks(&["dimensional_chest"]).build()` plus `block_entity_part(&kind, Chest)`. The chest block's
content JSON stays as carried (barrel template, facing/open); there is no data-file block entity
declaration to add.

`uses.json` in the package records **345 surface entries**: 10 hooks (`block_entity`,
`block_entity_ticks`, `block_entity_tick`, `register_commands`, `run_mod_command`, `break_block`,
`explosion_detonate`, `world_lifecycle`, `server_tick`, `server_stopping`) with their fields, 14
`Hooks`/`BlockEntityPart` methods, 5 part kinds, both macros (`horizon_mod`, `horizon_part`), 26
traits, 82 handles and 159 types, and the `registries` capability
(`../_shared/evidence/dimstorage/sdk-uses-from-package.json`, `sdk-uses-summary.log`).

### The ladder (all inside a build slot, `CARGO_INCREMENTAL=0`, no `--target-dir`)

`build-slot.sh` still speaks the retired tool, so there is **no staging step and no staged tree**:
the SDK crate is compiled in place against `.horizon/sdk/`. Every command ran in
`ports/dimstorage` under `BUILD_LOCAL=1 ports/tools/build-slot.sh run --job dimstorage-sdk …`
(see trap 1 for why `BUILD_LOCAL=1`).

| Step | Command | Result |
|---|---|---|
| carried runtime, **before any glue** | `cargo test --offline` | **16 passed**, 0 failed (`sdk-test-runtime-carry.log`) |
| new glue tests proved fail-capable (mutation) | `same_kind` → `true`, menu → `None`; then `settle`'s guard → `if true` | **2 failed** of 21 at both assertions, and **1 failed** of 6 `support::` tests on the deposit settle, then restored (`sdk-test-mutation-red.log`, `sdk-test-mutation-settle-red.log`) |
| tests | `cargo test --offline` | **22 passed**, 0 failed — 16 `runtime::`, 6 `support::` (`sdk-test.log`) |
| lints | `cargo clippy --all-targets --offline -- -D warnings` | exit 0, **0 warnings** (`sdk-clippy.log`) |
| format | `cargo fmt --check` (**not** `--all`) | `fmt_exit=0` (`sdk-fmt.log`) |
| package | `rm -f Cargo.lock && cargo horizon-mod build` | `SDK 0b14f8a … in ./.horizon/sdk`, `Built ./target/horizon-mod/dimstorage.hmod` (`sdk-build.log`) |
| reproducible | the same command again | **byte-identical**, 63,207 B, sha256 `5cfee46f3f88e4f0c0bec4fdefd8af75281ced4421199b9c2865b8bd966c0657` (`sdk-build-second.log`, `sdk-sha256.log`) |
| registry check | `cargo horizon-mod check dimstorage.hmod` | see the quote below, **EXIT 0** (`sdk-check.log`, `sdk-check-json.log`) |
| fits the pinned build | `cargo horizon-mod fits dimstorage.hmod --build nightly` | `DimStorage:` / `0b14f8a  made for`, **EXIT 0** (`sdk-fits.log`) |
| runtime untouched | `cmp` against the retired file **and** `unzip -p dimstorage.hmod src/runtime.rs \| sha256sum` | `430c51cd…8037` in all three places (`sdk-runtime-sha256.txt`) |
| package contents | `unzip -l dimstorage.hmod` | 52 entries: 5 root documents (`horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`), 3 `src/`, 26 `assets/`, 17 `data/`, `uses.json`; **no `Cargo.lock`** (`sdk-entries.log`) |

`check` output, verbatim:

```
dimstorage 0.2.0 (native-mod, format native-1/0b14f8acfd97fa2c750e42795249dcc981178ca4/e59da9094f0c2fdda6c627fe9eff22dec6a906f0, for 26.3)
  63207 bytes, SHA-256 5cfee46f3f88e4f0c0bec4fdefd8af75281ced4421199b9c2865b8bd966c0657, unsigned
  native source: unsandboxed; not executed by the registry check. Compile and test against the exact pinned Horizon and Mixins revisions before review.
```

`fits` output, verbatim:

```
DimStorage:
0b14f8a  made for
```

### Behaviour that changed with the format (deliberate, and why)

- **The store loads on the first server tick, not on an open phase.** There is no
  `WorldPhase::WorldOpenReady` at this pin (only `WorldOpenFailed`, `WorldCloseFinal`,
  `WorldCloseFailed`), so `ensure_world` loads the first tick of a world whose `host.world_dir()`
  differs from the loaded one, and every close phase saves and forgets it. Observable only as: a
  command typed before the first tick of a world sees an empty store.
- **The store's item lines are hex network NBT** (`i <slot> hex:<bytes>`). `Slot::encode`/`decode`
  take no host and the SDK exports neither `to_snbt` nor `parse_snbt` (only `host.item_from_snbt`,
  which needs a host), so the stack's network NBT in hex is the lossless host-free form. The `f`
  lines and the file name are unchanged, so `<world>/dimstorage/inventories.dat` still shows
  `f public,1`.
- **No log lines.** A native mod may not add a crate and the SDK exports no logger, so a corrupt
  store line and a failed write are silent (a failed write keeps the store dirty and is retried on
  the next debounce).
- **`/dimstorage put|take` re-expressed.** `put` reads the main hand through
  `ItemPlace::PlayerHand` and extracts inside one transaction; `take` adds with
  `PlayerRef::give_stack` (a `/give`-style add). The inventory room is checked first, so a full
  inventory still answers "There is no room in the inventory." and nothing is dropped.
- **`/dimstorage info` text is byte-identical**: `DimStorage: public / 1 (1/54 slots, 7 items)`.

### Traps this migration hit (the next port must know)

1. **`build-slot.sh run --port ports/<id>` forwards and `cd`s into the port**, so a wrapped
   `bash -c 'cd ports/<id> && …'` dies remotely with `cd: ports/<id>: No such file or directory`
   and exits 1 before cargo starts (one run). Use `BUILD_LOCAL=1` (the slot lease still counts) or
   drop the inner `cd`.
2. **The store's `deposit` takes the store's own item type.** `Store::deposit(&mut self, f, item:
   &T, …)` with `T = StoredItem`; a `ItemStack` is `E0308` (expected `&StoredItem`). Convert with
   `StoredItem::of(host, &stack)` first — the runtime is byte-identical and cannot grow an impl for
   the game's stack.
3. **Player verbs are on the `PlayerRef` handle**, not on `ServerHost`:
   `player.inventory_slots(host)`, `player.give_stack(host, stack)`; `ServerHost` only exposes the
   `*ServerHost` backend traits (`player_give_stack` is the backend method).
4. **`stack.item().id(&*host)` borrows the host immutably.** `say(host, &format!("… {}", id))`
   is `E0502`; build the `String` first, then `say`.
5. **The block entity type is a code declaration.** The carried block JSON is a barrel template and
   says nothing about block entities; the type comes from
   `Registrar::block_entity_type(...).blocks(...)` + `block_entity_part`, and a `BlockEntityPart`
   also needs an empty `impl BlockPart` beside it.
6. **A packed `Cargo.lock` changes the artifact.** `cargo test`/`clippy` write one; `rm -f
   Cargo.lock` right before `cargo horizon-mod build`, `/Cargo.lock` in `.gitignore`.
7. **`cargo fmt --all` would reformat the fetched SDK**, and `cargo fmt` would touch `runtime.rs` if
   it were unformatted. The runtime was already rustfmt-clean, so only `src/lib.rs` and
   `src/support.rs` were formatted with `rustfmt --edition 2024`; `cmp` + the packaged hash prove
   the runtime is untouched.
8. **Two `.hmod`s existed and only one was the content half.** Identify by size + sha256 + the file
   the `.hnmod` embeds before retiring either (`sdk-retired-artifacts.log`); never ship both.
9. **`cargo horizon-mod check --uses <file>` compiles the SDK project** (the wasm trap) instead of
   reading the package; the uses list is `unzip -p dimstorage.hmod uses.json`.

### Gaps after the migration

Everything in the 0.1.0 feature gaps above still stands (public-only frequencies, the vanilla 9×6
menu instead of the custom screen, no renderer/lid, tank/tablet omitted, no frequency component,
mirrored store, up to one second of loss on a crash). Added by this format:

1. **No in-game re-run in this environment.** A native `.hmod` is built and run by
   `horizon-launcher native-build`, and `horizon-launcher` is not installed here, so no `0b14f8a`
   client exists. The 0.1.0 runs remain the behaviour's evidence
   (`../_shared/evidence/dimstorage/e2e.log`, `e2e-selftest.log`, `e2e-embedded.log`); what changed
   is the transport of the same calls, not the rules, and `src/runtime.rs` is byte-identical.
   `tools/e2e.sh` is updated for a launcher-built game and has **not** been run.
2. **The menu opening from `Registrar::block_container` is unproven in game** at this pin; the
   0.1.0 run proved the old `block_entity_container` hook's declaration opened the 54-slot menu.
3. **A stored item whose id this build does not know is dropped** from the store rather than kept
   (it cannot be shared by id either).

`## Upstream defects fixed`: none found in this migration. The 0.1.0 port's own deviations are
listed above; `tools/package.sh`'s old AE2 write-out is superseded, not a defect.

### Changelog (0.2.0, supersedes #804)

> **0.2.0 — republished as an SDK `.hmod`.** Horizon's mod-API merge (launcher ≥ 0.2.70) retired
> `.hnmod` and closed **#804** (`dimstorage` 0.1.0); this version supersedes it. One flat
> `dimstorage.hmod` now carries the code *and* the content (the dimensional chest block and item,
> the three component items, recipes, loot, tags, advancements, lang, creative tab): `src/lib.rs`
> is the one `impl mod_api::Hooks`, `src/support.rs` the glue, and `src/runtime.rs` is
> byte-identical to 0.1.0, so the frequency store, the three-way merge and the store file behave
> exactly as before. Built against SDK `0b14f8a` / mixins `e59da90` for 26.3. The `.hnmod`, the
> mixin plan, the 12 per-hook helpers and the embedded-content injection are gone; one
> `Registrar::block_container` declaration and a `BlockEntityPart` replace the container and tick
> hooks; `perform_command` became the typed `run_mod_command`; the store loads on the first server
> tick and its item lines are hex network NBT. Same feature set and the same gaps (tank, tablet,
> owner/lock screen, renderer, frequency component).

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-port
