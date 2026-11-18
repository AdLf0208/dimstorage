//! Game-side glue for DimStorage slice 1 (frequency-linked dimensional chests).
//!
//! [`crate::runtime`] is the pure truth: the frequency, the 54-slot [`Store`], the three-way merge
//! and the store file codec, with its tests. This module is the only place that reads the game and
//! writes it back: the chest's saved compound and item slots, the store file beside the world, the
//! break and explosion accounting, and the `/dimstorage` commands.
//!
//! Upstream keeps one `DimChestStorage` per `Frequency` in a world-global `SavedData`, and every
//! chest with that frequency points at the same `Container`. A Horizon mod block entity owns its own
//! slots, so the port keeps the store as the truth and **mirrors** it into each chest once per
//! block-entity tick, merging the two sides per slot (see `notes.md`).
//!
//! The 0.2.0 package replaced the retired `.hnmod` bundle; the mapping from the old hook helpers to
//! this module and the Parts in `lib.rs` is in `notes.md` under `## Migration to the SDK format`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mod_api::{
    BlockEntityPart, BlockEntityRef, BlockEntityTickEvent, BlockEntityTicksEvent, BlockPart,
    BlockState, Command, CommandArgumentValue, CommandInvocation, CommandSource, Compound, Flow,
    Hand, Host, HostTransaction, Identifier, ItemKey, ItemPlace, ItemResource, ItemStack, LevelRef,
    ModContainer, PlayerRef, Registrar, RunModCommandEvent, ServerHost, ServerTickEvent,
    ServerTickPhase, Tag, Text, WorldLifecycleEvent, WorldPhase,
};

use crate::runtime::{Frequency, MAX_STACK, SLOTS, Slot, Stack, Store, commit, switch, sync};

/// The dimensional chest of slice 1 (its item has the same id).
const BLOCK: &str = "dimstorage:dimensional_chest";
/// The block entity type the block declares; it is in this mod's namespace.
const BLOCK_ENTITY: &str = "dimensional_chest";
/// The store lives beside the world, so backups and the world travel together.
const STORE_DIR: &str = "dimstorage";
const STORE_FILE: &str = "inventories.dat";
/// Write at most once a second; a crash loses at most that much.
const SAVE_INTERVAL_TICKS: u64 = 20;
/// The vanilla 9x6 menu the chest's 54 slots open, so hoppers, comparators and shift-clicks work
/// through the vanilla paths.
const MENU: &str = "minecraft:generic_9x6";
/// The main inventory slots a player has, for `/dimstorage take`'s room check.
const MAIN_SLOTS: u32 = 36;

// ---------------------------------------------------------------------------
// The store's slot: one item, with the id a stack is and the saved compound it carries
// ---------------------------------------------------------------------------

/// One stored item: the id the game showed, the count, and the stack's saved compound (components
/// and damage included), exactly as `ItemStack.CODEC` writes it.
///
/// [`Slot::encode`] and [`Slot::decode`] are part of the pure store and take no host, so the text is
/// the stack's **network NBT** in hex: the only lossless, host-free form the SDK offers (a saved item
/// compound, read back with `Tag::try_from`). The `i <slot> hex:<bytes>` lines are the port's own
/// format, as the notes say; the `f <frequency>` lines stay readable.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredItem {
    id: String,
    count: u32,
    nbt: Compound,
}

impl StoredItem {
    /// The store's value for a real stack; `None` for an item whose id this game does not know.
    fn of(host: &dyn Host, stack: &ItemStack) -> Option<Self> {
        let nbt = stack.to_nbt(host)?;
        Some(Self {
            id: stack.item().id(host),
            count: stack.count(),
            nbt,
        })
    }

    /// The stack a stored item stands for, including its components and damage.
    fn to_stack(&self, host: &dyn Host) -> Option<ItemStack> {
        if let Some(stack) = ItemStack::from_nbt(host, &self.nbt) {
            return Some(stack);
        }
        // A compound the game refuses falls back to the bare item and count, so a stored item is
        // never lost to a component this build does not know.
        let key: ItemKey = host.item_key(&Identifier::parse(&self.id).ok()?)?;
        ItemStack::new(key, self.count)
    }

    /// The stack's identity without its count: the id and its components, which is what
    /// `ItemStack.isSameItem` compares for a container.
    fn kind(&self) -> (String, Compound) {
        let mut kind = self.nbt.clone();
        kind.remove(&"count".into());
        (self.id.clone(), kind)
    }
}

impl Slot for StoredItem {
    fn encode(&self) -> String {
        let tag = Tag::from(&self.nbt);
        match Vec::<u8>::try_from(&tag) {
            Ok(bytes) => {
                let mut text = String::with_capacity(4 + bytes.len() * 2);
                text.push_str("hex:");
                for byte in bytes {
                    let _ = write!(text, "{byte:02x}");
                }
                text
            }
            // A compound past the network limits cannot be written; an empty slot is better than a
            // half-written file. `Store::encode` still produces a line, which drops the item.
            Err(_) => "hex:".to_owned(),
        }
    }

    fn decode(text: &str) -> Option<Self> {
        let bytes = unhex(text.strip_prefix("hex:")?)?;
        let Tag::Compound(nbt) = Tag::try_from(bytes).ok()? else {
            return None;
        };
        let id = match nbt.get(&"id".into()) {
            Some(Tag::String(id)) => id.to_string_lossy(),
            _ => return None,
        };
        let count = match nbt.get(&"count".into()) {
            Some(Tag::Int(count)) => u32::try_from(*count).ok()?,
            Some(Tag::Long(count)) => u32::try_from(*count).ok()?,
            None => 1,
            _ => return None,
        };
        (count > 0).then_some(Self { id, count, nbt })
    }
}

impl Stack for StoredItem {
    fn count(&self) -> u32 {
        self.count
    }

    /// `count` items of this kind. The pure store never asks for zero (it splits a slot only
    /// below its count), so a zero is clamped to one rather than producing an impossible stack.
    fn with_count(&self, count: u32) -> Self {
        let count = count.max(1);
        let mut nbt = self.nbt.clone();
        nbt.insert(
            "count".into(),
            Tag::Int(i32::try_from(count).unwrap_or(i32::MAX)),
        );
        Self {
            id: self.id.clone(),
            count,
            nbt,
        }
    }

    fn same_kind(&self, other: &Self) -> bool {
        self.kind() == other.kind()
    }
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let digits = text.as_bytes();
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for pair in digits.chunks(2) {
        let high = hex_digit(pair[0])?;
        let low = hex_digit(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Some(bytes)
}

fn hex_digit(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// A chest's slots as the store keeps them; a stack the game cannot name is dropped (it is not
/// reachable through any id, so it cannot be shared).
fn stored_slots(host: &dyn Host, items: &[Option<ItemStack>]) -> Vec<Option<StoredItem>> {
    items
        .iter()
        .map(|slot| slot.as_ref().and_then(|stack| StoredItem::of(host, stack)))
        .collect()
}

/// The store's slots as a chest's item slots, `SLOTS` long.
fn stacks(host: &dyn Host, items: &[Option<StoredItem>]) -> Vec<Option<ItemStack>> {
    let mut out: Vec<Option<ItemStack>> = items
        .iter()
        .map(|slot| slot.as_ref().and_then(|item| item.to_stack(host)))
        .collect();
    out.resize(SLOTS, None);
    out.truncate(SLOTS);
    out
}

// ---------------------------------------------------------------------------
// The port's state
// ---------------------------------------------------------------------------

/// The port's state: the store, each chest's last synced slots, and the work a hook defers.
struct Core {
    world: Option<PathBuf>,
    store: Store<StoredItem>,
    /// Each chest's last synced slots, by its own uid (it moves with the block entity, so a piston
    /// move does not confuse the merge).
    snapshots: HashMap<u64, Vec<Option<StoredItem>>>,
    /// Frequencies a chest must take from `/dimstorage channel`; the block entity is not in the
    /// world when the command runs.
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

/// The store is world-global (one per frequency, shared by every chest in every dimension), so it is
/// process state rather than a field of one block entity. `Hooks` is `Send + Sync`, and a `static`
/// `Mutex` is the smallest way to keep the one store the game needs.
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

/// The chest's frequency from its saved compound; the default when absent or unreadable. Inserts
/// the default so the chest keeps it.
fn be_frequency(nbt: &mut Compound) -> Frequency {
    if let Some(Tag::String(text)) = nbt.get(&"frequency".into())
        && let Some(frequency) = Frequency::parse(&text.to_string_lossy())
    {
        return frequency;
    }
    let frequency = Frequency::DEFAULT;
    set_be_frequency(nbt, &frequency);
    frequency
}

/// The chest's frequency, read-only (the default when absent).
fn read_frequency(nbt: &Compound) -> Frequency {
    nbt.get(&"frequency".into())
        .and_then(|tag| match tag {
            Tag::String(text) => Frequency::parse(&text.to_string_lossy()),
            _ => None,
        })
        .unwrap_or(Frequency::DEFAULT)
}

fn set_be_frequency(nbt: &mut Compound, frequency: &Frequency) {
    nbt.insert(
        "frequency".into(),
        Tag::String(frequency.text().as_str().into()),
    );
}

/// The chest's uid, made and kept on first use.
fn be_uid(nbt: &mut Compound) -> u64 {
    if let Some(Tag::Long(uid)) = nbt.get(&"uid".into()) {
        return *uid as u64;
    }
    let uid = fresh_uid();
    nbt.insert("uid".into(), Tag::Long(uid as i64));
    uid
}

// ---------------------------------------------------------------------------
// The store file
// ---------------------------------------------------------------------------

fn store_path(world: &Path) -> PathBuf {
    world.join(STORE_DIR).join(STORE_FILE)
}

fn load_store(world: &Path) -> Store<StoredItem> {
    let path = store_path(world);
    let Ok(text) = fs::read_to_string(&path) else {
        return Store::new();
    };
    // A corrupt line is skipped by the decoder; its warning has no route to the player at this pin
    // (the SDK exports no logger, and a native mod may not add a crate), and the rest still loads.
    let (store, _warnings) = Store::<StoredItem>::decode(&text);
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
    if write_file(&world.join(STORE_DIR), STORE_FILE, &text) {
        let mut guard = core();
        if let Some(state) = guard.as_mut() {
            state.dirty = false;
            state.last_save = state.ticks;
        }
    }
}

/// `<dir>/<name>` written through a temporary and a rename; whether it landed. A failed write keeps
/// the store dirty, so the next tick tries again; the SDK has no logger to tell the player with.
fn write_file(dir: &Path, name: &str, text: &str) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let temporary = dir.join(format!("{name}.tmp"));
    if fs::write(&temporary, text.as_bytes()).is_err() {
        return false;
    }
    fs::rename(&temporary, dir.join(name)).is_ok()
}

/// Load the store of the open world, once per world: `WorldOpenReady` is not a phase at this pin, so
/// the first server tick (or the first chest tick) of a world does it, and every close phase clears
/// it so the next world loads its own.
fn ensure_world(host: &mut dyn ServerHost) {
    let loaded = {
        let guard = core();
        guard.as_ref().is_some_and(|state| state.world.is_some())
    };
    if loaded {
        return;
    }
    let dir = host.world_dir();
    let Some(dir) = dir else {
        return;
    };
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    if state.world.as_deref() == Some(dir.as_path()) {
        return;
    }
    state.store = load_store(&dir);
    state.snapshots.clear();
    state.pending.clear();
    state.dirty = false;
    state.ticks = 0;
    state.last_save = 0;
    state.world = Some(dir);
}

/// Forget the open world, saving it first when asked.
fn close_world(save: bool) {
    let world = {
        let guard = core();
        guard.as_ref().and_then(|state| state.world.clone())
    };
    if save && let Some(world) = world {
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

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

/// Declare the chest: a block entity type with its Part and its first saved compound, and the
/// 54-slot container shown as the vanilla 9x6 menu.
pub fn declare(r: &mut dyn Registrar) {
    let chest = r.block_key(BLOCK);
    let kind = r
        .block_entity_type(BLOCK_ENTITY)
        .blocks(&[BLOCK_ENTITY])
        .build();
    r.block_part(chest, Chest);
    r.block_entity_part(&kind, Chest)
        .expect("the chest's Part attaches to the type this mod just declared");
    r.block_container(&chest, container());
}

/// 54 slots reachable from every side, shown as the vanilla 9x6 menu.
fn container() -> ModContainer {
    let all: Vec<usize> = (0..SLOTS).collect();
    ModContainer {
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
        menu: Some(MENU.to_owned()),
    }
}

/// `/dimstorage channel|info|put|take`.
pub fn declare_commands(commands: &mut Vec<Command>) {
    commands.push(Command::new(
        "dimstorage channel <pos:block_pos> <channel:integer>",
    ));
    commands.push(Command::new("dimstorage info <pos:block_pos>"));
    commands.push(Command::new(
        "dimstorage put <pos:block_pos> <count:integer>",
    ));
    commands.push(Command::new(
        "dimstorage take <pos:block_pos> <slot:integer> <count:integer>",
    ));
}

// ---------------------------------------------------------------------------
// The chest's Part
// ---------------------------------------------------------------------------

/// The chest's Part: the initial compound, the tick and the per-tick merge with the shared store.
pub struct Chest;

#[mod_api::horizon_part]
impl BlockPart for Chest {}

#[mod_api::horizon_part]
impl BlockEntityPart for Chest {
    /// A fresh chest starts on the default public/1 frequency, as upstream's `new Frequency()`.
    fn block_entity(
        &self,
        _level: mod_api::LevelSide,
        _event: &mut mod_api::BlockEntityEvent<'_>,
    ) -> Option<Compound> {
        let mut nbt = Compound::new();
        set_be_frequency(&mut nbt, &Frequency::DEFAULT);
        Some(nbt)
    }

    /// The chest ticks on the server (`EntityBlock.getTicker`).
    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        event.ticks = true;
    }

    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        tick_chest(host, event);
    }
}

/// One server tick of a chest: apply a queued frequency, then merge its slots with the store.
fn tick_chest(host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
    ensure_world(host);
    let pos = event.pos;
    let dimension = host.dimension().to_owned();
    let mut frequency = be_frequency(&mut event.entity.nbt);
    let uid = be_uid(&mut event.entity.nbt);
    let current = stored_slots(&*host, &event.entity.items);
    let mut out = current.clone();
    {
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
                    &current,
                    snapshot.as_deref(),
                    &mut state.store,
                    &frequency,
                    &wanted,
                );
                if changed {
                    state.dirty = true;
                }
                state.snapshots.insert(uid, items.clone());
                out = items;
                set_be_frequency(&mut event.entity.nbt, &wanted);
                frequency = wanted;
            }
        }
        let snapshot = state.snapshots.get(&uid).cloned();
        let synced = sync(&out, snapshot.as_deref(), &mut state.store, &frequency);
        if synced.store_changed {
            state.dirty = true;
        }
        state.snapshots.insert(uid, synced.items.clone());
        out = synced.items;
    }
    if out != current {
        event.entity.items = stacks(&*host, &out);
    }
}

// ---------------------------------------------------------------------------
// The world's lifecycle and the server tick
// ---------------------------------------------------------------------------

/// The world closed: save the store, then forget it.
pub fn world_lifecycle(_host: &mut dyn ServerHost, event: &mut WorldLifecycleEvent<'_>) {
    match event.phase {
        WorldPhase::WorldCloseFinal | WorldPhase::WorldCloseFailed => close_world(true),
        WorldPhase::WorldOpenFailed => close_world(false),
        // `WorldPhase` is non-exhaustive: an open phase this pin adds later needs no handling, as
        // `ensure_world` loads lazily.
        _ => {}
    }
}

/// End of a running tick: load the world on the first tick, then debounce the store write.
pub fn server_tick(host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
    let ServerTickPhase::ServerTickEnd { world_running } = event.phase else {
        return;
    };
    if !world_running {
        return;
    }
    ensure_world(host);
    let (due, world) = {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        state.ticks += 1;
        let due = state.dirty && state.ticks.saturating_sub(state.last_save) >= SAVE_INTERVAL_TICKS;
        (due, state.world.clone())
    };
    if due && let Some(world) = world {
        save_store(&world);
    }
}

/// The server is stopping: flush the store.
pub fn server_stopping(_host: &mut dyn ServerHost, _event: &mut mod_api::ServerStoppingEvent) {
    close_world(true);
}

// ---------------------------------------------------------------------------
// The blocks
// ---------------------------------------------------------------------------

/// The mod block entity at `pos`, when the block there is the chest.
fn our_entity(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
    let key = host.block_key(BLOCK)?;
    if level.block(host, pos) != Some(key) {
        return None;
    }
    host.block_entity(level, pos)
}

/// Commit a chest's last changes to the store, empty its slots (so a break never spills a second
/// copy of what the store keeps), and forget its snapshot.
fn commit_and_empty(host: &mut dyn ServerHost, pos: [i32; 3]) {
    let Some(level) = host.level(host.dimension()) else {
        return;
    };
    let Some(entity) = our_entity(&*host, level, pos) else {
        return;
    };
    let Some(mut nbt) = entity.data(&*host) else {
        return;
    };
    let frequency = read_frequency(&nbt);
    let uid = be_uid(&mut nbt);
    {
        let items = read_slots(&*host, entity);
        let current = stored_slots(&*host, &items);
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        let snapshot = state.snapshots.get(&uid).cloned();
        if commit(&current, snapshot.as_deref(), &mut state.store, &frequency) {
            state.dirty = true;
        }
        state.snapshots.remove(&uid);
    }
    // Empty the block entity before it goes: the store already keeps everything, so a slot left
    // filled would be a second copy of it. A chest's items are its slots, never a saved key.
    let _ = host.transaction(|host, tx| {
        for slot in 0..SLOTS {
            let _ = entity.set_item_slot(host, tx, slot as u32, None);
        }
        Ok::<(), ()>(())
    });
}

/// A chest's slots as the game holds them.
fn read_slots(host: &dyn Host, entity: BlockEntityRef) -> Vec<Option<ItemStack>> {
    (0..SLOTS)
        .map(|slot| entity.item_slot(host, slot as u32).unwrap_or_default())
        .collect()
}

/// Whether a state is the chest block.
fn is_chest(host: &dyn Host, state: BlockState) -> bool {
    host.block_key(BLOCK) == Some(state.block())
}

/// A player broke a chest: commit and empty it, so the store keeps the contents and the block drops
/// through its ordinary loot table without a second copy.
pub fn break_block(host: &mut dyn ServerHost, event: &mut mod_api::BreakBlockEvent<'_>) -> Flow {
    if is_chest(&*host, event.state) {
        commit_and_empty(host, event.pos);
    }
    Flow::Continue
}

/// An explosion about to break chests: the same commit and empty for each one.
pub fn explosion_detonate(
    host: &mut dyn ServerHost,
    event: &mut mod_api::ExplosionDetonateEvent<'_>,
) {
    let Some(level) = host.level(host.dimension()) else {
        return;
    };
    let found: Vec<[i32; 3]> = event
        .blocks
        .iter()
        .filter(|pos| {
            level
                .block_state(&*host, **pos)
                .is_some_and(|state| is_chest(&*host, state))
        })
        .copied()
        .collect();
    for pos in found {
        commit_and_empty(host, pos);
    }
}

// ---------------------------------------------------------------------------
// The commands
// ---------------------------------------------------------------------------

/// `/dimstorage channel <pos> <channel>`, `/dimstorage info <pos>`, `/dimstorage put <pos> <count>`
/// and `/dimstorage take <pos> <slot> <count>`. The channel form queues the change for the chest's
/// next tick, as the block entity is not in the world here. This is a port-owned fallback for the
/// deferred frequency screen.
pub fn run_command(host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
    let invocation = event.command;
    let literals: Vec<&str> = invocation.literals.iter().map(String::as_str).collect();
    match literals.as_slice() {
        ["dimstorage", "channel"] => {
            let (Some(pos), Some(channel)) =
                (block_pos(invocation, "pos"), integer(invocation, "channel"))
            else {
                say(host, "Usage: /dimstorage channel <x> <y> <z> <channel>");
                return Flow::Cancel;
            };
            if !chest_there(host, pos) {
                say(host, "No dimensional chest there.");
                return Flow::Cancel;
            }
            let frequency = Frequency::public(channel);
            let dimension = host.dimension().to_owned();
            {
                let mut guard = core();
                let state = guard.get_or_insert_with(Core::default);
                state
                    .pending
                    .retain(|(dim, at, _)| !(dim == &dimension && *at == pos));
                state.pending.push((dimension, pos, frequency.clone()));
            }
            say(
                host,
                &format!("DimStorage frequency: {}", frequency.label()),
            );
        }
        ["dimstorage", "info"] => {
            let Some(pos) = block_pos(invocation, "pos") else {
                say(host, "Usage: /dimstorage info <x> <y> <z>");
                return Flow::Cancel;
            };
            let line = readout(host, pos);
            say(host, &line);
        }
        ["dimstorage", "put"] => {
            let (Some(pos), Some(count)) =
                (block_pos(invocation, "pos"), integer(invocation, "count"))
            else {
                say(host, "Usage: /dimstorage put <x> <y> <z> <count>");
                return Flow::Cancel;
            };
            let Ok(count) = u32::try_from(count) else {
                say(host, "The count must be a positive integer.");
                return Flow::Cancel;
            };
            let Some(frequency) = chest_frequency(host, pos) else {
                say(host, "No dimensional chest there.");
                return Flow::Cancel;
            };
            let Some(player) = source_player(event.source) else {
                say(host, "Hold the stack to store.");
                return Flow::Cancel;
            };
            let hand = ItemPlace::PlayerHand {
                player: player.uuid(),
                hand: Hand::Main,
            };
            let Some(held) = hand.stack(host) else {
                say(host, "Hold the stack to store.");
                return Flow::Cancel;
            };
            let take = count.min(held.count());
            if take == 0 {
                say(host, "There is nothing to store.");
                return Flow::Cancel;
            }
            let Some(stored) = StoredItem::of(&*host, &held) else {
                say(host, "The held stack could not be taken.");
                return Flow::Cancel;
            };
            // Deposit on a copy first, so the amount the hand really gives decides what the store
            // takes: a hand that gives less is deposited again for what it gave.
            let mut scratch = store_copy();
            let left = scratch.deposit(&frequency, &stored.with_count(take), MAX_STACK);
            let moved = take - left;
            if moved == 0 {
                say(host, "The dimensional chest is full.");
                return Flow::Cancel;
            }
            let resource = ItemResource::of(&held);
            let took = host
                .transaction(|host, tx| Ok::<u32, ()>(hand.extract(host, tx, &resource, moved)))
                .unwrap_or(0);
            if took == 0 {
                say(host, "The held stack could not be taken.");
                return Flow::Cancel;
            }
            {
                let mut guard = core();
                let state = guard.get_or_insert_with(Core::default);
                settle(&mut state.store, &frequency, &scratch, &stored, took, moved);
                state.dirty = true;
            }
            let line = format!("Stored {took} x {}.", held.item().id(&*host));
            say(host, &line);
        }
        ["dimstorage", "take"] => {
            let (Some(pos), Some(slot), Some(count)) = (
                block_pos(invocation, "pos"),
                integer(invocation, "slot"),
                integer(invocation, "count"),
            ) else {
                say(host, "Usage: /dimstorage take <x> <y> <z> <slot> <count>");
                return Flow::Cancel;
            };
            let (Ok(slot), Ok(count)) = (usize::try_from(slot), u32::try_from(count)) else {
                say(host, "The slot and count must be positive integers.");
                return Flow::Cancel;
            };
            if slot >= SLOTS || count == 0 {
                say(host, "No such slot, or a zero count.");
                return Flow::Cancel;
            }
            let Some(frequency) = chest_frequency(host, pos) else {
                say(host, "No dimensional chest there.");
                return Flow::Cancel;
            };
            let stored = {
                let guard = core();
                guard
                    .as_ref()
                    .and_then(|state| state.store.slots(&frequency).get(slot).cloned().flatten())
            };
            let Some(stored) = stored else {
                say(host, "That slot is empty.");
                return Flow::Cancel;
            };
            let Some(stack) = stored.to_stack(&*host) else {
                say(host, "The stored item is not known to this game.");
                return Flow::Cancel;
            };
            let Some(player) = source_player(event.source) else {
                say(host, "There is no room in the inventory.");
                return Flow::Cancel;
            };
            let wanted = count.min(stack.count());
            let room = room_for(&*host, player, &stack);
            let give = wanted.min(room);
            if give == 0 {
                say(host, "There is no room in the inventory.");
                return Flow::Cancel;
            }
            let Some(stack) = ItemStack::new(stack.item(), give) else {
                say(host, "There is no room in the inventory.");
                return Flow::Cancel;
            };
            if player.give_stack(host, stack).is_err() {
                say(host, "There is no room in the inventory.");
                return Flow::Cancel;
            }
            {
                let mut guard = core();
                let state = guard.get_or_insert_with(Core::default);
                state.store.withdraw(&frequency, slot, give);
                state.dirty = true;
            }
            say(host, &format!("Withdrew {give} x {}.", stored.id));
        }
        _ => {
            say(
                host,
                "usage: /dimstorage channel <pos>|info <pos>|put <pos>|take <pos>",
            );
        }
    }
    Flow::Cancel
}

/// A copy of the store, so a command can try an insert before touching the world.
fn store_copy() -> Store<StoredItem> {
    let guard = core();
    guard
        .as_ref()
        .map_or_else(Store::new, |state| state.store.clone())
}

/// Apply `/dimstorage put`'s outcome. When the hand gave exactly what the copy planned, the copy's
/// slots become the store (all of the plan landed). When it gave less, only what it really gave is
/// deposited on top of the current store: the copy's slots assumed the larger amount and must not
/// be written, or the store would hold items nobody gave.
fn settle(
    store: &mut Store<StoredItem>,
    frequency: &Frequency,
    planned: &Store<StoredItem>,
    stored: &StoredItem,
    took: u32,
    planned_take: u32,
) {
    if took == planned_take {
        store.set(frequency, planned.slots(frequency));
    } else if took > 0 {
        store.deposit(frequency, &stored.with_count(took), MAX_STACK);
    }
}

/// Whether the chest block stands at `pos`.
fn chest_there(host: &mut dyn ServerHost, pos: [i32; 3]) -> bool {
    let Some(level) = host.level(host.dimension()) else {
        return false;
    };
    host.block_key(BLOCK).is_some_and(|key| {
        level
            .block_state(&*host, pos)
            .is_some_and(|state| state.block() == key)
    })
}

/// The frequency of the chest at `pos`, when the block there is the chest.
fn chest_frequency(host: &mut dyn ServerHost, pos: [i32; 3]) -> Option<Frequency> {
    let level = host.level(host.dimension())?;
    let entity = our_entity(&*host, level, pos)?;
    Some(read_frequency(&entity.data(&*host)?))
}

/// The `pos` block-position argument.
fn block_pos(invocation: &CommandInvocation, name: &str) -> Option<[i32; 3]> {
    match invocation.argument(name)? {
        CommandArgumentValue::BlockPos(pos) => Some(*pos),
        _ => None,
    }
}

/// An `integer` argument.
fn integer(invocation: &CommandInvocation, name: &str) -> Option<i32> {
    match invocation.argument(name)? {
        CommandArgumentValue::Integer(value) => Some(*value),
        _ => None,
    }
}

/// The player a command came from.
fn source_player(source: &CommandSource) -> Option<PlayerRef> {
    match source {
        CommandSource::Player { player, .. } => Some(*player),
        _ => None,
    }
}

/// One feedback line to the command's source, vanilla's silence and track-output rules included.
fn say(host: &mut dyn ServerHost, line: &str) {
    let _ = host.commands().feedback(Text::literal(line));
}

/// How many of `stack` fit into the player's main inventory, as `/give` adds them: matching
/// stacks first, then empty slots. Zero means a give would have dropped the items.
fn room_for(host: &dyn Host, player: PlayerRef, stack: &ItemStack) -> u32 {
    let Some(slots) = player.inventory_slots(host) else {
        return 0;
    };
    let max = host.max_stack_size(stack.item()).unwrap_or(MAX_STACK);
    let mut room = 0u32;
    let mut used = 0u32;
    let kind = kind_of(host, stack);
    for (slot, other) in &slots {
        if *slot >= MAIN_SLOTS {
            continue;
        }
        used += 1;
        if kind_of(host, other) == kind {
            room += max.saturating_sub(other.count());
        }
    }
    room + MAIN_SLOTS.saturating_sub(used) * max
}

/// A stack's identity without its count: the id and its saved compound.
fn kind_of(host: &dyn Host, stack: &ItemStack) -> (String, Option<Compound>) {
    let id = stack.item().id(host);
    let nbt = stack.to_nbt(host).map(|mut nbt| {
        nbt.remove(&"count".into());
        nbt
    });
    (id, nbt)
}

/// The chest's state, as `/dimstorage info` prints it.
fn readout(host: &mut dyn ServerHost, pos: [i32; 3]) -> String {
    let Some(frequency) = chest_frequency(host, pos) else {
        return "No dimensional chest there.".to_owned();
    };
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
    format!(
        "DimStorage: {} ({used}/{SLOTS} slots, {total} items)",
        frequency.label()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A saved item compound in the shape `ItemStack.CODEC` writes.
    fn item(id: &str, count: i32, damage: Option<i32>) -> Compound {
        let mut nbt = Compound::new();
        nbt.insert("count".into(), Tag::Int(count));
        nbt.insert("id".into(), Tag::String(id.into()));
        if let Some(damage) = damage {
            let mut components = Compound::new();
            components.insert("minecraft:damage".into(), Tag::Int(damage));
            nbt.insert("components".into(), Tag::Compound(components));
        }
        nbt
    }

    fn stored(id: &str, count: u32, damage: Option<i32>) -> StoredItem {
        StoredItem {
            id: id.to_owned(),
            count,
            nbt: item(id, count as i32, damage),
        }
    }

    #[test]
    fn a_stored_item_round_trips_through_the_store_text() {
        let original = stored("minecraft:diamond", 5, Some(3));
        let text = original.encode();
        assert!(text.starts_with("hex:"), "{text}");
        assert_eq!(StoredItem::decode(&text), Some(original));

        assert_eq!(StoredItem::decode("not-a-hex-carrying-line"), None);
        assert_eq!(StoredItem::decode("hex:zz"), None);
    }

    #[test]
    fn same_kind_ignores_the_count_but_not_the_damage() {
        let five = stored("minecraft:diamond_sword", 1, Some(5));
        let other_five = stored("minecraft:diamond_sword", 7, Some(5));
        let worn = stored("minecraft:diamond_sword", 1, Some(9));
        assert!(five.same_kind(&other_five));
        assert!(!five.same_kind(&worn));
        assert_eq!(other_five.with_count(2).count(), 2);
        assert_eq!(other_five.with_count(0).count(), 1);
    }

    #[test]
    fn the_store_file_keeps_readable_frequencies_and_hex_items() {
        let mut store = Store::new();
        let mut slots = vec![None; SLOTS];
        slots[0] = Some(stored("minecraft:diamond", 3, None));
        store.set(&Frequency::public(2), slots.clone());
        let text = store.encode();
        assert!(text.contains("f public,2"), "{text}");
        assert!(text.contains("i 0 hex:"), "{text}");
        let (decoded, warnings) = Store::<StoredItem>::decode(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(decoded.slots(&Frequency::public(2)), slots);
    }

    #[test]
    fn the_chest_compound_defaults_to_public_channel_one() {
        let mut nbt = Compound::new();
        assert_eq!(be_frequency(&mut nbt), Frequency::DEFAULT);
        // The default was written back, so the chest keeps it.
        assert_eq!(
            nbt.get(&"frequency".into()),
            Some(&Tag::String("public,1".into()))
        );

        set_be_frequency(&mut nbt, &Frequency::public(4));
        assert_eq!(read_frequency(&nbt), Frequency::public(4));
        let uid = be_uid(&mut nbt);
        assert_eq!(be_uid(&mut nbt), uid, "the uid is stable");
    }

    #[test]
    fn the_container_is_fifty_four_slots_on_every_face() {
        let container = container();
        assert_eq!(container.slots, SLOTS);
        assert_eq!(container.faces.len(), 6);
        assert!(container.faces.iter().all(|face| face.len() == SLOTS));
        assert_eq!(container.menu.as_deref(), Some(MENU));
    }

    #[test]
    fn a_partial_hand_deposit_settles_at_what_it_gave() {
        let frequency = Frequency::public(1);
        let diamond = stored("minecraft:diamond", 64, None);
        let mut store = Store::new();
        // `/dimstorage put … 40`: the copy plans 40.
        let mut planned = Store::new();
        planned.deposit(&frequency, &diamond.with_count(40), MAX_STACK);
        assert_eq!(planned.count(&frequency), 1);

        // The hand gave all 40: the copy becomes the store.
        settle(&mut store, &frequency, &planned, &diamond, 40, 40);
        assert_eq!(store.slots(&frequency)[0], Some(diamond.with_count(40)));

        // A hand that gave only 10 must add 10, never the copy's 40.
        let mut other = Store::new();
        settle(&mut other, &frequency, &planned, &diamond, 10, 40);
        assert_eq!(other.slots(&frequency)[0], Some(diamond.with_count(10)));
    }
}
