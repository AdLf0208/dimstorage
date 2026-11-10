// Game-side glue for DimStorage slice 1 (frequency-linked dimensional chests).
//
// Injected into `crate::game` as `dimstorage_state`; it needs the game, so it
// is not part of the standalone library (only `runtime.rs` is).
//
// Upstream keeps one `DimChestStorage` per frequency and every chest points at
// it. Horizon gives each mod block entity its own slots, so the port keeps the
// store in memory (and in `<world>/dimstorage/inventories.dat`), mirrors it
// into each chest once per tick, and merges the two sides per slot. The store
// file is written at most once a second and on close/stop.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use horizon_client::containers::{BlockEntity, ModBlockEntity, ModContainer};
use horizon_client::items::ItemStack;
use horizon_client::text::Text;
use horizon_world::nbt::{Compound, JavaString, Tag};

use crate::game::dimstorage::{
    Frequency, MAX_STACK, SLOTS, Slot, Stack, Store, commit, switch, sync,
};
use crate::game::native_lifecycle::{ServerTickPhase, WorldPhase};
use crate::game::native_server::{NativeCommand, NativeServerPlayer};

/// The dimensional chest of slice 1 (its item has the same id).
const BLOCK: &str = "dimstorage:dimensional_chest";
/// The store lives beside the world, so backups and the world travel together.
const STORE_DIR: &str = "dimstorage";
const STORE_FILE: &str = "inventories.dat";
/// Write at most once a second; a crash loses at most that much.
const SAVE_INTERVAL_TICKS: u64 = 20;

/// One item slot as the store persists it: the game's `ItemStack.CODEC` as
/// SNBT (`{id:"minecraft:diamond",count:5}`), which is lossless and canonical.
impl Slot for ItemStack {
    fn encode(&self) -> String {
        horizon_world::snbt_text::to_snbt(&horizon_client::save::item_to_nbt(self))
    }

    fn decode(text: &str) -> Option<Self> {
        let (tag, _) = horizon_world::nbt::parse_snbt(text).ok()?;
        horizon_client::save::item_from_nbt(&tag)
    }
}

impl Stack for ItemStack {
    fn count(&self) -> u32 {
        self.count
    }

    fn with_count(&self, count: u32) -> Self {
        let mut stack = self.clone();
        stack.count = count;
        stack
    }

    fn same_kind(&self, other: &Self) -> bool {
        self.item == other.item && self.damage == other.damage && self.extra == other.extra
    }
}

/// The port's state: the store, each chest's last synced slots, and the
/// deferred work a hook cannot do in place.
struct Core {
    world: Option<PathBuf>,
    store: Store<ItemStack>,
    /// Each block entity's last synced slots, by its own uid (it moves with
    /// the block entity, so a piston move does not confuse the merge).
    snapshots: HashMap<u64, Vec<Option<ItemStack>>>,
    /// Frequencies a chest must take from `/dimstorage channel`.
    pending: Vec<(String, [i32; 3], Frequency)>,
    dirty: bool,
    ticks: u64,
    last_save: u64,
}

impl Default for Core {
    fn default() -> Self {
        Core {
            world: None,
            store: Store::new(),
            snapshots: HashMap::new(),
            pending: Vec::new(),
            dirty: false,
            ticks: 0,
            last_save: 0,
        }
    }
}

static CORE: Mutex<Option<Core>> = Mutex::new(None);

fn core() -> std::sync::MutexGuard<'static, Option<Core>> {
    CORE.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// A uid for a block entity, unique across a save: the time and a counter.
fn fresh_uid() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let count = NEXT.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    now.rotate_left(17) ^ count.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn nbt_key(name: &str) -> JavaString {
    JavaString::from(name)
}

/// The chest's frequency from its saved compound; the default when absent or
/// unreadable. Inserts the default so the chest keeps it.
fn be_frequency(entity: &mut ModBlockEntity) -> Frequency {
    if let Some(Tag::String(text)) = entity.nbt.get(&nbt_key("frequency"))
        && let Some(frequency) = Frequency::parse(&text.to_string_lossy())
    {
        return frequency;
    }
    let frequency = Frequency::DEFAULT;
    entity.nbt.insert(
        nbt_key("frequency"),
        Tag::String(JavaString::from(frequency.text().as_str())),
    );
    frequency
}

/// The chest's frequency, read-only (the default when absent).
fn read_frequency(entity: &ModBlockEntity) -> Frequency {
    entity
        .nbt
        .get(&nbt_key("frequency"))
        .and_then(|tag| match tag {
            Tag::String(text) => Frequency::parse(&text.to_string_lossy()),
            _ => None,
        })
        .unwrap_or(Frequency::DEFAULT)
}

fn set_be_frequency(entity: &mut ModBlockEntity, frequency: &Frequency) {
    entity.nbt.insert(
        nbt_key("frequency"),
        Tag::String(JavaString::from(frequency.text().as_str())),
    );
}

/// The chest's uid, made and kept on first use.
fn be_uid(entity: &mut ModBlockEntity) -> u64 {
    if let Some(Tag::Long(uid)) = entity.nbt.get(&nbt_key("uid")) {
        return *uid as u64;
    }
    let uid = fresh_uid();
    entity.nbt.insert(nbt_key("uid"), Tag::Long(uid as i64));
    uid
}

fn state_name(game: &crate::game::Game<'_>, state: horizon_world::registry::BlockStateId) -> String {
    game.res.facts().get(state).name.to_string()
}

fn block_at(game: &crate::game::Game<'_>, pos: [i32; 3]) -> String {
    state_name(game, game.world.block(pos[0], pos[1], pos[2]))
}

/// `BlockEntity.setChanged`: the chunk is written at the next save.
fn mark_modified(game: &mut crate::game::Game<'_>, pos: [i32; 3]) {
    game.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
}

fn store_path(world: &Path) -> PathBuf {
    world.join(STORE_DIR).join(STORE_FILE)
}

fn load_store(world: &Path) -> Store<ItemStack> {
    let path = store_path(world);
    let Ok(text) = fs::read_to_string(&path) else {
        return Store::new();
    };
    let (store, warnings) = Store::decode(&text);
    for warning in warnings {
        log::warn!("[dimstorage] {}: {warning}", path.display());
    }
    store
}

/// Write the store atomically: a `.tmp` beside it, then a rename.
fn save_store(world: &Path) {
    let text = {
        let guard = core();
        let Some(state) = guard.as_ref() else {
            return;
        };
        if !state.dirty {
            return;
        }
        state.store.encode()
    };
    let dir = world.join(STORE_DIR);
    if let Err(error) = fs::create_dir_all(&dir) {
        log::warn!("[dimstorage] could not create {}: {error}", dir.display());
        return;
    }
    let path = store_path(world);
    let temporary = dir.join(format!("{STORE_FILE}.tmp"));
    if let Err(error) = fs::write(&temporary, text.as_bytes()) {
        log::warn!(
            "[dimstorage] could not write {}: {error}",
            temporary.display()
        );
        return;
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        log::warn!(
            "[dimstorage] could not replace {}: {error}",
            path.display()
        );
        return;
    }
    let mut guard = core();
    if let Some(state) = guard.as_mut() {
        state.dirty = false;
        state.last_save = state.ticks;
    }
}

/// Give the chest block a block entity (`native_block_entity`, API 15).
#[allow(private_interfaces)]
pub fn on_block_entity(block: &str, entity: &mut Option<(String, Compound)>) {
    if block == BLOCK {
        let mut nbt = Compound::new();
        nbt.insert(
            nbt_key("frequency"),
            Tag::String(JavaString::from(Frequency::DEFAULT.text().as_str())),
        );
        *entity = Some((block.to_owned(), nbt));
    }
}

/// The chest block entity ticks (`native_block_entity_ticks`, API 15).
#[allow(private_interfaces)]
pub fn on_block_entity_ticks(block: &str, ticks: &mut bool) {
    if block == BLOCK {
        *ticks = true;
    }
}

/// 54 slots reachable from every side, shown as the vanilla 9x6 menu
/// (`native_block_entity_container`, API 16/17).
#[allow(private_interfaces)]
pub fn on_block_entity_container(block: &str, container: &mut Option<ModContainer>) {
    if block != BLOCK {
        return;
    }
    let all: Vec<usize> = (0..SLOTS).collect();
    *container = Some(ModContainer {
        slots: SLOTS,
        faces: [
            all.clone(),
            all.clone(),
            all.clone(),
            all.clone(),
            all.clone(),
            all,
        ],
        inputs: (0..SLOTS).collect(),
        outputs: (0..SLOTS).collect(),
        menu: Some("minecraft:generic_9x6".to_owned()),
    });
}

/// One server tick of a chest: apply a queued frequency, then merge its slots
/// with the store (`native_block_entity_tick`, API 17).
#[allow(private_interfaces)]
pub fn on_block_entity_tick(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    entity: &mut ModBlockEntity,
) {
    if entity.id != BLOCK {
        return;
    }
    let dimension = game.native_dimension().to_owned();
    let mut frequency = be_frequency(entity);
    let uid = be_uid(entity);
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    if let Some(index) = state
        .pending
        .iter()
        .position(|(dim, at, _)| dim == &dimension && *at == pos)
    {
        let (_, _, wanted) = state.pending.remove(index);
        if wanted != frequency {
            let snapshot = state.snapshots.get(&uid).cloned();
            let (items, changed) = switch(
                &entity.items,
                snapshot.as_deref(),
                &mut state.store,
                &frequency,
                &wanted,
            );
            if changed {
                state.dirty = true;
            }
            entity.items = items.clone();
            state.snapshots.insert(uid, items);
            set_be_frequency(entity, &wanted);
            frequency = wanted;
            mark_modified(game, pos);
        }
    }
    let snapshot = state.snapshots.get(&uid).cloned();
    let synced = sync(
        &entity.items,
        snapshot.as_deref(),
        &mut state.store,
        &frequency,
    );
    if synced.store_changed {
        state.dirty = true;
    }
    entity.items = synced.items.clone();
    state.snapshots.insert(uid, synced.items);
}

/// A broken chest commits its last changes and empties its slots, so the
/// store's only copy does not spill on the ground (`native_destroy_block`,
/// API 14).
#[allow(private_interfaces)]
pub fn on_destroy_block(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    state: horizon_world::registry::BlockStateId,
    canceled: &mut bool,
) {
    let name = state_name(game, state);
    if *canceled || name != BLOCK {
        return;
    }
    let key = (pos[0], pos[1], pos[2]);
    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&key) else {
        return;
    };
    let frequency = be_frequency(entity);
    let uid = be_uid(entity);
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    let snapshot = state.snapshots.get(&uid).cloned();
    if commit(
        &entity.items,
        snapshot.as_deref(),
        &mut state.store,
        &frequency,
    ) {
        state.dirty = true;
    }
    entity.items = vec![None; SLOTS];
    state.snapshots.remove(&uid);
}

/// A blown-up chest must not spill a second copy of the store
/// (`native_explosion_detonate`, API 22).
#[allow(private_interfaces)]
pub fn on_explosion_detonate(
    game: &mut crate::game::Game<'_>,
    _dimension: &str,
    blocks: &mut Vec<[i32; 3]>,
) {
    let found: Vec<[i32; 3]> = blocks
        .iter()
        .filter(|pos| {
            matches!(
                game.world.block_entities.get(&(pos[0], pos[1], pos[2])),
                Some(BlockEntity::Mod(entity)) if entity.id == BLOCK
            )
        })
        .copied()
        .collect();
    if found.is_empty() {
        return;
    }
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    for pos in found {
        let Some(BlockEntity::Mod(entity)) =
            game.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        else {
            continue;
        };
        let frequency = be_frequency(entity);
        let uid = be_uid(entity);
        let snapshot = state.snapshots.get(&uid).cloned();
        if commit(
            &entity.items,
            snapshot.as_deref(),
            &mut state.store,
            &frequency,
        ) {
            state.dirty = true;
        }
        entity.items = vec![None; SLOTS];
        state.snapshots.remove(&uid);
    }
}

/// The world opened or closed: load or save the store
/// (`native_world_lifecycle`, API 3).
#[allow(private_interfaces)]
pub fn on_world_lifecycle(game: &mut crate::game::Game<'_>, phase: WorldPhase) {
    match phase {
        WorldPhase::WorldOpenReady => {
            let world = game.native_world_dir();
            let store = world.as_deref().map(load_store).unwrap_or_default();
            let mut guard = core();
            let state = guard.get_or_insert_with(Core::default);
            state.world = world;
            state.store = store;
            state.snapshots.clear();
            state.pending.clear();
            state.dirty = false;
            state.ticks = 0;
            state.last_save = 0;
        }
        WorldPhase::WorldCloseFinal | WorldPhase::WorldCloseFailed => {
            let world = {
                let guard = core();
                guard.as_ref().and_then(|state| state.world.clone())
            };
            if let Some(world) = world {
                save_store(&world);
            }
            let mut guard = core();
            if let Some(state) = guard.as_mut() {
                state.world = None;
                state.store = Store::new();
                state.snapshots.clear();
                state.pending.clear();
                state.dirty = false;
            }
        }
        WorldPhase::WorldOpenFailed => {
            let mut guard = core();
            if let Some(state) = guard.as_mut() {
                state.world = None;
                state.store = Store::new();
                state.snapshots.clear();
                state.pending.clear();
                state.dirty = false;
            }
        }
    }
}

/// End of a running tick: debounce the store write (`native_server_tick`,
/// API 4).
#[allow(private_interfaces)]
pub fn on_server_tick(_game: &mut crate::game::Game<'_>, phase: ServerTickPhase) {
    let ServerTickPhase::ServerTickEnd { world_running } = phase else {
        return;
    };
    if !world_running {
        return;
    }
    let (due, world) = {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        state.ticks += 1;
        let due = state.dirty
            && state.ticks.saturating_sub(state.last_save) >= SAVE_INTERVAL_TICKS;
        (due, state.world.clone())
    };
    if due && let Some(world) = world {
        save_store(&world);
    }
}

/// The server is stopping: flush the store (`native_server_stopping`, API 44).
#[allow(private_interfaces)]
pub fn on_server_stopping(_game: &mut crate::game::Game<'_>) {
    let world = {
        let guard = core();
        guard.as_ref().and_then(|state| state.world.clone())
    };
    if let Some(world) = world {
        save_store(&world);
    }
}

/// `/dimstorage channel|info` (`register_commands`, API 8).
#[allow(private_interfaces)]
pub fn on_register_commands(commands: &mut Vec<NativeCommand>) {
    commands.push(NativeCommand::new(
        "dimstorage channel <pos:block_pos> <channel:integer>",
    ));
    commands.push(NativeCommand::new("dimstorage info <pos:block_pos>"));
    commands.push(NativeCommand::new(
        "dimstorage put <pos:block_pos> <count:integer>",
    ));
    commands.push(NativeCommand::new(
        "dimstorage take <pos:block_pos> <slot:integer> <count:integer>",
    ));
}

fn parse_pos(words: &[&str]) -> Option<[i32; 3]> {
    Some([
        words.first()?.parse().ok()?,
        words.get(1)?.parse().ok()?,
        words.get(2)?.parse().ok()?,
    ])
}

/// The `/dimstorage` commands (`perform_command`, API 8). The channel form
/// queues the change for the chest's next tick, as the block entity is not in
/// the world here. This is a port-owned fallback for the deferred frequency
/// screen.
#[allow(private_interfaces)]
pub fn on_perform_command(
    game: &mut crate::game::Game<'_>,
    player: &NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    let words: Vec<&str> = command.split_whitespace().collect();
    match words.as_slice() {
        ["dimstorage", "channel", x, y, z, channel] => {
            *ran = true;
            let Some(pos) = parse_pos(&[*x, *y, *z]) else {
                game.native_feedback(Text::literal(
                    "Usage: /dimstorage channel <x> <y> <z> <channel>",
                ));
                return;
            };
            let Ok(channel) = channel.parse::<i32>() else {
                game.native_feedback(Text::literal("The channel must be an integer."));
                return;
            };
            if block_at(game, pos) != BLOCK {
                game.native_feedback(Text::literal("No dimensional chest there."));
                return;
            }
            let frequency = Frequency::public(channel);
            let dimension = game.native_dimension().to_owned();
            let mut guard = core();
            let state = guard.get_or_insert_with(Core::default);
            state
                .pending
                .retain(|(dim, at, _)| !(dim == &dimension && *at == pos));
            state.pending.push((dimension, pos, frequency.clone()));
            drop(guard);
            game.native_feedback(Text::literal(format!(
                "DimStorage frequency: {}",
                frequency.label()
            )));
        }
        ["dimstorage", "put", x, y, z, count] => {
            *ran = true;
            let Some(pos) = parse_pos(&[*x, *y, *z]) else {
                game.native_feedback(Text::literal(
                    "Usage: /dimstorage put <x> <y> <z> <count>",
                ));
                return;
            };
            let Ok(count) = count.parse::<u32>() else {
                game.native_feedback(Text::literal("The count must be a positive integer."));
                return;
            };
            let frequency = {
                match game.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
                    Some(BlockEntity::Mod(entity)) if entity.id == BLOCK => {
                        read_frequency(entity)
                    }
                    _ => {
                        game.native_feedback(Text::literal("No dimensional chest there."));
                        return;
                    }
                }
            };
            let Some(held) = game.held().cloned() else {
                game.native_feedback(Text::literal("Hold the stack to store."));
                return;
            };
            let take = count.min(held.count);
            if take == 0 {
                game.native_feedback(Text::literal("There is nothing to store."));
                return;
            }
            let (moved, slots) = {
                let guard = core();
                let mut store = guard
                    .as_ref()
                    .map_or_else(Store::new, |state| state.store.clone());
                let left = store.deposit(&frequency, &held.with_count(take), MAX_STACK);
                (take - left, store.slots(&frequency))
            };
            if moved == 0 {
                game.native_feedback(Text::literal("The dimensional chest is full."));
                return;
            }
            let took = game.native_take_items(&held.item, moved);
            if took == 0 {
                game.native_feedback(Text::literal("The held stack could not be taken."));
                return;
            }
            let slots = if took == moved {
                slots
            } else {
                let guard = core();
                let mut store = guard
                    .as_ref()
                    .map_or_else(Store::new, |state| state.store.clone());
                store.deposit(&frequency, &held.with_count(took), MAX_STACK);
                store.slots(&frequency)
            };
            {
                let mut guard = core();
                let state = guard.get_or_insert_with(Core::default);
                state.store.set(&frequency, slots);
                state.dirty = true;
            }
            game.native_feedback(Text::literal(format!("Stored {took} x {}.", held.item)));
        }
        ["dimstorage", "take", x, y, z, slot, count] => {
            *ran = true;
            let Some(pos) = parse_pos(&[*x, *y, *z]) else {
                game.native_feedback(Text::literal(
                    "Usage: /dimstorage take <x> <y> <z> <slot> <count>",
                ));
                return;
            };
            let (Ok(slot), Ok(count)) = (slot.parse::<usize>(), count.parse::<u32>()) else {
                game.native_feedback(Text::literal(
                    "The slot and count must be positive integers.",
                ));
                return;
            };
            if slot >= SLOTS || count == 0 {
                game.native_feedback(Text::literal("No such slot, or a zero count."));
                return;
            }
            let frequency = {
                match game.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
                    Some(BlockEntity::Mod(entity)) if entity.id == BLOCK => {
                        read_frequency(entity)
                    }
                    _ => {
                        game.native_feedback(Text::literal("No dimensional chest there."));
                        return;
                    }
                }
            };
            let stack = {
                let guard = core();
                guard
                    .as_ref()
                    .and_then(|state| state.store.slots(&frequency).get(slot).cloned().flatten())
            };
            let Some(stack) = stack else {
                game.native_feedback(Text::literal("That slot is empty."));
                return;
            };
            let wanted = stack.with_count(count.min(stack.count));
            let leftover = game.native_give_stack(player.uuid, wanted.clone());
            let given = wanted.count - leftover.map_or(0, |left| left.count);
            if given == 0 {
                game.native_feedback(Text::literal("There is no room in the inventory."));
                return;
            }
            {
                let mut guard = core();
                let state = guard.get_or_insert_with(Core::default);
                state.store.withdraw(&frequency, slot, given);
                state.dirty = true;
            }
            game.native_feedback(Text::literal(format!("Withdrew {given} x {}.", wanted.item)));
        }
        ["dimstorage", "info", x, y, z] => {
            *ran = true;
            let Some(pos) = parse_pos(&[*x, *y, *z]) else {
                game.native_feedback(Text::literal("Usage: /dimstorage info <x> <y> <z>"));
                return;
            };
            let Some(BlockEntity::Mod(entity)) =
                game.world.block_entities.get(&(pos[0], pos[1], pos[2]))
            else {
                game.native_feedback(Text::literal("No dimensional chest there."));
                return;
            };
            if entity.id != BLOCK {
                game.native_feedback(Text::literal("No dimensional chest there."));
                return;
            }
            let frequency = read_frequency(entity);
            let (used, total) = {
                let guard = core();
                let store = guard
                    .as_ref()
                    .map_or_else(Store::new, |state| state.store.clone());
                let slots = store.slots(&frequency);
                let total = slots
                    .iter()
                    .flatten()
                    .map(|stack| stack.count())
                    .sum::<u32>();
                (store.count(&frequency), total)
            };
            game.native_feedback(Text::literal(format!(
                "DimStorage: {} ({used}/{SLOTS} slots, {total} items)",
                frequency.label()
            )));
        }
        _ => {}
    }
}
