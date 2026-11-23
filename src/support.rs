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

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mod_api::{
    BlockEntityPart, BlockEntityRef, BlockEntityTickEvent, BlockEntityTicksEvent, BlockPart,
    BlockState, Command, CommandArgumentValue, CommandInvocation, CommandSource, Compound, Flow,
    FluidResource, FluidStack, GameMode, Hand, Host, HostTransaction, ITEM_FLUIDS, Identifier,
    InteractionResult, ItemKey, ItemPlace, ItemResource, ItemStack, LevelRef, ModContainer,
    ModFluidTanks, PlayerRef, Registrar, RunModCommandEvent, ServerHost, ServerTickEvent,
    ServerTickPhase, Tag, Text, WorldLifecycleEvent, WorldPhase,
};

use crate::runtime::{
    Frequency, MAX_STACK, SLOTS, Slot, Stack, Store, TANK_CAPACITY, Tank, TankFluid, TankStore,
    commit, may_read, switch, sync, take_allowed, tank_switch, tank_sync, tank_transfer,
};

/// The dimensional chest of slice 1 (its item has the same id).
const BLOCK: &str = "dimstorage:dimensional_chest";
/// The block entity type the block declares; it is in this mod's namespace.
const BLOCK_ENTITY: &str = "dimensional_chest";
/// The dimensional tank of slice 2 (its item has the same id).
const TANK_BLOCK: &str = "dimstorage:dimensional_tank";
/// The tank's block entity type, also in this mod's namespace.
const TANK_ENTITY: &str = "dimensional_tank";
/// The store lives beside the world, so backups and the world travel together.
const STORE_DIR: &str = "dimstorage";
const STORE_FILE: &str = "inventories.dat";
/// The tanks' contents live in their own file beside the chests': upstream keeps
/// both in one `SavedData`, and this port keeps one file per store.
const TANK_FILE: &str = "tanks.dat";
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
    /// Every frequency's tank contents, the world's fluid truth.
    tanks: TankStore,
    /// Each chest's last synced slots, by its own uid (it moves with the block entity, so a piston
    /// move does not confuse the merge).
    snapshots: HashMap<u64, Vec<Option<StoredItem>>>,
    /// Frequencies a chest must take from `/dimstorage channel`; the block entity is not in the
    /// world when the command runs.
    pending: Vec<(String, [i32; 3], Frequency)>,
    /// The loaded chests of each channel, by uid, with the server tick each last ticked in. Each
    /// chest's copy of the channel is a *share* of it ([`runtime::share`]), so two loaded chests
    /// can never dispense the same items.
    mirrors: BTreeMap<Frequency, BTreeMap<u64, Mirror>>,
    dirty: bool,
    ticks: u64,
    last_save: u64,
}

impl Default for Core {
    fn default() -> Self {
        Core {
            world: None,
            store: Store::new(),
            tanks: TankStore::new(),
            snapshots: HashMap::new(),
            pending: Vec::new(),
            mirrors: BTreeMap::new(),
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

fn tank_path(world: &Path) -> PathBuf {
    world.join(STORE_DIR).join(TANK_FILE)
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

fn load_tanks(world: &Path) -> TankStore {
    let path = tank_path(world);
    let Ok(text) = fs::read_to_string(&path) else {
        return TankStore::new();
    };
    let (tanks, _warnings) = TankStore::decode(&text);
    tanks
}

/// Write the stores atomically: a `.tmp` beside each, then a rename.
fn save_store(world: &Path) {
    let (inventories, tanks) = {
        let guard = core();
        let Some(state) = guard.as_ref() else {
            return;
        };
        if !state.dirty {
            return;
        }
        (state.store.encode(), state.tanks.encode())
    };
    let dir = world.join(STORE_DIR);
    let saved = write_file(&dir, STORE_FILE, &inventories) & write_file(&dir, TANK_FILE, &tanks);
    if saved {
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
    state.tanks = load_tanks(&dir);
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
        state.tanks = TankStore::new();
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
    r.block_entity_part(&kind, ChestEntity)
        .expect("the chest's Part attaches to the type this mod just declared");
    r.block_container(&chest, container());
}

/// Declare the tank: a block entity type with its Part, and one 16000 mB tank every
/// side may fill and drain, so a bucket, a pipe or a hopper-style handler reaches it.
pub fn declare_tank(r: &mut dyn Registrar) {
    let tank = r.block_key(TANK_BLOCK);
    let kind = r
        .block_entity_type(TANK_ENTITY)
        .blocks(&[TANK_ENTITY])
        .build();
    r.block_entity_part(&kind, TankBlock)
        .expect("the tank's Part attaches to the type this mod just declared");
    r.block_fluids(&tank, tanks());
}

/// The tank's fluid capability: one tank of the upstream capacity, every face on
/// both the fill and the drain side.
fn tanks() -> ModFluidTanks {
    // The type is non-exhaustive, so it is built from its default.
    let mut tanks = ModFluidTanks::default();
    tanks.tanks = vec![TANK_CAPACITY];
    tanks.faces = [vec![0], vec![0], vec![0], vec![0], vec![0], vec![0]];
    tanks.inputs = vec![0];
    tanks.outputs = vec![0];
    tanks
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
impl BlockPart for Chest {
    /// A chest's inventory changed outside its tick: a player's menu or a hopper. Commit the change
    /// to the authoritative store **at once** and push the result into every other loaded chest of
    /// the channel, so no sibling can hand out contents the store no longer holds between ticks.
    fn contents_changed(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::ContentsChangedEvent,
    ) {
        on_contents_changed(host, event.level, [event.pos.x, event.pos.y, event.pos.z]);
    }
}

thread_local! {
    /// Set while one access is being pushed into the other mirrors, so those mirrors' own notices
    /// do not push it back: without it two chests of one channel would notify each other forever
    /// on the server thread.
    static SYNCING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Commit one chest's access to the store and align every other loaded mirror of the channel.
fn on_contents_changed(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) {
    if SYNCING.with(std::cell::Cell::get) {
        return;
    }
    SYNCING.with(|flag| flag.set(true));
    push_access(host, level, pos);
    SYNCING.with(|flag| flag.set(false));
}

/// The body of [`on_contents_changed`], under the re-entrancy guard.
fn push_access(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) {
    let Some(entity) = our_entity(&*host, level, pos) else {
        return;
    };
    let Some(mut data) = entity.data(&*host) else {
        return;
    };
    let frequency = read_frequency(&data);
    let uid = be_uid(&mut data);
    let current = read_slots(&*host, entity);
    let seats: Vec<(u64, String, [i32; 3])> = {
        let mut guard = core();
        let Some(state) = guard.as_mut() else {
            return;
        };
        let snapshot = state.snapshots.get(&uid).cloned();
        let committed = commit(
            &stored_slots(&*host, &current),
            snapshot.as_deref(),
            &mut state.store,
            &frequency,
        );
        if committed {
            state.dirty = true;
        }
        // This chest's new snapshot is what the store holds for it, after the delta.
        state.snapshots.insert(uid, state.store.slots(&frequency));
        let limit = SYNC_MIRRORS_PER_ACCESS;
        state
            .mirrors
            .get(&frequency)
            .map(|mirrors| {
                mirrors
                    .iter()
                    .filter(|(other, _)| **other != uid)
                    .take(limit)
                    .map(|(id, mirror)| (*id, mirror.dimension.clone(), mirror.pos))
                    .collect()
            })
            .unwrap_or_default()
    };
    if seats.is_empty() {
        return;
    }
    let slots = {
        let guard = core();
        guard
            .as_ref()
            .map(|state| state.store.slots(&frequency))
            .unwrap_or_default()
    };
    let stacks = stacks(&*host, &slots);
    for (other, dimension, at) in seats {
        let Some(level) = host.level(&dimension) else {
            continue;
        };
        let Some(entity) = our_entity(&*host, level, at) else {
            continue;
        };
        let _ = host.transaction(|host, tx| -> Result<(), ()> {
            for (slot, stack) in stacks.iter().enumerate() {
                let _ = entity.set_item_slot(
                    host,
                    tx,
                    u32::try_from(slot).unwrap_or(u32::MAX),
                    stack.clone(),
                );
            }
            Ok(())
        });
        let mut guard = core();
        if let Some(state) = guard.as_mut() {
            state.snapshots.insert(other, slots.clone());
        }
    }
}

/// The chest's block-entity Part: a **separate type** from the block Part, because the engine
/// refuses one part type attached to a block id twice and a block-entity part registers against
/// the block it belongs to.
pub struct ChestEntity;

#[mod_api::horizon_part]
impl BlockPart for ChestEntity {}

#[mod_api::horizon_part]
impl BlockEntityPart for ChestEntity {
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

/// How many sibling chests one access aligns. A channel with more loaded chests than this keeps
/// the rest on their tick merge; the cap keeps one click's work bounded.
const SYNC_MIRRORS_PER_ACCESS: usize = 16;

/// One loaded chest of a channel: where it is, and the server tick it last ticked in.
#[derive(Clone, Debug)]
struct Mirror {
    tick: u64,
    dimension: String,
    pos: [i32; 3],
}

/// Register this chest as a loaded mirror of `frequency` for the current tick.
///
/// Every loaded chest ticks every tick, so a uid that has not ticked for two ticks is unloaded,
/// broken or retuned. The registry is what lets an access on one chest push the store's state into
/// its siblings at once: Horizon gives each block entity its own inventory, so without that push a
/// sibling keeps a stale copy between ticks and can hand out items the store no longer holds.
fn register_mirror(
    state: &mut Core,
    frequency: &Frequency,
    uid: u64,
    dimension: &str,
    pos: [i32; 3],
) {
    let tick = state.ticks;
    let mirrors = state.mirrors.entry(frequency.clone()).or_default();
    mirrors.retain(|_, seen| tick.saturating_sub(seen.tick) <= 1);
    mirrors.insert(
        uid,
        Mirror {
            tick,
            dimension: dimension.to_owned(),
            pos,
        },
    );
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
        register_mirror(state, &frequency, uid, &dimension, pos);
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
// The tank's Part (slice 2)
// ---------------------------------------------------------------------------

/// The tank's Part: its first saved compound, the tick that mirrors the shared
/// channel into the entity's `fluids` list, and the bucket interaction on use.
pub struct TankBlock;

/// One tank entry from a `Fluid`/`Amount` compound: the shape the game's fluid
/// capability reads and writes (millibuckets, empty spelled `minecraft:empty`
/// with amount 0).
fn read_entry(map: &Compound) -> Tank {
    let id = match map.get(&"Fluid".into()) {
        Some(Tag::String(id)) if id.to_string_lossy() != "minecraft:empty" => id.to_string_lossy(),
        _ => return None,
    };
    let amount = match map.get(&"Amount".into()) {
        Some(Tag::Int(amount)) => *amount,
        _ => 0,
    };
    (amount > 0).then(|| TankFluid::new(id, amount))
}

/// A tank entry, as the game's `fluids` list holds one.
fn entry(tank: &Tank) -> Compound {
    let mut map = Compound::new();
    map.insert(
        "Fluid".into(),
        Tag::String(
            tank.as_ref()
                .map_or("minecraft:empty", |fluid| fluid.id.as_str())
                .into(),
        ),
    );
    map.insert(
        "Amount".into(),
        Tag::Int(tank.as_ref().map_or(0, |fluid| fluid.amount)),
    );
    map
}

/// The tank's contents from its saved compound: the first `fluids` entry.
fn read_tank(nbt: &Compound) -> Tank {
    let Some(Tag::List(list)) = nbt.get(&"fluids".into()) else {
        return None;
    };
    let Some(Tag::Compound(map)) = list.first() else {
        return None;
    };
    read_entry(map)
}

/// Write a tank's contents into the saved compound, one entry for the one tank.
fn set_tank(nbt: &mut Compound, tank: &Tank) {
    nbt.insert("fluids".into(), Tag::List(vec![Tag::Compound(entry(tank))]));
}

/// What the channel held when this tank last synced, saved beside the mirror: the
/// delta since then is a transfer the channel has not committed yet (a pipe, a
/// crash between the handler and the tick). `None` for a tank with no marker (a
/// fresh block, or one saved by 0.2.3).
fn read_synced(nbt: &Compound) -> Option<Tank> {
    match nbt.get(&"synced".into()) {
        Some(Tag::Compound(map)) => Some(read_entry(map)),
        _ => None,
    }
}

fn set_synced(nbt: &mut Compound, tank: &Tank) {
    nbt.insert("synced".into(), Tag::Compound(entry(tank)));
}

/// Free millibuckets for `id` in the channel.
fn channel_room(frequency: &Frequency, id: &str) -> i32 {
    let guard = core();
    guard
        .as_ref()
        .map_or(TANK_CAPACITY, |state| state.tanks.room(frequency, id))
}

#[mod_api::horizon_part]
impl BlockPart for TankBlock {
    /// A bucket (or any fluid container) on the tank: empty it in, or fill it out,
    /// through the game's own fluid handler over this block's `fluids` capability,
    /// so the bucket's item, sound and game event are vanilla's. The transfer is
    /// committed to the channel's one balance before this reports success.
    fn use_item_on(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::UseItemOnEvent<'_>,
    ) -> Option<InteractionResult> {
        Some(tank_use(host, event))
    }
}

#[mod_api::horizon_part]
impl BlockEntityPart for TankBlock {
    /// A fresh tank starts on the default public/1 frequency, as upstream's
    /// `new Frequency()`; its `fluids` list is written by the first sync.
    fn block_entity(
        &self,
        _level: mod_api::LevelSide,
        _event: &mut mod_api::BlockEntityEvent<'_>,
    ) -> Option<Compound> {
        let mut nbt = Compound::new();
        set_be_frequency(&mut nbt, &Frequency::DEFAULT);
        Some(nbt)
    }

    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        event.ticks = true;
    }

    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        tick_tank(host, event);
    }
}

/// What a click's container may do with the channel before the game's handler runs.
enum TankMove {
    /// Let the handler attempt the whole transfer.
    Whole,
    /// The hand holds no fluid container; the handler decides.
    NotFluid,
    /// The transfer cannot complete; say why and consume the click.
    Refused(&'static str),
}

/// The body of the tank's `use_item_on`.
fn tank_use(host: &mut dyn ServerHost, event: &mod_api::UseItemOnEvent<'_>) -> InteractionResult {
    let click = event.click;
    let Some(level) = host.level(click.dimension.key().id()) else {
        return InteractionResult::Pass;
    };
    // Bring this tank's copy up to the channel's truth before anything reads it:
    // a sibling may have filled or drained it since this tank's last tick.
    let Some((frequency, before, channel)) = sync_tank_at(host, level, click.pos) else {
        return InteractionResult::Pass;
    };
    let Some(player) = host.player(click.player) else {
        return InteractionResult::Pass;
    };
    match tank_move(host, click, &frequency, &channel) {
        TankMove::Refused(line) => {
            // A capacity refusal is decided before the handler moves anything, and
            // the click is consumed so the held bucket does not try the world.
            say(host, line);
            return InteractionResult::Success;
        }
        TankMove::Whole | TankMove::NotFluid => {}
    }
    if mod_api::fluid_util::interact_with_fluid_handler(
        host, player, click.hand, level, click.pos, None,
    ) {
        // The handler moved fluid into or out of this tank's copy. Commit that
        // delta to the channel's one balance now, before success is reported.
        commit_tank_at(host, level, click.pos, &before);
        return InteractionResult::Success;
    }
    if click.stack.is_some() {
        // An item that is not a fluid container: let the game have the click.
        return InteractionResult::Pass;
    }
    // An empty hand: answer with the channel's contents, the port's stand-in for
    // the deferred tank screen.
    let line = tank_readout(host, click.pos);
    say(host, &line);
    InteractionResult::Success
}

/// Decide whether the held container's whole transfer fits the channel, using the
/// item's own fluid handler: a fill needs as much free room as the container
/// holds, and a drain needs the channel to fill a container of its fluid.
fn tank_move(
    host: &mut dyn ServerHost,
    click: &mod_api::BlockClick<'_>,
    frequency: &Frequency,
    channel: &Tank,
) -> TankMove {
    let place = ItemPlace::PlayerHand {
        player: click.player,
        hand: click.hand,
    };
    let Some(handler) = place.capability(host, &ITEM_FLUIDS) else {
        return TankMove::NotFluid;
    };
    let held = (0..handler.size(host))
        .find_map(|index| handler.resource(host, index).map(|fluid| (index, fluid)));
    if let Some((index, fluid)) = held {
        let Some(key) = fluid.fluid().key() else {
            return TankMove::NotFluid;
        };
        let id = key.id(&*host);
        let amount = i32::try_from(handler.amount(host, index)).unwrap_or(i32::MAX);
        if amount > channel_room(frequency, &id) {
            return TankMove::Refused("The channel has no room for that much fluid.");
        }
        return TankMove::Whole;
    }
    // An empty container to fill: the channel must give a whole load of its fluid.
    let Some(channel) = channel else {
        return TankMove::Whole;
    };
    let Some(key) = host.fluid_key(&channel.id) else {
        return TankMove::Whole;
    };
    let resource = FluidResource::of(&FluidStack::new(key, 1));
    let capacity = (0..handler.size(host))
        .map(|index| handler.capacity(host, index, &resource))
        .max()
        .unwrap_or(0);
    if capacity > 0 && u32::try_from(channel.amount).unwrap_or(0) < capacity {
        return TankMove::Refused("The channel does not hold enough fluid for that.");
    }
    TankMove::Whole
}

/// Mirror the tank at `pos` with the channel store before an access, as the
/// chest's `contents_changed` commits its own at the point of access. Answers the
/// channel's value (what the tank now shows).
fn sync_tank_at(
    host: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
) -> Option<(Frequency, Tank, Tank)> {
    ensure_world(host);
    let entity = our_tank_entity(&*host, level, pos)?;
    let mut nbt = entity.data(&*host)?;
    let frequency = be_frequency(&mut nbt);
    let current = read_tank(&nbt);
    let snapshot = read_synced(&nbt);
    let (out, channel) = {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        let synced = tank_sync(&current, snapshot.as_ref(), &mut state.tanks, &frequency);
        if synced.store_changed {
            state.dirty = true;
        }
        (synced.tank, state.tanks.get(&frequency))
    };
    let mut data = Compound::new();
    set_tank(&mut data, &out);
    // The marker is the channel's value, never a local excess: a capability fill
    // the channel could not take stays visible in `fluids` and is drained first,
    // so it is not destroyed by the next sync.
    set_synced(&mut data, &channel);
    let _ = entity.merge_data(host, data);
    Some((frequency, out, channel))
}

/// Commit a transfer the game's handler just made in this tank's copy to the
/// channel, then write the channel's balance back into that copy.
fn commit_tank_at(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3], before: &Tank) {
    let Some(entity) = our_tank_entity(&*host, level, pos) else {
        return;
    };
    let Some(mut nbt) = entity.data(&*host) else {
        return;
    };
    let frequency = be_frequency(&mut nbt);
    let after = read_tank(&nbt);
    if &after == before {
        return;
    }
    let tank = {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        let transfer = tank_transfer(before, &after, &mut state.tanks, &frequency);
        if transfer.committed {
            state.dirty = true;
        }
        transfer.tank
    };
    let mut data = Compound::new();
    set_tank(&mut data, &tank);
    set_synced(&mut data, &tank);
    let _ = entity.merge_data(host, data);
}

/// One server tick of a tank: apply a queued frequency, then merge its contents
/// with the channel store. A capability transfer since the last sync (a pipe, or
/// a crash between the handler and this tick) is the delta against the saved
/// marker, so a drained channel is never re-seeded by a stale copy.
fn tick_tank(host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
    ensure_world(host);
    let pos = event.pos;
    let dimension = host.dimension().to_owned();
    let mut frequency = be_frequency(&mut event.entity.nbt);
    let current = read_tank(&event.entity.nbt);
    let mut snapshot = read_synced(&event.entity.nbt);
    let mut out = current.clone();
    let channel;
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
                let (tank, changed) = tank_switch(
                    &current,
                    snapshot.as_ref(),
                    &mut state.tanks,
                    &frequency,
                    &wanted,
                );
                if changed {
                    state.dirty = true;
                }
                out = tank.clone();
                snapshot = Some(tank);
                set_be_frequency(&mut event.entity.nbt, &wanted);
                frequency = wanted;
            }
        }
        let synced = tank_sync(&out, snapshot.as_ref(), &mut state.tanks, &frequency);
        if synced.store_changed {
            state.dirty = true;
        }
        out = synced.tank;
        channel = state.tanks.get(&frequency);
    }
    if out != current || snapshot.as_ref() != Some(&channel) {
        set_tank(&mut event.entity.nbt, &out);
        // As at an access: the marker is the channel's value, so a capability
        // fill the channel could not take stays in `fluids` instead of being
        // clamped away by the next sync.
        set_synced(&mut event.entity.nbt, &channel);
    }
}

/// The tank's state, as `/dimstorage info` prints it for a tank block.
fn tank_readout(host: &mut dyn ServerHost, pos: [i32; 3]) -> String {
    let Some(frequency) = tank_frequency(host, pos) else {
        return "No dimensional tank there.".to_owned();
    };
    let tank = {
        let guard = core();
        guard.as_ref().and_then(|state| state.tanks.get(&frequency))
    };
    match tank {
        Some(fluid) => format!(
            "DimStorage tank: {} ({} / {TANK_CAPACITY} mB of {})",
            frequency.label(),
            fluid.amount,
            fluid.id
        ),
        None => format!(
            "DimStorage tank: {} (0 / {TANK_CAPACITY} mB, empty)",
            frequency.label()
        ),
    }
}

/// Whether the tank block stands at `pos`.
fn tank_there(host: &mut dyn ServerHost, pos: [i32; 3]) -> bool {
    let Some(level) = host.level(host.dimension()) else {
        return false;
    };
    host.block_key(TANK_BLOCK).is_some_and(|key| {
        level
            .block_state(&*host, pos)
            .is_some_and(|state| state.block() == key)
    })
}

/// The frequency of the tank at `pos`, when the block there is the tank.
fn tank_frequency(host: &mut dyn ServerHost, pos: [i32; 3]) -> Option<Frequency> {
    let level = host.level(host.dimension())?;
    let entity = our_tank_entity(&*host, level, pos)?;
    Some(read_frequency(&entity.data(&*host)?))
}

/// The mod's tank block entity at `pos`, when the block there is the tank.
fn our_tank_entity(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
    let key = host.block_key(TANK_BLOCK)?;
    if level.block(host, pos) != Some(key) {
        return None;
    }
    host.block_entity(level, pos)
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

/// Whether a state is the tank block.
fn is_tank(host: &dyn Host, state: BlockState) -> bool {
    host.block_key(TANK_BLOCK) == Some(state.block())
}

/// A tank about to be removed: commit the transfer it made since its last sync
/// to the channel before the block (and its `fluids` compound) goes. A tank's
/// fluid lives in the channel, never in the block, so nothing is emptied here.
fn commit_tank(host: &mut dyn ServerHost, pos: [i32; 3]) {
    let Some(level) = host.level(host.dimension()) else {
        return;
    };
    let Some(entity) = our_tank_entity(&*host, level, pos) else {
        return;
    };
    let Some(nbt) = entity.data(&*host) else {
        return;
    };
    let frequency = read_frequency(&nbt);
    let mirror = read_tank(&nbt);
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    let changed = match read_synced(&nbt) {
        Some(snapshot) => tank_transfer(&snapshot, &mirror, &mut state.tanks, &frequency).committed,
        // A tank saved by 0.2.3 (no marker): its compound is the only copy, so an
        // unknown channel is seeded from it; a known one is never overwritten.
        None => !state.tanks.knows(&frequency) && state.tanks.set(&frequency, mirror),
    };
    if changed {
        state.dirty = true;
    }
}

/// A player broke a chest or a tank: commit it, so the store keeps the contents
/// and the block drops through its ordinary loot table without a second copy. A
/// chest's slots are also emptied; a tank's fluid stays in the channel.
pub fn break_block(host: &mut dyn ServerHost, event: &mut mod_api::BreakBlockEvent<'_>) -> Flow {
    if is_chest(&*host, event.state) {
        commit_and_empty(host, event.pos);
    } else if is_tank(&*host, event.state) {
        commit_tank(host, event.pos);
    }
    Flow::Continue
}

/// An explosion about to break chests or tanks: the same commit for each one.
pub fn explosion_detonate(
    host: &mut dyn ServerHost,
    event: &mut mod_api::ExplosionDetonateEvent<'_>,
) {
    let Some(level) = host.level(host.dimension()) else {
        return;
    };
    let found: Vec<([i32; 3], bool, bool)> = event
        .blocks
        .iter()
        .filter_map(|pos| {
            level
                .block_state(&*host, *pos)
                .map(|state| (*pos, is_chest(&*host, state), is_tank(&*host, state)))
        })
        .collect();
    for (pos, chest, tank) in found {
        if chest {
            commit_and_empty(host, pos);
        } else if tank {
            commit_tank(host, pos);
        }
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
            if !reach(host, event.source, pos) {
                say(host, "That chest is out of reach.");
                return Flow::Cancel;
            }
            if !chest_there(host, pos) && !tank_there(host, pos) {
                say(host, "No dimensional chest or tank there.");
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
            if !reach(host, event.source, pos) {
                say(host, "That chest is out of reach.");
                return Flow::Cancel;
            }
            let line = if tank_there(host, pos) {
                tank_readout(host, pos)
            } else {
                readout(host, pos)
            };
            say(host, &line);
        }
        ["dimstorage", "put"] => {
            let (Some(pos), Some(count)) =
                (block_pos(invocation, "pos"), integer(invocation, "count"))
            else {
                say(host, "Usage: /dimstorage put <x> <y> <z> <count>");
                return Flow::Cancel;
            };
            if !reach(host, event.source, pos) {
                say(host, "That chest is out of reach.");
                return Flow::Cancel;
            }
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
            if !reach(host, event.source, pos) {
                say(host, "That chest is out of reach.");
                return Flow::Cancel;
            }
            let (Ok(slot), Ok(count)) = (usize::try_from(slot), u32::try_from(count)) else {
                say(host, "The slot and count must be positive integers.");
                return Flow::Cancel;
            };
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
            if !take_allowed(
                player.game_mode(&*host) != Some(GameMode::Spectator),
                slot,
                count,
            ) {
                say(
                    host,
                    "A spectator cannot withdraw from a dimensional chest.",
                );
                return Flow::Cancel;
            }
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
            // The withdrawal is reserved in the authoritative store *before* the items are handed
            // out, and given back if the hand cannot take them: the old order gave the stack first
            // and updated the store afterwards, so a chest copy could dispense the same items in
            // between, and a failed give left the store unchanged while the player held the items.
            let reserved = {
                let mut guard = core();
                let state = guard.get_or_insert_with(Core::default);
                let taken = state.store.withdraw(&frequency, slot, give);
                if taken.is_some() {
                    state.dirty = true;
                }
                taken
            };
            let Some(taken) = reserved else {
                say(host, "That slot is empty.");
                return Flow::Cancel;
            };
            if player.give_stack(host, stack).is_err() {
                let mut guard = core();
                let state = guard.get_or_insert_with(Core::default);
                state.store.deposit(&frequency, &taken, MAX_STACK);
                state.dirty = true;
                say(host, "There is no room in the inventory.");
                return Flow::Cancel;
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

/// The source's right to use a chest's channel: whether its permission reaches the operator level
/// (2) and the player's position when it is a player. A non-operator player may use only a chest
/// within [`crate::runtime::READ_REACH`]; any other non-operator source may use none.
fn reader(host: &dyn ServerHost, source: &CommandSource) -> (bool, Option<[f64; 3]>) {
    let operator = source.permission().level() >= 2;
    let player = match source {
        CommandSource::Player { player, .. } => player.entity().position(host),
        _ => None,
    };
    (operator, player)
}

/// Whether this source may use the chest at `pos` (see [`reader`]).
fn reach(host: &mut dyn ServerHost, source: &CommandSource, pos: [i32; 3]) -> bool {
    let (operator, player) = reader(&*host, source);
    may_read(operator, player, pos)
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
    fn the_tank_compound_round_trips_its_fluid_and_amount() {
        // The shape the game's fluid capability reads and writes: `fluids` is a
        // list of `{Fluid, Amount}` in millibuckets. A fresh compound has no list
        // and reads as empty; an emptied tank is written as minecraft:empty / 0.
        let mut nbt = Compound::new();
        assert_eq!(read_tank(&nbt), None, "no fluids list is an empty tank");

        set_tank(&mut nbt, &Some(TankFluid::new("minecraft:water", 3000)));
        assert_eq!(
            read_tank(&nbt),
            Some(TankFluid::new("minecraft:water", 3000))
        );

        set_tank(&mut nbt, &None);
        assert_eq!(read_tank(&nbt), None);
        let Some(Tag::List(list)) = nbt.get(&"fluids".into()) else {
            panic!("an emptied tank keeps its one entry so the tank number stays put");
        };
        let Some(Tag::Compound(entry)) = list.first() else {
            panic!("one entry");
        };
        assert_eq!(
            entry.get(&"Fluid".into()),
            Some(&Tag::String("minecraft:empty".into()))
        );
        assert_eq!(entry.get(&"Amount".into()), Some(&Tag::Int(0)));
    }

    #[test]
    fn the_tanks_synced_marker_round_trips_and_distinguishes_no_marker() {
        // The marker is what the channel held when this tank last synced: a tank
        // with none (0.2.3, or a fresh block) may seed an unknown channel, and one
        // with a marker may only commit its delta. An empty marker is meaningful.
        let mut nbt = Compound::new();
        assert_eq!(read_synced(&nbt), None, "no marker at all");

        set_synced(&mut nbt, &Some(TankFluid::new("minecraft:water", 4000)));
        assert_eq!(
            read_synced(&nbt),
            Some(Some(TankFluid::new("minecraft:water", 4000)))
        );

        set_synced(&mut nbt, &None);
        assert_eq!(
            read_synced(&nbt),
            Some(None),
            "synced empty is not no marker"
        );
    }

    #[test]
    fn the_tank_capability_is_one_tank_of_sixteen_buckets_on_every_face() {
        let tanks = tanks();
        assert_eq!(tanks.tanks, vec![TANK_CAPACITY]);
        assert_eq!(TANK_CAPACITY, 16_000);
        assert!(tanks.faces.iter().all(|face| face == &vec![0]));
        assert_eq!(tanks.inputs, vec![0]);
        assert_eq!(tanks.outputs, vec![0]);
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
