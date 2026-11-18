# DimStorage — upstream findings

**Mod:** DimStorage 9.1.1, Minecraft Java 1.21.1 NeoForge, mod id `dimstorage`.
**Licence:** AGPL-3.0-only (the jar's `META-INF/neoforge.mods.toml` and the
repository `LICENSE`). Campaign rank **1** (copyleft): publishable under the
same licence with the complete corresponding source; local-only until the
maintainer approves publishing (ARCHITECTURE.md §8).

## Provenance

- ATM10 Crash Assistant export:
  `https://raw.githubusercontent.com/AllTheMods/ATM-10/main/config/crash_assistant/modlist.json`
  — key `DimStorage-1.21.1-9.1.1.jar`, `modId: dimstorage`,
  `version: 9.1.1`, `curseForgeHash: 3196259727`,
  `modrinthHash: b2b89b7ce8f44c957935e26399cbdafc186b6b9b`.
- Modrinth version id `R7rQFgQh`; file
  `DimStorage-1.21.1-9.1.1.jar`, size **179,454 B**, SHA-1
  `b2b89b7ce8f44c957935e26399cbdafc186b6b9b`, SHA-256
  `430e34540916cae401f8c18fe92c59d4dfa96f8e86cea3e14346bb1d2f9191db`.
- Upstream analysis: `Edivad99/DimStorage`, tag `9.1.1`, commit
  `0c7cd55151b73dd7a69fb1b02c6259e980c46c1d`; `gradle.properties` says
  `minecraft_version=1.21.1`, `mod_version=9.1.1`.
- The Java source was read for behaviour only; the Rust is a clean-room
  rewrite.

## Storage model

`DimStorageManager` is a `SavedData` (`dimstorage.inventories`) on the server.
It keys a storage by `Frequency + ",type=" + type` (`type` is `item` or
`liquid`). `DimChestStorage` is a `Container` with **54 slots** (6×9). Its
`ItemStack[]` is saved as `items` under the frequency key; every mutating
`Container` call marks the storage dirty, and `DimStorageManager.save` writes
the dirty frequencies into the `inventory` compound. The data is world-global,
so two chests with one frequency share their 54 slots across dimensions.

A `Frequency` is `Optional<GameProfile>` (owner) plus an `int channel`;
the default is **public, channel 1** (`new Frequency()`). `canAccess(player)`
is true for a public frequency or the owner's UUID.

## Block and interaction

`dimensional_chest` is a `BaseEntityBlock` with `BlockEntityDimChest`:

- `Properties.of().mapColor(MapColor.METAL).sound(SoundType.METAL)
  .requiresCorrectToolForDrops().strength(3.5F).noOcclusion()`.
- A block entity, saved `frequency`, `locked`, `rot` (placement yaw) and the
  open-count state; it synchronises frequency/lock/rotation to clients.
- `useItemOn` opens a `DimChestMenu` (54 storage slots + the player's 36).
  A locked chest refuses an item handler; the menu itself is not gated by
  `locked`.
- A comparator reads the storage's fullness. Breaking empties/removes the
  block entity and does not spill the shared storage.
- `dimensional_tank`, the `dimensional_tablet` (remote access/autocollect),
  the owner/lock/frequency buttons, the custom chest renderer/lid animation,
  auto-eject and the config are separate features.

## Slice 1 scope

The port implements the **dimensional chest** only:

- one block/item, 54 slots, a `minecraft:generic_9x6` menu;
- a world-global store keyed by public frequency (default channel 1);
- mirroring the store into each block entity's slots with a three-way merge,
  because Horizon gives every mod block entity its own slots;
- persistence in `<world>/dimstorage/inventories.dat`, atomic `.tmp` rename,
  at most once a second and on world close/stop;
- comparator/hoppers/break through the normal mod-container path;
- `/dimstorage channel <pos> <channel>` and `/dimstorage info <pos>` as a
  fallback for the deferred frequency screen, plus `put`/`take` for headless
  checks.

Everything else is a recorded gap in `notes.md`.

## Carry notes

`horizon-port-pack check` on the exact jar produced 2 blocks, 4 items, 6
recipes, 2 loot tables and 12 textures, guessing `minecraft:stone` for both
blocks. Slice 1 keeps the chest plus its three component items and their four
recipes. The block declaration is patched to `minecraft:barrel` (the template
supplies a block entity and the use-to-open path) with the upstream metal
properties; the blockstate covers the template's `facing`/`open` states. The
carry's tank/tablet blocks, recipes, advancements and assets are removed. The
patch is reproducible with
`ports/_shared/carry-work/dimstorage/patch-content.py`.
