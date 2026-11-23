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

Status: **slices 1–3 implemented and verified in-game** — the dimensional chest (its 54-slot
remote inventory, the menu, and persistence across a relaunch), the dimensional tank (one
16,000 mB fluid per frequency, shared across tanks, filled and drained by buckets, capped and
persisted), and the reviewer's tank-transfer fix (one authoritative channel balance, committed
before success; a drained channel protected from stale block data; all-or-nothing fills). The
frequency/owner/lock screen, the tablet, the custom renderer/lid animation, the tank's own screen
and comparator and the remaining cosmetics are deferred (see Gaps). **Package 0.2.4** (pin
`0a875da`) carries all three; `## Slice 3 — the tank-transfer fix (0.2.4)` is the newest record,
`## Slice 2 — the dimensional tank (0.2.3)` the tank, and `## Migration to the SDK format` the
0.2.0 file map.

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

## Review fixes (2026-11-18)

The reviewer's source review of PR #1221 (`var/review-all/1221-dimstorage.md`, **held**) left three
findings; all three are fixed in **0.2.1** (`69500 B`, sha256 `18641dc422a34eadab58d4f049e23f6642a7307bb4513f450765e0a84d4884d6`).

- **`src/support.rs:435` — `channel`, `info`, `put` and `take` were public and the coordinate lookup
  checked only the block identity: a non-operator could retune another chest or withdraw its
  contents.** All four now need physical access or the operator level (`runtime::may_read`, 6
  blocks), `take` additionally needs a recipient who can hold items (a spectator is refused), and
  the slot must be the chest's own (`runtime::take_allowed`). Tests:
  `runtime::tests::only_an_operator_or_a_nearby_player_uses_a_channel_command`; the harness player
  is an ordinary player standing beside each chest (no operator), and the e2e's `info` restores its
  position.
- **`src/runtime.rs:371` — two loaded chests exposed independently writable copies, and `take` gave
  items before updating the store.** The merge is now a **count delta** against the authoritative
  store (`merged_slot`, used by both `sync` and `commit`), a chest's access commits at the point of
  access (`Hooks::contents_changed`) and is pushed into every other loaded chest of the channel at
  once (with a re-entrancy guard), and `take` reserves from the store *before* giving the stack and
  gives it back if the inventory cannot hold it. Tests:
  `runtime::tests::two_chests_each_withdraw_from_one_shared_stack`,
  `…::two_chests_each_insert_into_one_shared_stack`,
  `…::a_sibling_that_has_not_changed_adopts_the_committed_withdrawal`.
- **`src/support.rs:518` — syncing a visited channel materialized a permanent 54-slot entry even
  when empty, so arbitrary signed ids plus public retuning grew the store without bound.**
  `Store::set` drops a channel that holds nothing, refuses a new channel past
  `runtime::MAX_CHANNELS` (4096), and `sync` no longer records an empty unknown channel. Tests:
  `…::an_empty_channel_is_not_retained`, `…::the_store_refuses_more_channels_than_its_cap`. The e2e
  now asserts the empty channel is **absent** from the store file (the previous assertion required
  it to be present).

**In-game leg:** `ports/tools/launcher-game.sh run dimstorage` → `PASS` + `E2E EXIT:0`, control
`EXIT:1`, recorded in `ports/_shared/evidence/dimstorage-e2e/`.

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

## Slice 2 — the dimensional tank (0.2.3)

**The gap it closes**, verbatim from `Gaps / deviations` above:

> 4. **Tank/tablet content is omitted.** Only the chest and the component items its recipe tree
>    needs are carried; the tank/tablet recipes, advancements and assets are removed rather than
>    shipping inert blocks.

The tank is the second half of the mod's own product (chest + tank + tablet) and the one remaining
family that is a **machine**: a block entity, a fluid store shared by frequency, a capability the
game (and pipes) reach, a bucket interaction and a persisted world store. It fits the `machines`
tier exactly; the tablet (remote item access from an item) and the frequency/owner/lock screen
(client UI) stay deferred.

### Behaviour (as upstream, within the pin)

- **One fluid per frequency**, `DimTankStorage.CAPACITY = 16000` millibuckets (sixteen buckets),
  keyed exactly as the chest's store is (`Frequency`, default `public / 1`), and **separate from
  the chest's channels** — upstream keys item and fluid stores by a `type` suffix.
- **Shared across tanks**: every loaded tank of a channel shows the same fluid. The store is the
  truth and each tank **mirrors** it into its own `fluids` saved list once per block-entity tick,
  merged with the tank's last snapshot as an amount delta (the same model the chest uses).
- **Buckets**: right-clicking the tank with any fluid container runs the game's own
  `fluid_util::interact_with_fluid_handler` over the block's `fluids` capability, so the item
  exchange, the `BucketResourceHandler` semantics, the bucket sound and the fluid game event are
  vanilla's (the tank is not a bucket special case). An empty hand answers the channel's contents.
- **Cap**: a fill takes only the free millibuckets; the 17th bucket of water into a full tank is
  refused and the bucket stays full.
- **Break/explosion**: the fluid lives in the store, never in the block, so a broken tank drops
  (its loot table) and a fresh tank on that channel finds the fluid.
- **Commands**: `/dimstorage channel <pos> <n>` now retunes a tank as well as a chest, and
  `/dimstorage info <pos>` prints a tank's line (`DimStorage tank: public / 1 (3000 / 16000 mB of
  minecraft:water)`); both are the port's headless fallback for the deferred tank screen.

### What was added (SDK format)

| Where | What |
|---|---|
| `src/runtime.rs` | `TANK_CAPACITY`, `TankFluid`, `Tank`, `TankStore` (fill/drain/cap/channels/codec), `merged_tank`, `shown_tank`, `tank_sync`, `tank_commit`, `tank_switch` — 9 of the 32 runtime tests, beside the chest's 23 |
| `src/support.rs` | `declare_tank` (`block_fluids` = one 16,000 mB tank on every face), `TankBlock` (the `use_item_on` bucket path), `tank_tick` (mirror + merge), the `fluids` compound read/write, `<world>/dimstorage/tanks.dat`, the `info`/`channel` tank paths |
| `src/lib.rs` | `support::declare_tank(r)` beside the chest |
| `assets/` | the carried tank blockstate/model/item definition/textures, the tank's lang keys |
| `data/` | `horizon/block/dimensional_tank.json` (glass template), recipe, loot table, advancement, creative-tab entry, the pickaxe tag, `c:tags/item/glass_blocks` |

### TDD: red first, then green

The store's rule was written as a test and watched fail twice:

1. The **delta merge**, against the naive absolute write (the defect the chest's review found):

```
---- runtime::tests::two_tanks_each_fill_a_bucket_into_one_shared_tank stdout ----
thread 'runtime::tests::two_tanks_each_fill_a_bucket_into_one_shared_tank' panicked at src/runtime.rs:1268:9:
assertion `left == right` failed: both bucket fills add: 1000 + 1000
  left: Some(TankFluid { id: "minecraft:water", amount: 1000 })
 right: Some(TankFluid { id: "minecraft:water", amount: 2000 })

---- runtime::tests::a_tank_that_has_not_changed_adopts_a_sibling_drain stdout ----
thread 'runtime::tests::a_tank_that_has_not_changed_adopts_a_sibling_drain' panicked at src/runtime.rs:1288:9:
assertion `left == right` failed
  left: Some(TankFluid { id: "minecraft:water", amount: 2000 })
 right: Some(TankFluid { id: "minecraft:water", amount: 1000 })

test result: FAILED. 35 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out
```

2. A **real defect the in-game leg found** — a sibling tank mirroring the channel wrote the amount
   with an empty fluid id (the id was taken from the local/snapshot copy only, and an untouched
   tank has neither). This is the red that fixed it:

```
thread 'runtime::tests::an_untouched_sibling_adopts_the_channels_fluid_with_its_id' panicked at src/runtime.rs:1346:9:
assertion `left == right` failed
  left: Some(TankFluid { id: "", amount: 1000 })
 right: Some(TankFluid { id: "minecraft:water", amount: 1000 })

test result: FAILED. 39 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

Green after the fix: **40 passed, 0 failed** (`../_shared/evidence/dimstorage/sdk-test.log`).

### Ladder (0.2.3, pin `0a875da91dbb0fdcb7ba8c42c11341aa229af51a`, mixins `e59da909…`)

Every step ran through `ports/tools/build-slot.sh` with `CARGO_INCREMENTAL=0`, local steps with
`BUILD_LOCAL=1` (the SDK project has no staged tree), never a bare `cargo` outside a lease, never
`--target-dir`. `.horizon/sdk` was **deleted** after the repin and the build printed
`SDK 0a875da (build 0a875da91dbb…)`, so the cached surface is the pin's.

| Step | Result | Evidence |
|---|---|---|
| `cargo test --offline` | **40 passed**, 0 failed | `sdk-test.log` |
| `cargo clippy --all-targets --offline -- -D warnings` | exit 0, **0 warnings** | `sdk-clippy.log` |
| `cargo fmt --check` | `fmt_exit=0` | `sdk-fmt.log` |
| build 1 (`rm -f Cargo.lock` first) | `Built ./target/horizon-mod/dimstorage.hmod`, 81,237 B, sha256 `0ef5637c…741c` | `sdk-build.log` |
| build 2 (same, after `rm -f Cargo.lock`) | **`cmp: identical`** | `sdk-build-second.log` |
| `cargo horizon-mod check dimstorage.hmod` | exit 0 | `sdk-check.log` |
| `cargo horizon-mod fits dimstorage.hmod --build 0a875da` | `0a875da  made for` | `sdk-fits.log` |
| `cargo horizon-mod fits dimstorage.hmod --build 0b14f8a` | `0b14f8a  fits` | `sdk-fits-other.log` |
| `python3 ports/tools/verify-sdk-package.py dimstorage` | **1/1**, runtime byte-identical inside the package | — |
| `python3 ports/tools/check_261_dialect.py .` | exit 0, every recipe/loot file in the 26.3 shape | — |

`check`, verbatim:

```
dimstorage 0.2.3 (native-mod, format native-1/0a875da91dbb0fdcb7ba8c42c11341aa229af51a/e59da9094f0c2fdda6c627fe9eff22dec6a906f0, for 26.3)
  81237 bytes, SHA-256 0ef5637cb09c57f989f275c2cc067006f0403e382ef540e94fd12192913d741c, unsigned
  native source: unsandboxed; not executed by the registry check. Compile and test against the exact pinned Horizon and Mixins revisions before review.
```

`fits`, verbatim:

```
DimStorage:
0a875da  made for
```

```
DimStorage:
0b14f8a  fits
```

### In-game leg (launcher-built game, `launcher-game.sh build dimstorage`)

`Compiling dimstorage (1 of 1)` in the build log (the manifest was bumped, so the launcher did not
reuse the previous binary), then `build-slot.sh run --step e2e -- tools/e2e.sh`:
`PASS` + `E2E EXIT:0` in `../_shared/evidence/dimstorage-e2e/e2e-launcher.log`; the control on the
same Nightly **without** the package ends `EXIT:1`, `SELFTEST=1` (the chest assertion inverted)
`EXIT:1`, `SELFTEST=tank` (the tank assertion inverted) `EXIT:1`;
`check-e2e-logs.py dimstorage-e2e --strict` → **4/4**.

Quoted from `e2e-launcher.log`:

```
== the tank recipe crafts in a crafting table ==
  result slot: {"count":1,"item":"dimstorage:dimensional_tank","slot":200}
  the grid is empty after the craft (every ingredient was consumed)
  dimensional tanks in the inventory: 1
  block at 2,100,12: {"block":"dimstorage:dimensional_tank","loaded":true,"pos":[2,100,12],"state":"dimstorage:dimensional_tank"}
== the tank: channel 1 shares a bucket, channel 2 holds its own fluid ==
  info(0,100,8): DimStorage tank: public / 1 (0 / 16000 mB, empty)
  info(0,100,8): DimStorage tank: public / 1 (1000 / 16000 mB of minecraft:water)
  info(4,100,8): DimStorage tank: public / 1 (1000 / 16000 mB of minecraft:water)
  info(8,100,8): DimStorage tank: public / 2 (0 / 16000 mB, empty)
  info(20,100,8): DimStorage tank: public / 3 (0 / 16000 mB, empty)
  channel 1 shared the bucket; channel 2 and the unused channel 3 stayed empty
== an empty bucket drains the channel; a second fluid stays on its channel ==
  info(0,100,8): DimStorage tank: public / 1 (0 / 16000 mB, empty)
  water buckets after the drain: 1 (the drained bucket came back full)
  info(8,100,8): DimStorage tank: public / 2 (1000 / 16000 mB of minecraft:lava)
  a lava bucket filled only the channel-2 tank
== a tank channel caps at sixteen buckets ==
  info(0,100,8): DimStorage tank: public / 1 (16000 / 16000 mB of minecraft:water)
  water buckets around the refused 17th fill: 1 -> 1
  info(0,100,8): DimStorage tank: public / 1 (16000 / 16000 mB of minecraft:water)
== breaking a tank drops it (survival) and leaves the store intact ==
  has(2,99,12) dimstorage:dimensional_tank x1: ok
  hopper(2,99,12)=[{"count":1,"item":"dimstorage:dimensional_tank"}]
  info(0,100,8): DimStorage tank: public / 1 (16000 / 16000 mB of minecraft:water)
  has(0,100,0) minecraft:diamond x7: ok
  a fresh tank found the channel's water after the break
== every carried loot table rolls its own item ==
  dimstorage:blocks/dimensional_chest: Dropped 1 [Dimensional Chest]
  dimstorage:blocks/dimensional_tank: Dropped 1 [Dimensional Tank]
== both store files exist and both stores survive a relaunch ==
  inventories: f public,1 f public,2
  tanks: t public,1 minecraft:water 16000 t public,2 minecraft:lava 1000
  info(0,100,8): DimStorage tank: public / 1 (16000 / 16000 mB of minecraft:water)
  info(8,100,8): DimStorage tank: public / 2 (1000 / 16000 mB of minecraft:lava)
PASS: the tank recipe crafted and placed, same-channel tanks shared one fluid while another channel held its own, an empty bucket drained the channel, a channel capped at 16000 mB and refused the 17th bucket, a broken tank dropped into a hopper with its fluid intact, same-frequency chests shared 54 slots, a vanilla barrel never shared, and both stores survived a relaunch
EXIT:0
```

Fail-capable: the control (the same script on the Nightly without the package) fails at the craft
(`the crafting table offered {"item":null,"slot":200}`) and exits 1; `SELFTEST=1` fails at
`expected an empty container at 4,100,0; got [{"count":8,"item":"minecraft:diamond"}]`; `SELFTEST=tank`
fails at `expected … '(0 / 16000 mB, empty)', got 'DimStorage tank: public / 1 (1000 / 16000 mB of
minecraft:water)'`. The differential controls inside the pass: a lava bucket on channel 2 leaves
channel 1 empty, an unused channel 3 stays empty and unrecorded, and the vanilla barrel still never
shares.

Two harness facts this leg cost (for the next slice): a **creative** break drops nothing, so the drop
leg switches to survival with a pickaxe; and a **hopper under a chest pulls from the shared channel**
(correct mod behaviour), so a chest is never broken over a hopper in a test that wants the channel
intact — the loot tables are rolled with `/loot give` instead. A crafting grid slot takes the whole
carried stack on a left click; one item per right click.

### Coverage (family → how it is proven)

| Family (from the jar) | Status |
|---|---|
| `dimensional_chest` block + item | `tested` — placed, read back, ids unknown without the package (control leg) |
| chest 54-slot menu, deposit/withdraw | `tested` — menu deposit, right-click split, withdraw, shared across chests |
| chest frequency isolation | `tested` — channel 2/3 chests stay separate; vanilla barrel differential |
| chest recipes (4) | `tested` — the chest's own tree was carried in slice 1; this slice's craft proof is the tank's, and slice 1's four recipes remain unproven in game (recorded gap) |
| `dimensional_tank` block + item | `tested` — crafted, placed by its item, read back |
| tank recipe (`c:glass_blocks`, cauldron) | `tested` — crafted in a crafting table, result id + count, grid emptied |
| tank fluid store, sharing, cap, drain | `tested` — bucket in/out, sibling mirror, 16,000 mB cap, 17th refused |
| tank frequency isolation | `tested` — lava on channel 2, water on channel 1, channel 3 unused |
| tank loot table / block loot tables | `tested` — survival break into a hopper (tank) and `/loot give` for both tables |
| tank drop keeps the fluid | `tested` — re-placed tank finds channel 1's water |
| tag `minecraft:mineable/pickaxe` | `tested` — `/execute if block … #minecraft:mineable/pickaxe` → `Test passed`, and the survival drop |
| `c:glass_blocks` tag | `tested` — the recipe matched with `minecraft:glass` |
| persistence (`inventories.dat`, `tanks.dat`) | `tested` — both files asserted line by line, then a relaunch |
| `/dimstorage channel`, `/dimstorage info` | `tested` — parsed reply lines for chests and tanks |
| tank comparator output | `not ported` — no `analog_output` part: `LevelHost` in `analog_output` cannot read a block entity at a position, and the readout would need a cached (pos → channel) table; upstream returns `amount / 1000` |
| tank screen / GUI, custom tank renderer | `not ported` — `machines`/`client` boundary: the port ships the command fallback, not a screen |
| tank waterlogged state | `not ported` — the glass template has no `waterlogged` property; upstream is `SimpleWaterloggedBlock` |
| tank auto-eject, tank lock | `not ported` — upstream features outside this slice |
| `dimensional_tablet`, owner/lock/frequency screen, custom chest renderer | `not ported` — deferred as before (client UI / item-remote-access) |
| item `dimstorage:frequency` component | `not ported` — a re-placed chest/tank starts on public/1; the tank's loot table drops the `copy_components` function for that reason |

### Deviations this slice adds

1. **A separate tank file.** `<world>/dimstorage/tanks.dat` (`t public,1 minecraft:water 16000`)
   beside `inventories.dat`, where upstream keeps both in one `SavedData`. Same rename-and-debounce
   write; the item store's format is untouched, so 0.2.1 worlds load.
2. **The mirror staleness**, as the chest's gap 7: a pipe or a bucket writes the tank's copy between
   ticks; the tick merges it. A tank's own `use_item_on` syncs the channel *before* the handler reads
   it, so a bucket never drains fluid a sibling already took in that window.
3. **No waterlogging, a full-cube collision box**, and no tank screen/comparator/auto-eject (table
   above). The carried model is a 12×16×12 shell but the block collides as the glass template's
   cube; upstream's `box(2,0,2,14,16,14)` is not expressible in a content block declaration at this
   pin.
4. **Fluid ids are the pin's**: a tank holds what the game's fluid handler accepted, so a fluid this
   build does not know cannot be stored (the same limit the chest's item ids have).

### Upstream defects fixed

- **The tank advancement showed the tablet's text.** `data/dimstorage/advancement/dimensional_tank.json`
  names `advancements.dimstorage.dimensional_tablet` / `.desc` for its title and description, and the
  lang file repeats the copy-paste (`advancements.dimstorage.dimensional_tank = "Dimensional Tablet"`,
  `.desc = "A remote inventory"`). The port points the advancement at the **tank's own** keys and
  gives them the tank's English. A user notices the tank's advancement reading "Dimensional Tank".
  No `ctl` readout exposes an advancement toast at this pin, so this is a data fix evidenced by the
  files, not an in-game assertion.
- **`tank` recipe tree in the 1.21.1 dialect** (a carry defect, not the mod's): the recipe's
  ingredients were `{"item": …}` objects and the loot pool used the `"conditions": [ … ]` array and
  `bonus_rolls`/`rolls: 1.0`. Converted to bare ids / `#tag`, a singular `"condition"` and
  `"rolls": 1` — `check_261_dialect.py` exit 0, and both tables proven in game.
- **`#c:glass_blocks` has no 26.3 tag**, so the carried recipe could never match. The port ships
  `data/c/tags/item/glass_blocks.json` with vanilla's glass family (the NeoForge conventional tag
  the recipe was written against), and proves the craft with `minecraft:glass`.
- **The tank block was in no `minecraft:mineable/*` tag** while declaring `requires_tool: true`
  (the measured "no drop" defect): added to `minecraft:mineable/pickaxe`, proven by the survival
  drop into the hopper.

### Changelog text (0.2.3, for the publisher)

> **0.2.3 — the dimensional tank.** The second half of the mod: `dimstorage:dimensional_tank`, one
> 16,000 mB fluid per frequency, shared by every tank on its channel in any dimension and persisted
> in `<world>/dimstorage/tanks.dat`. A bucket fills or drains it through the game's own fluid handler
> (vanilla sound, game event and container semantics); the 17th bucket is refused at the cap; a
> broken tank drops and the channel keeps its fluid. `/dimstorage channel` retunes a tank,
> `/dimstorage info` prints its contents. The recipe crafts in a crafting table (with the
> `c:glass_blocks` tag it was written against), the loot table and the pickaxe tag are carried, and
> the tank's advancement no longer shows the tablet's text. Chest behaviour is unchanged; pin
> `0a875da` / mixins `e59da90`.

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-port

## Slice 3 — the tank-transfer fix (0.2.4)

**The review this closes.** `NikitaMGrimm` reviewed the staged 0.2.3 archive/source at `91a7f390`
and accepted three defects (all three are fixed here; the orchestrator's brief carries the text):

1. **One authoritative channel balance.** Bucket and capability transfers acted on separate
   per-tank mirrors, so fluid could be lost or duplicated, and removing a tank before its next sync
   could lose a completed transfer. The transfer must commit against a single authoritative balance
   for the channel **before** it reports success.
2. **Stale block data must not restore a drained channel after a restart**, and a capacity
   rejection must propagate **before** the transfer is accepted (not after part of it landed).
3. **Regression tests** for sibling transfers (two tanks in one channel), removal of a tank before
   the next tick, restart after draining, and capacity rejection; written red first.

This is a behaviour fix on the `machines` tier's existing tank capability — no new family, no
renderer, no client half.

### What changed

- **Commit at the point of access (`src/support.rs`).** `tank_use` still lets the game's fluid
  handler move the bucket over the block's `fluids` capability, but it now records the tank's
  contents *before*, and after a successful handler call commits the delta to the channel through
  `runtime::tank_transfer` and writes the channel's value back into the tank's own compound — all
  before returning `Success`. A completed transfer therefore no longer waits for a block-entity
  tick, and a tank removed after it cannot take the transfer with it.
- **A persisted `synced` marker** (`{Fluid, Amount}` beside `fluids` in the tank's saved compound).
  It is what the channel held when this tank last synced; the delta against it is a foreign
  capability transfer (a pipe, or a crash between the handler and the tick). It replaces the
  in-memory per-uid snapshot, which a restart lost.
- **Known-channel markers (`TankStore`).** A drained channel drops its fluid entry but keeps
  `e <frequency>` in `tanks.dat`; a tank with contents and **no** `synced` marker may seed only a
  channel the world has never known (a lost store file). A known, emptied channel is never re-seeded
  by a stale saved tank — the restart the review names. The marker set is bounded by the same
  `MAX_CHANNELS` as fluid entries.
- **All-or-nothing fills (`TankStore::transfer_in`).** A fill that does not fit is refused whole,
  and `tank_move` checks the held container against the channel's free room (and an empty
  container against the channel's fluid) **before** the handler runs, so a capacity refusal leaves
  both sides untouched. The click is consumed with a message so the held bucket never tries the
  world.
- **Break/explosion commit for tanks.** `Hooks::break_block` and `explosion_detonate` now commit a
  tank's delta to its channel before the block (and its compound) is removed — the tank path joins
  the chest path.
- `tanks.dat` is version `2` (the `e` lines); 0.2.2/0.2.3 files decode unchanged. A tank saved by
  0.2.3 has no `synced` marker and is treated as before (adopt the known channel, or bootstrap an
  unknown one), so upgrading does not lose or duplicate fluid.

### TDD: the four red tests, verbatim

Added to `src/runtime.rs` before the glue; the first implementation of `tank_transfer`/`transfer_in`
kept today's model (the transfer stays in one tank's copy; a fill takes what fits), and all four
failed for their stated reason. Quoted from `../_shared/evidence/dimstorage/sdk-test-transfer-red.log`:

```
---- runtime::tests::a_fill_that_does_not_fit_is_refused_before_any_of_it_lands stdout ----
thread '...a_fill_that_does_not_fit_is_refused_before_any_of_it_lands' panicked:
a fill that does not fit is refused whole

---- runtime::tests::a_restart_does_not_restore_a_drained_channel_from_a_stale_tank stdout ----
assertion `left == right` failed: a drained channel is not re-seeded by a stale tank
  left: Some(TankFluid { id: "minecraft:water", amount: 1000 })
 right: None

---- runtime::tests::a_transfer_into_a_tank_removed_before_its_next_tick_is_not_lost stdout ----
a finished transfer is committed at once

---- runtime::tests::a_transfer_through_one_tank_is_in_the_channel_before_its_sibling_reads_it stdout ----
the access commits before it answers

test result: FAILED. 40 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out
```

The test bodies are the contract: a drain through tank A is in the channel before sibling B reads
it; a fill into a tank removed before its tick is not lost; a save/load of a drained channel plus a
stale tank stays empty; 15,500 mB + a 1,000 mB bucket is refused whole.

### Ladder (0.2.4, pin `0a875da91dbb0fdcb7ba8c42c11341aa229af51a`, mixins `e59da909…`)

Every step ran through `ports/tools/build-slot.sh` with `CARGO_INCREMENTAL=0`; the SDK project has
no staged tree, so local steps used `BUILD_LOCAL=1`. The build printed `SDK 0a875da (build
0a875da91dbb…)`.

| Step | Result | Evidence |
|---|---|---|
| `cargo test --offline` | **45 passed**, 0 failed (41 `runtime::`, 4 `support::`) | `sdk-test.log` |
| `cargo clippy --all-targets --offline -- -D warnings` | exit 0, **0 warnings** | `sdk-clippy.log` |
| `cargo fmt --check` | `fmt_exit=0` | `sdk-fmt.log` |
| build 1 (`rm -f Cargo.lock` first) | 86,836 B, sha256 `02cc3b1c…33ed` | `sdk-build.log` |
| build 2 (same) | **`cmp: identical`** | `sdk-build-second.log` |
| `cargo horizon-mod check dimstorage.hmod` | exit 0, `dimstorage 0.2.4 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)` | `sdk-check.log` |
| `cargo horizon-mod fits … --build 0a875da` | `0a875da  made for` | `sdk-fits.log` |
| `cargo horizon-mod fits … --build 0b14f8a` | `0b14f8a  fits` | `sdk-fits-other.log` |
| `python3 ports/tools/verify-sdk-package.py dimstorage` | **1/1** (package sha, runtime byte-identical, no `Cargo.lock`, tests, clippy, check, fits, skills) | `sdk-verify-0.2.4.log` |

Artifact: **86,836 B**, SHA-256 `02cc3b1c937686ba6a4e8f1929ca470f6d91fe5cecdc597fbd20c58a5b6333ed`, **64
entries** (same layout as 0.2.3: 5 root documents, 3 `src/`, 31 `assets/dimstorage/`, 1 `data/c/`,
21 `data/dimstorage/`, 2 `data/minecraft/`, `uses.json`), **no `Cargo.lock`**.

### In-game leg (launcher-built game, `launcher-game.sh build dimstorage`)

`Compiling dimstorage (1 of 1)` in `../_shared/evidence/dimstorage-e2e/launcher-build-0.2.4.log`
(the manifest was bumped), then
`build-slot.sh run --job W1-dimstorage --port ports/dimstorage --step e2e -- bash tools/e2e.sh`.
The scenario now reads the block's **own saved compound** with `/data get block … fluids` as well
as the store/capability readout, and it freezes the clock with `/tick freeze` so the sibling tank is
genuinely removed before its next tick:

```
== a tank channel caps at sixteen buckets ==
  block compound before the 17th fill: 0,100,8hasthefollowingblockdata:[{Amount:16000,Fluid:"minecraft:water"}]
  water buckets around the refused 17th fill: 1 -> 1
  info(0,100,8): DimStorage tank: public / 1 (16000 / 16000 mB of minecraft:water)
== a drain is committed at access; a stale sibling cannot restore it ==
  water buckets after the frozen drain: 1
  block compound before the relaunch: 0,100,8hasthefollowingblockdata:[{Amount:15000,Fluid:"minecraft:water"}]
  info(0,100,8): DimStorage tank: public / 1 (15000 / 16000 mB of minecraft:water)
  block compound after the relaunch: 0,100,8hasthefollowingblockdata:[{Amount:15000,Fluid:"minecraft:water"}]
  the channel stayed at 15000 mB after removing the stale sibling and relaunching
```

The main leg ends `PASS` + `EXIT:0` (`e2e-launcher.log`). The control (the same script on the
Nightly **without** the package) fails at the craft and exits 1 (`e2e-launcher-control.log`);
`SELFTEST=1` exits 1 at the chest sharing assertion (`e2e-launcher-selftest.log`), `SELFTEST=tank`
at the sibling tank (`-selftest-tank.log`), `SELFTEST=capacity` at the refused 17th bucket
(`-selftest-capacity.log`), and `SELFTEST=transfer` at the drained channel's compound
(`-selftest-transfer.log`). `check-e2e-logs.py dimstorage-e2e --strict` → **6/6**.

The freeze is what makes the second assertion fail-capable: without it, the sibling ticks within
50 ms of the drain and its compound is no longer stale. The store file after the run carries
`t public,1 minecraft:water 15000` (no resurrection) and the drained bucket is in the inventory.

### Coverage (families this slice touches)

| Family | Status |
|---|---|
| tank bucket fill/drain through the capability | `tested` — 16 fills to the cap, a refused 17th (bucket and compound unchanged), a drain through one tank read from the block's own compound and the store |
| tank transfer atomicity (sibling, removal, restart, capacity) | `tested` — the four runtime tests above; in game the frozen-clock drain + removed sibling + relaunch |
| sibling sharing / channel isolation | `tested` — unchanged from slice 2 (two tanks, lava channel 2, unused channel 3) |
| tank break/explosion commit | `tested` — survival break into a hopper keeps channel 1 at 15,000 mB and the fresh tank finds it; the explosion path shares `commit_tank` with the chest's |
| tank recipe, loot tables, pickaxe tag, `c:glass_blocks`, persistence, commands | `tested` — unchanged, still green in the same run |
| tank comparator, screen, waterlogging, auto-eject, lock | `not ported` — unchanged from slice 2 |

### Deviations / notes this slice adds

1. **`tanks.dat` gains `e <frequency>` lines** for known channels that hold nothing (bounded by
   `MAX_CHANNELS`). 0.2.2/0.2.3 files load unchanged; a downlevel binary would skip the `e` lines
   and could resurrect a drained channel, so this is forward-only.
2. **The tank's compound carries `synced`** beside `fluids` (the last channel value this tank saw).
   A 0.2.3 tank without it is upgraded in place on its first tick/access.
3. **`/setblock` removal bypasses the break hook** (as it does for the chest): a pending pipe
   transfer in a tank deleted that way is lost. A player break and an explosion commit first.
4. **A refused fill now answers a message** ("The channel has no room for that much fluid." / "The
   channel does not hold enough fluid for that.") and consumes the click; upstream silently
   returned a partial fill. This is the pre-move refusal the review asked for.
5. The pre-check reads the held container's own fluid handler (`ITEM_FLUIDS`), so it works for any
   container the game registers, not just vanilla buckets.

### Changelog text (0.2.4)

> **0.2.4 — the tank transfer is one authoritative channel balance.** A bucket or any capability
> transfer now commits to the channel before it reports success, so a sibling can never serve the
> same fluid twice and a tank removed before its next tick cannot lose a completed transfer. A
> drained channel is protected by a persisted known-channel marker and each tank's `synced` delta:
> a stale saved compound cannot restore it after a restart. A fill that does not fit the channel is
> refused whole, before anything moves. Four regression tests cover sibling transfers, removal
> before the tick, restart after draining and capacity rejection. Chest behaviour is unchanged;
> pin `0a875da` / mixins `e59da90`.

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-review
