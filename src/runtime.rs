// DimStorage for Horizon — the frequency store and its merge (pure).
//
// Ported from DimStorage 9.1.1 (Edivad99/DimStorage, AGPL-3.0-only) for
// Minecraft Java 1.21.1 NeoForge. The upstream source was read for behaviour
// only; this is a clean-room Rust rewrite.
//
// Upstream keeps one `DimChestStorage` per `Frequency` (an optional owner
// profile and an integer channel) in a world-global `SavedData`, and every
// dimensional chest with that frequency opens the same 54-slot `Container`.
// Horizon gives a mod block entity its own item slots, so this port keeps the
// frequency store here (the truth) and mirrors it into each chest's slots once
// per block-entity tick. A per-chest snapshot makes the mirror a three-way
// merge: local edits (a player or a hopper) are written to the store, remote
// edits (another chest, another dimension) are adopted, and a slot changed on
// both sides keeps the local value.
//
// Slice 1 is the dimensional chest. The tank, tablet, owner/lock screen,
// upgrades and the custom renderer are deferred; see ../notes.md.

use std::collections::{BTreeMap, BTreeSet};

/// A dimensional chest's slots: six rows of nine, as upstream's
/// `DimChestStorage.getContainerSize()`.
pub const SLOTS: usize = 54;

/// Upstream's container stack limit (`getMaxStackSize`). A menu slot still
/// respects the item's own limit below it.
pub const MAX_STACK: u32 = 64;

/// A store key: upstream's `Frequency(Optional<GameProfile>, int channel)`.
/// Slice 1 only creates public frequencies; the owner field is kept so a
/// saved frequency from a later slice is not lost.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Frequency {
    pub owner: Option<String>,
    pub channel: i32,
}

impl Frequency {
    /// `new Frequency()` upstream: public, channel 1.
    pub const DEFAULT: Frequency = Frequency {
        owner: None,
        channel: 1,
    };

    pub fn new(owner: Option<String>, channel: i32) -> Self {
        Frequency { owner, channel }
    }

    pub fn public(channel: i32) -> Self {
        Frequency {
            owner: None,
            channel,
        }
    }

    /// The store file's and block entity's text: `public,1` or `<uuid>,1`.
    pub fn text(&self) -> String {
        format!(
            "{},{}",
            self.owner.as_deref().unwrap_or("public"),
            self.channel
        )
    }

    /// Parse `public,1` or `<uuid>,1`; none for anything else.
    pub fn parse(text: &str) -> Option<Frequency> {
        let (owner, channel) = text.rsplit_once(',')?;
        let channel = channel.parse().ok()?;
        let owner = if owner == "public" {
            None
        } else if owner.is_empty() {
            return None;
        } else {
            Some(owner.to_owned())
        };
        Some(Frequency::new(owner, channel))
    }

    /// A player-facing label (`public / 1`).
    pub fn label(&self) -> String {
        format!(
            "{} / {}",
            self.owner.as_deref().unwrap_or("public"),
            self.channel
        )
    }
}

/// One item slot as the store persists it. The glue implements this for the
/// game's item stack (SNBT text); the tests use a small struct.
pub trait Slot: Clone + PartialEq {
    fn encode(&self) -> String;
    fn decode(text: &str) -> Option<Self>;
}

/// A slot value that can be split and merged. Counts are part of the value,
/// but `same_kind` compares only the item and its components, as
/// `ItemStack.isSameItem` does for a container.
pub trait Stack: Slot {
    fn count(&self) -> u32;
    fn with_count(&self, count: u32) -> Self;
    fn same_kind(&self, other: &Self) -> bool;
}

/// Put as much of `item` as fits into these slots, topping up equal stacks
/// first and then using empty slots; answers how many items were left over.
pub fn deposit<T: Stack>(slots: &mut [Option<T>], item: &T, max_stack: u32) -> u32 {
    let mut remaining = item.count();
    if remaining == 0 || max_stack == 0 {
        return remaining;
    }
    // Vanilla `SimpleContainer.addItem` tops up equal stacks before it uses
    // an empty slot.
    for slot in slots.iter_mut() {
        let Some(existing) = slot.as_mut() else {
            continue;
        };
        if !existing.same_kind(item) {
            continue;
        }
        let room = max_stack.saturating_sub(existing.count());
        if room == 0 {
            continue;
        }
        let add = room.min(remaining);
        *existing = existing.with_count(existing.count() + add);
        remaining -= add;
        if remaining == 0 {
            return 0;
        }
    }
    for slot in slots.iter_mut() {
        if slot.is_some() {
            continue;
        }
        let add = max_stack.min(remaining);
        *slot = Some(item.with_count(add));
        remaining -= add;
        if remaining == 0 {
            return 0;
        }
    }
    remaining
}

/// Take up to `count` items from one slot; answers the stack removed.
pub fn withdraw<T: Stack>(slots: &mut [Option<T>], slot: usize, count: u32) -> Option<T> {
    if count == 0 {
        return None;
    }
    let existing = slots.get(slot)?.clone()?;
    let take = count.min(existing.count());
    if take == 0 {
        return None;
    }
    if take == existing.count() {
        slots[slot] = None;
    } else {
        slots[slot] = Some(existing.with_count(existing.count() - take));
    }
    Some(existing.with_count(take))
}

/// The most channels one world's store keeps. A player can mint channel ids with a command, so
/// the map is bounded: past this, a new channel is refused until one empties (an empty channel is
/// dropped). 4096 channels is far beyond any real shared-storage world.
pub const MAX_CHANNELS: usize = 4096;

/// How many slots hold an item.
pub fn used<T>(slots: &[Option<T>]) -> usize {
    slots.iter().filter(|slot| slot.is_some()).count()
}

/// Every frequency's inventory, the world's truth.
#[derive(Clone, Debug, PartialEq)]
pub struct Store<T> {
    frequencies: BTreeMap<Frequency, Vec<Option<T>>>,
}

impl<T: Slot> Default for Store<T> {
    fn default() -> Self {
        Store {
            frequencies: BTreeMap::new(),
        }
    }
}

impl<T: Slot> Store<T> {
    pub fn new() -> Self {
        Store::default()
    }

    /// Whether this frequency has an entry (an empty entry still counts).
    pub fn contains(&self, frequency: &Frequency) -> bool {
        self.frequencies.contains_key(frequency)
    }

    /// The frequency's slots, `SLOTS` long; empty when it has no entry.
    pub fn slots(&self, frequency: &Frequency) -> Vec<Option<T>> {
        self.frequencies
            .get(frequency)
            .cloned()
            .unwrap_or_else(|| vec![None; SLOTS])
    }

    /// Store a channel's slots; answers whether they were taken.
    ///
    /// An empty channel is not retained: syncing a chest that has never held anything (or one
    /// that just emptied) used to materialize a permanent 54-slot entry, and arbitrary signed
    /// channel ids plus a public retune path made that unbounded. A new channel beyond
    /// [`MAX_CHANNELS`] is refused rather than growing the store without limit; an existing one
    /// always accepts.
    pub fn set(&mut self, frequency: &Frequency, slots: Vec<Option<T>>) -> bool {
        let slots = pad(&slots);
        if used(&slots) == 0 {
            self.frequencies.remove(frequency);
            return false;
        }
        if !self.frequencies.contains_key(frequency) && self.frequencies.len() >= MAX_CHANNELS {
            return false;
        }
        self.frequencies.insert(frequency.clone(), slots);
        true
    }

    /// How many channels the store remembers.
    pub fn channels(&self) -> usize {
        self.frequencies.len()
    }

    /// The non-empty slots of a frequency.
    pub fn count(&self, frequency: &Frequency) -> usize {
        self.frequencies
            .get(frequency)
            .map_or(0, |slots| used(slots))
    }

    /// The store file's text: a header, then one `f` line per frequency and
    /// one `i <slot> <snbt>` line per item.
    pub fn encode(&self) -> String {
        let mut out = String::from("# dimstorage 1\n");
        for (frequency, slots) in &self.frequencies {
            out.push_str("f ");
            out.push_str(&frequency.text());
            out.push('\n');
            for (index, slot) in slots.iter().enumerate() {
                if let Some(item) = slot {
                    out.push_str(&format!("i {index} {}\n", item.encode()));
                }
            }
        }
        out
    }

    /// Parse the store file; bad lines are skipped and reported.
    pub fn decode(text: &str) -> (Self, Vec<String>) {
        let mut store = Store::new();
        let mut warnings = Vec::new();
        let mut current = None;
        for (number, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("f ") {
                match Frequency::parse(rest.trim()) {
                    Some(frequency) => {
                        store
                            .frequencies
                            .entry(frequency.clone())
                            .or_insert_with(|| vec![None; SLOTS]);
                        current = Some(frequency);
                    }
                    None => {
                        warnings.push(format!(
                            "line {}: bad frequency {:?}",
                            number + 1,
                            rest.trim()
                        ));
                        current = None;
                    }
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("i ") {
                let Some(frequency) = current.as_ref() else {
                    warnings.push(format!("line {}: item before a frequency", number + 1));
                    continue;
                };
                let Some((index_text, item_text)) = rest.split_once(' ') else {
                    warnings.push(format!("line {}: item line without a slot", number + 1));
                    continue;
                };
                let Ok(index) = index_text.parse::<usize>() else {
                    warnings.push(format!("line {}: bad slot {:?}", number + 1, index_text));
                    continue;
                };
                if index >= SLOTS {
                    warnings.push(format!("line {}: slot {index} is past {SLOTS}", number + 1));
                    continue;
                }
                match T::decode(item_text) {
                    Some(item) => {
                        if let Some(slots) = store.frequencies.get_mut(frequency)
                            && let Some(slot) = slots.get_mut(index)
                        {
                            *slot = Some(item);
                        }
                    }
                    None => warnings.push(format!("line {}: bad item {:?}", number + 1, item_text)),
                }
                continue;
            }
            warnings.push(format!("line {}: unknown line {line:?}", number + 1));
        }
        (store, warnings)
    }
}

impl<T: Stack> Store<T> {
    /// Deposit into one frequency; answers the leftover count.
    pub fn deposit(&mut self, frequency: &Frequency, item: &T, max_stack: u32) -> u32 {
        let mut slots = self.slots(frequency);
        let left = deposit(&mut slots, item, max_stack);
        if left < item.count() {
            self.set(frequency, slots);
        }
        left
    }

    /// Withdraw from one frequency's slot.
    pub fn withdraw(&mut self, frequency: &Frequency, slot: usize, count: u32) -> Option<T> {
        let mut slots = self.slots(frequency);
        let taken = withdraw(&mut slots, slot, count);
        if taken.is_some() {
            self.set(frequency, slots);
        }
        taken
    }
}

/// One slot's edit, applied to the store's value.
///
/// `snap` is what this chest last saw, `local` what it holds now and `remote` what the store holds
/// now. A same-kind edit is a count delta on the store's value (clamped to a slot); a slot whose
/// item changed is a replacement, so the local value wins.
fn merged_slot<T: Stack>(snap: Option<&T>, local: Option<&T>, remote: Option<&T>) -> Option<T> {
    let same_kind = snap.is_some_and(|snap| local.is_none_or(|local| local.same_kind(snap)));
    if !same_kind {
        return local.cloned();
    }
    let snap = snap.expect("same_kind needs a snapshot");
    if remote.is_some_and(|remote| !remote.same_kind(snap)) {
        // Another chest replaced the slot with a different item at the same time: a tie, and the
        // local edit is the newer value.
        return local.cloned();
    }
    let base = i64::from(remote.map_or(0, Stack::count));
    let delta = i64::from(local.map_or(0, Stack::count)) - i64::from(snap.count());
    let want = base + delta;
    if want <= 0 {
        return None;
    }
    let kept = want.min(i64::from(MAX_STACK)).min(i64::from(u32::MAX)) as u32;
    local.unwrap_or(snap).with_count(kept).into()
}

/// What an edited slot shows: the store's value, unless the chest holds more (an insert that did
/// not fit) — then the local stack, which the chest keeps rather than losing.
fn shown_slot<T: Stack>(local: Option<&T>, authoritative: Option<&T>) -> Option<T> {
    match (local, authoritative) {
        (Some(local), Some(authoritative)) if local.count() > authoritative.count() => {
            Some(local.clone())
        }
        _ => authoritative.cloned(),
    }
}

/// How far from a block a player without operator rights may still use a channel command: the
/// vanilla block-interaction reach (4.5 blocks) plus slack, from the player's position to the
/// block centre. The commands name and retune a chest at any loaded coordinates, so a remote use
/// has to be an operator's.
pub const READ_REACH: f64 = 6.0;

/// Whether a command source may use the chest at `pos`: `operator` is its permission level
/// reaching 2; a player below it may use only a chest within [`READ_REACH`], and a non-player
/// source without it nothing.
#[must_use]
pub fn may_read(operator: bool, player: Option<[f64; 3]>, pos: [i32; 3]) -> bool {
    if operator {
        return true;
    }
    player.is_some_and(|player| {
        let centre = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        let distance_squared: f64 = (0..3)
            .map(|axis| (player[axis] - centre[axis]).powi(2))
            .sum();
        distance_squared <= READ_REACH * READ_REACH
    })
}

/// Whether a withdrawal may run: it needs a recipient who can hold the items.
#[must_use]
pub fn take_allowed(can_hold: bool, slot: usize, count: u32) -> bool {
    can_hold && slot < SLOTS && count > 0
}

/// How a tick's merge went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    /// The store was the truth; the chest took its slots.
    Adopted,
    /// The chest was the only copy of an unknown frequency; it filled the store.
    Bootstrapped,
    /// Slot-wise changes went both ways.
    Merged,
}

/// One chest's tick result.
#[derive(Clone, Debug, PartialEq)]
pub struct Synced<T> {
    pub items: Vec<Option<T>>,
    pub store_changed: bool,
    pub how: Tick,
}

/// Merge one chest's slots with the store. `snapshot` is what the chest held
/// after its previous tick; none on the first tick after it loads, when the
/// store (or, for an unknown frequency that has items, the chest) wins.
pub fn sync<T: Stack>(
    items: &[Option<T>],
    snapshot: Option<&[Option<T>]>,
    store: &mut Store<T>,
    frequency: &Frequency,
) -> Synced<T> {
    let current = pad(items);
    let Some(snapshot) = snapshot else {
        let known = store.contains(frequency);
        if !known && current.iter().any(Option::is_some) {
            store.set(frequency, current.clone());
            return Synced {
                items: current,
                store_changed: true,
                how: Tick::Bootstrapped,
            };
        }
        let stored = store.slots(frequency);
        // An empty, never-visited channel stays unrecorded: materializing its 54 empty slots for
        // every channel a chest is retuned through was the unbounded growth the reviewer found.
        let store_changed = !known && store.set(frequency, stored.clone());
        return Synced {
            items: stored,
            store_changed,
            how: Tick::Adopted,
        };
    };
    let snapshot = pad(snapshot);
    let mut stored = store.slots(frequency);
    let mut merged = current.clone();
    let mut changed = false;
    for index in 0..SLOTS {
        if current[index] != snapshot[index] {
            // The chest edited this slot. A same-item edit is applied as a **count delta** on
            // top of the store's value, not as a new absolute value: two chests that each
            // withdraw 32 from one stack of 64 must subtract twice, and two that each insert must
            // add twice. Storing the local absolute value let the second withdrawal land on the
            // first's 32 and resurrect items. A slot whose item changed is a replacement, which
            // is the one case a mirrored slot cannot reconcile.
            let authoritative = merged_slot(
                snapshot[index].as_ref(),
                current[index].as_ref(),
                stored[index].as_ref(),
            );
            // What the chest shows: the store's value, or — when an insert did not fit — the
            // whole local stack, so an item is never destroyed by the merge.
            merged[index] = shown_slot(current[index].as_ref(), authoritative.as_ref());
            if stored[index] != authoritative {
                stored[index] = authoritative;
                changed = true;
            }
        } else if stored[index] != snapshot[index] {
            // Another chest changed it: take theirs.
            merged[index] = stored[index].clone();
        }
    }
    if changed || !store.contains(frequency) {
        store.set(frequency, stored);
        changed = true;
    }
    Synced {
        items: merged,
        store_changed: changed,
        how: Tick::Merged,
    }
}

/// Write a chest's locally changed slots to the store; whether it changed.
/// Nothing happens without a snapshot: the chest is then not authoritative.
pub fn commit<T: Stack>(
    items: &[Option<T>],
    snapshot: Option<&[Option<T>]>,
    store: &mut Store<T>,
    frequency: &Frequency,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    let current = pad(items);
    let snapshot = pad(snapshot);
    let mut stored = store.slots(frequency);
    let mut changed = false;
    for index in 0..SLOTS {
        if current[index] != snapshot[index] {
            // The same count delta as the tick merge: an access between ticks must subtract from
            // the authoritative store, not overwrite it with a copy's absolute value.
            let authoritative = merged_slot(
                snapshot[index].as_ref(),
                current[index].as_ref(),
                stored[index].as_ref(),
            );
            if stored[index] != authoritative {
                stored[index] = authoritative;
                changed = true;
            }
        }
    }
    if changed {
        store.set(frequency, stored);
    }
    changed
}

/// Re-point a chest at another frequency: commit what it held to the old
/// frequency, then show the new frequency's inventory. Returns the new slots
/// and whether the store changed.
pub fn switch<T: Stack>(
    items: &[Option<T>],
    snapshot: Option<&[Option<T>]>,
    store: &mut Store<T>,
    from: &Frequency,
    to: &Frequency,
) -> (Vec<Option<T>>, bool) {
    if from == to {
        return (store.slots(to), false);
    }
    let changed = commit(items, snapshot, store, from);
    (store.slots(to), changed)
}

// ---------------------------------------------------------------------------
// The dimensional tank's fluid store (slice 2)
// ---------------------------------------------------------------------------

/// A dimensional tank's capacity in millibuckets: upstream's
/// `DimTankStorage.CAPACITY` is 16000, sixteen buckets.
pub const TANK_CAPACITY: i32 = 16_000;

/// What one tank holds: a fluid id and an amount in millibuckets. An amount is
/// always positive; an empty tank is [`Tank::None`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TankFluid {
    pub id: String,
    pub amount: i32,
}

impl TankFluid {
    pub fn new(id: impl Into<String>, amount: i32) -> Self {
        TankFluid {
            id: id.into(),
            amount,
        }
    }

    /// Whether both are the same fluid, whatever the amount.
    #[must_use]
    pub fn same_kind(&self, other: &Self) -> bool {
        self.id == other.id
    }

    /// The same fluid, never past [`TANK_CAPACITY`] and never empty.
    #[must_use]
    pub fn clamped(&self) -> Option<Self> {
        (self.amount > 0).then(|| TankFluid {
            id: self.id.clone(),
            amount: self.amount.min(TANK_CAPACITY),
        })
    }
}

/// One tank's contents: `Some` while it holds fluid.
pub type Tank = Option<TankFluid>;

/// Every frequency's tank, the world's truth beside the chest store.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TankStore {
    tanks: BTreeMap<Frequency, TankFluid>,
    /// Channels this world has known, including ones that now hold nothing. A
    /// drained channel drops its fluid entry, but the marker must survive a save:
    /// otherwise a tank's stale saved copy re-seeds the channel it no longer owns.
    known: BTreeSet<Frequency>,
}

impl TankStore {
    pub fn new() -> Self {
        TankStore::default()
    }

    /// Whether this frequency has a tank entry.
    pub fn contains(&self, frequency: &Frequency) -> bool {
        self.tanks.contains_key(frequency)
    }

    /// Whether this world has ever known this channel, even if it holds nothing
    /// now. A channel that is not known may be bootstrapped from a tank; a known
    /// empty one may not.
    pub fn knows(&self, frequency: &Frequency) -> bool {
        self.known.contains(frequency)
    }

    /// Remember a channel, empty or not; whether the marker was new. Bounded by
    /// [`MAX_CHANNELS`], as a new channel with fluid is.
    pub fn know(&mut self, frequency: &Frequency) -> bool {
        if self.known.contains(frequency) {
            return false;
        }
        if self.known.len() >= MAX_CHANNELS {
            return false;
        }
        self.known.insert(frequency.clone())
    }

    /// The frequency's tank, `None` when it is empty or unknown.
    pub fn get(&self, frequency: &Frequency) -> Tank {
        self.tanks.get(frequency).cloned()
    }

    /// Store a channel's tank; an empty one keeps only its known marker, as the
    /// chest store's empty channel is not retained, and a new channel past
    /// [`MAX_CHANNELS`] is refused. Whether the saved file would change.
    pub fn set(&mut self, frequency: &Frequency, tank: Tank) -> bool {
        let tank = tank.and_then(|fluid| fluid.clamped());
        let Some(tank) = tank else {
            let removed = self.tanks.remove(frequency).is_some();
            let known = self.know(frequency);
            return removed || known;
        };
        if !self.known.contains(frequency) {
            if self.known.len() >= MAX_CHANNELS {
                return false;
            }
            self.known.insert(frequency.clone());
        }
        let changed = self.tanks.get(frequency) != Some(&tank);
        self.tanks.insert(frequency.clone(), tank);
        changed
    }

    /// How many channels this world knows (with fluid or emptied).
    pub fn channels(&self) -> usize {
        self.known.len()
    }

    /// Free millibuckets for `id`: the whole capacity on an empty channel, the
    /// capacity minus what it holds when it holds that fluid, and none for
    /// another fluid.
    pub fn room(&self, frequency: &Frequency, id: &str) -> i32 {
        match self.tanks.get(frequency) {
            Some(held) if held.id != id => 0,
            Some(held) => TANK_CAPACITY.saturating_sub(held.amount),
            None => TANK_CAPACITY,
        }
    }

    /// Whether the channel holds at least `amount` of `id`.
    pub fn holds(&self, frequency: &Frequency, id: &str, amount: i32) -> bool {
        self.tanks
            .get(frequency)
            .is_some_and(|held| held.id == id && held.amount >= amount)
    }

    /// Put `amount` of `id` into a channel; answers how much it took.
    pub fn fill(&mut self, frequency: &Frequency, id: &str, amount: i32) -> i32 {
        let free = self.room(frequency, id);
        let taken = amount.min(free);
        if taken <= 0 {
            return 0;
        }
        let held = self.tanks.get(frequency).map_or(0, |held| held.amount);
        self.set(frequency, Some(TankFluid::new(id, held + taken)));
        taken
    }

    /// Take `amount` of `id` into a channel only when all of it fits: a fill that
    /// does not fit is refused whole, so a container never gives fluid the
    /// channel cannot take. Answers whether it accepted the whole amount.
    pub fn transfer_in(&mut self, frequency: &Frequency, id: &str, amount: i32) -> bool {
        if amount <= 0 || self.room(frequency, id) < amount {
            return false;
        }
        self.fill(frequency, id, amount) == amount
    }

    /// Take up to `amount` of `id` out of a channel; answers what left.
    pub fn drain(&mut self, frequency: &Frequency, id: &str, amount: i32) -> Tank {
        let held = self.tanks.get(frequency)?;
        if held.id != id {
            return None;
        }
        let taken = amount.min(held.amount);
        if taken <= 0 {
            return None;
        }
        self.set(frequency, Some(TankFluid::new(id, held.amount - taken)));
        Some(TankFluid::new(id, taken))
    }

    /// The tank file's text: a header, one `t <frequency> <fluid> <amount>` line
    /// per channel with fluid, and one `e <frequency>` line for a known channel
    /// that holds nothing.
    pub fn encode(&self) -> String {
        let mut out = String::from("# dimstorage-tanks 2\n");
        for frequency in &self.known {
            match self.tanks.get(frequency) {
                Some(tank) => out.push_str(&format!(
                    "t {} {} {}\n",
                    frequency.text(),
                    tank.id,
                    tank.amount
                )),
                None => out.push_str(&format!("e {}\n", frequency.text())),
            }
        }
        out
    }

    /// Parse the tank file; bad lines are skipped and reported.
    pub fn decode(text: &str) -> (Self, Vec<String>) {
        let mut store = TankStore::new();
        let mut warnings = Vec::new();
        for (number, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("e ") {
                let mut parts = rest.split_whitespace();
                let (Some(frequency), None) = (parts.next(), parts.next()) else {
                    warnings.push(format!("line {}: bad tank line {line:?}", number + 1));
                    continue;
                };
                if let Some(frequency) = Frequency::parse(frequency) {
                    store.know(&frequency);
                } else {
                    warnings.push(format!("line {}: bad tank line {line:?}", number + 1));
                }
                continue;
            }
            let Some(rest) = line.strip_prefix("t ") else {
                warnings.push(format!("line {}: unknown line {line:?}", number + 1));
                continue;
            };
            let mut parts = rest.split_whitespace();
            let (Some(frequency), Some(id), Some(amount), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                warnings.push(format!("line {}: bad tank line {line:?}", number + 1));
                continue;
            };
            let (Some(frequency), Ok(amount)) =
                (Frequency::parse(frequency), amount.parse::<i32>())
            else {
                warnings.push(format!("line {}: bad tank line {line:?}", number + 1));
                continue;
            };
            if amount <= 0 {
                warnings.push(format!(
                    "line {}: tank amount {amount} is not positive",
                    number + 1
                ));
                continue;
            }
            store.set(&frequency, Some(TankFluid::new(id, amount)));
        }
        (store, warnings)
    }
}

/// One tank's tick result.
#[derive(Clone, Debug, PartialEq)]
pub struct TankSynced {
    pub tank: Tank,
    pub store_changed: bool,
    pub how: Tick,
}

/// One tank's edit, applied to the store's value: a same-fluid edit is an amount
/// delta on the store's value; a tank whose fluid changed is a replacement, so
/// the local value wins. Both are clamped to [`TANK_CAPACITY`].
#[must_use]
pub fn merged_tank(snapshot: &Tank, local: &Tank, remote: &Tank) -> Tank {
    // An empty snapshot is not a fluid a fill could "replace": a tank that was
    // empty and now holds a bucket is a +1000 delta, exactly as a shared
    // container would have taken it, so two tanks that each empty a bucket into
    // one channel add up. Only a tank that held one fluid and now holds another
    // is a replacement.
    let same_kind = match (snapshot, local) {
        (Some(snapshot), Some(local)) => local.same_kind(snapshot),
        _ => true,
    };
    if !same_kind {
        return local.clone().and_then(|fluid| fluid.clamped());
    }
    if let (Some(theirs), Some(mine)) = (remote, local)
        && !theirs.same_kind(mine)
    {
        // Another tank put a different fluid in at the same time: a tie, and the
        // local edit is the newer value (a tank holds one fluid).
        return local.clone().and_then(|fluid| fluid.clamped());
    }
    // The fluid the channel holds: the local copy's, else what the tank last saw,
    // else the store's — a sibling's fill must keep its id when mirrored here.
    let fluid = local.as_ref().or(snapshot.as_ref()).or(remote.as_ref())?;
    let base = i64::from(remote.as_ref().map_or(0, |remote| remote.amount));
    let delta = i64::from(local.as_ref().map_or(0, |local| local.amount))
        - i64::from(snapshot.as_ref().map_or(0, |snapshot| snapshot.amount));
    let want = (base + delta).clamp(0, i64::from(TANK_CAPACITY)) as i32;
    (want > 0).then(|| TankFluid::new(fluid.id.clone(), want))
}

/// What an edited tank shows: the store's value, unless the tank holds more (a
/// fill that did not fit) — then the local amount, so fluid is never destroyed.
#[must_use]
pub fn shown_tank(local: &Tank, authoritative: &Tank) -> Tank {
    match (local, authoritative) {
        (Some(local), Some(authoritative)) if local.amount > authoritative.amount => {
            Some(local.clone())
        }
        _ => authoritative.clone(),
    }
}

/// Merge one tank's contents with the store: `snapshot` is what it held after its
/// previous tick, none on the first tick after it loads.
pub fn tank_sync(
    local: &Tank,
    snapshot: Option<&Tank>,
    store: &mut TankStore,
    frequency: &Frequency,
) -> TankSynced {
    let Some(snapshot) = snapshot else {
        // A tank with no synced marker (a fresh block, or one saved by 0.2.3): a
        // channel the world has never known may be seeded from the tank's own
        // compound, which recovers a lost store file. A channel it has known —
        // possibly drained to nothing — may not: the tank's copy is stale data,
        // not the channel's balance.
        if !store.knows(frequency) && local.is_some() {
            let store_changed = store.set(frequency, local.clone());
            return TankSynced {
                tank: local.clone(),
                store_changed,
                how: Tick::Bootstrapped,
            };
        }
        let stored = store.get(frequency);
        let store_changed = store.set(frequency, stored.clone()) | store.know(frequency);
        return TankSynced {
            tank: stored,
            store_changed,
            how: Tick::Adopted,
        };
    };
    let stored = store.get(frequency);
    let authoritative = merged_tank(snapshot, local, &stored);
    // A tank that has not edited its copy takes the store's value; only a tank
    // that did edit it may show more than the store holds (a fill that did not
    // fit), so a stale sibling never keeps handing out fluid the store lost.
    let edited = local != snapshot;
    let tank = if edited {
        shown_tank(local, &authoritative)
    } else {
        authoritative.clone()
    };
    let store_changed = stored != authoritative;
    if store_changed {
        store.set(frequency, authoritative);
    }
    TankSynced {
        tank,
        store_changed,
        how: Tick::Merged,
    }
}

/// Write a tank's locally changed contents to the store; whether it changed.
pub fn tank_commit(
    local: &Tank,
    snapshot: Option<&Tank>,
    store: &mut TankStore,
    frequency: &Frequency,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    let stored = store.get(frequency);
    let authoritative = merged_tank(snapshot, local, &stored);
    if stored == authoritative {
        return false;
    }
    store.set(frequency, authoritative);
    true
}

/// Re-point a tank at another frequency: commit what it held to the old one,
/// then show the new one. Answers its contents and whether the store changed.
pub fn tank_switch(
    local: &Tank,
    snapshot: Option<&Tank>,
    store: &mut TankStore,
    from: &Frequency,
    to: &Frequency,
) -> (Tank, bool) {
    if from == to {
        return (store.get(to), false);
    }
    let changed = tank_commit(local, snapshot, store, from);
    (store.get(to), changed)
}

/// One tank's capability transfer: `before` is what its own copy held when the
/// access began, `after` what the game's fluid handler left there. The transfer
/// commits to the channel's one authoritative balance before the caller reports
/// success, and `tank` is what the tank must show afterwards.
#[derive(Clone, Debug, PartialEq)]
pub struct TankTransfer {
    pub tank: Tank,
    pub committed: bool,
}

/// Commit a capability transfer to the channel before it is reported as done.
pub fn tank_transfer(
    before: &Tank,
    after: &Tank,
    store: &mut TankStore,
    frequency: &Frequency,
) -> TankTransfer {
    let known = store.know(frequency);
    let changed = tank_commit(after, Some(before), store, frequency);
    TankTransfer {
        tank: store.get(frequency),
        committed: changed | known,
    }
}

/// `items` padded or cut to `SLOTS`.
fn pad<T: Clone>(items: &[Option<T>]) -> Vec<Option<T>> {
    let mut out = items.to_vec();
    out.resize(SLOTS, None);
    out.truncate(SLOTS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for the game's item stack.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct It {
        id: &'static str,
        count: u32,
    }

    impl Slot for It {
        fn encode(&self) -> String {
            format!("{}:{}", self.id, self.count)
        }

        fn decode(text: &str) -> Option<Self> {
            let (id, count) = text.split_once(':')?;
            Some(It {
                id: Box::leak(id.to_owned().into_boxed_str()),
                count: count.parse().ok()?,
            })
        }
    }

    impl Stack for It {
        fn count(&self) -> u32 {
            self.count
        }

        fn with_count(&self, count: u32) -> Self {
            It { id: self.id, count }
        }

        fn same_kind(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }

    fn it(id: &'static str, count: u32) -> It {
        It { id, count }
    }

    fn empty_slots() -> Vec<Option<It>> {
        vec![None; SLOTS]
    }

    fn stacks(entries: &[(usize, u32)]) -> Vec<Option<It>> {
        let mut slots = empty_slots();
        for &(index, count) in entries {
            slots[index] = Some(it("diamond", count));
        }
        slots
    }

    #[test]
    fn an_empty_channel_is_not_retained() {
        // Visiting a channel with an empty chest must not materialize a permanent 54-slot entry:
        // arbitrary signed channel ids plus a public retune command made that unbounded.
        let mut store = Store::new();
        store.set(&Frequency::public(9), empty_slots());
        assert!(!store.contains(&Frequency::public(9)));
        store.set(&Frequency::public(9), stacks(&[(0, 5)]));
        assert!(store.contains(&Frequency::public(9)));
        // Taking the last item leaves nothing to remember, so the entry goes too.
        assert_eq!(
            store.withdraw(&Frequency::public(9), 0, 5),
            Some(it("diamond", 5))
        );
        assert!(
            !store.contains(&Frequency::public(9)),
            "an emptied channel is dropped, not kept as 54 empty slots"
        );
    }

    #[test]
    fn the_store_refuses_more_channels_than_its_cap() {
        let mut store = Store::new();
        for channel in 0..MAX_CHANNELS {
            let channel = i32::try_from(channel).expect("the cap fits an i32");
            assert!(store.set(&Frequency::public(channel), stacks(&[(0, 1)])));
        }
        let one_too_many = i32::try_from(MAX_CHANNELS).expect("the cap fits an i32");
        assert!(
            !store.set(&Frequency::public(one_too_many), stacks(&[(0, 1)])),
            "a full store refuses a new channel instead of growing without bound"
        );
        assert_eq!(store.channels(), MAX_CHANNELS);
        // An existing channel still accepts items.
        assert!(store.set(&Frequency::public(0), stacks(&[(0, 2)])));
    }

    #[test]
    fn two_chests_each_withdraw_from_one_shared_stack() {
        // Both chests mirror a stack of 64 and each loses 32 before its next sync. Both
        // withdrawals are real (the players hold them), so the store must go 64 -> 32 -> 0
        // instead of storing one chest's absolute value twice.
        let mut store = Store::new();
        store.set(&Frequency::DEFAULT, stacks(&[(0, 64)]));
        let snapshot = stacks(&[(0, 64)]);
        let first = sync(
            &stacks(&[(0, 32)]),
            Some(&snapshot),
            &mut store,
            &Frequency::DEFAULT,
        );
        assert_eq!(first.items[0], Some(it("diamond", 32)));
        assert_eq!(store.slots(&Frequency::DEFAULT)[0], Some(it("diamond", 32)));
        let second = sync(
            &stacks(&[(0, 32)]),
            Some(&snapshot),
            &mut store,
            &Frequency::DEFAULT,
        );
        assert_eq!(
            store.slots(&Frequency::DEFAULT)[0],
            None,
            "the second withdrawal subtracts what the first left"
        );
        assert_eq!(second.items[0], None);
    }

    #[test]
    fn two_chests_each_insert_into_one_shared_stack() {
        let mut store = Store::new();
        store.set(&Frequency::DEFAULT, stacks(&[(0, 10)]));
        let snapshot = stacks(&[(0, 10)]);
        let _first = sync(
            &stacks(&[(0, 30)]),
            Some(&snapshot),
            &mut store,
            &Frequency::DEFAULT,
        );
        let second = sync(
            &stacks(&[(0, 30)]),
            Some(&snapshot),
            &mut store,
            &Frequency::DEFAULT,
        );
        assert_eq!(
            store.slots(&Frequency::DEFAULT)[0],
            Some(it("diamond", 50)),
            "both insertions add: 10 + 20 + 20"
        );
        assert_eq!(second.items[0], Some(it("diamond", 50)));
    }

    #[test]
    fn a_sibling_that_has_not_changed_adopts_the_committed_withdrawal() {
        // The chest's inventory is a copy of the channel; a sibling keeps its copy between ticks.
        // The fix commits an access to the store at once and pushes the store's slots into every
        // loaded sibling, so this is what a sibling's next merge must see: the committed value,
        // never the stale stack it still held.
        let mut store = Store::new();
        store.set(&Frequency::DEFAULT, stacks(&[(0, 64)]));
        let before = stacks(&[(0, 64)]);
        // A player empties the first chest: its access commits the delta at once.
        assert!(commit(
            &empty_slots(),
            Some(&before),
            &mut store,
            &Frequency::DEFAULT
        ));
        assert_eq!(store.slots(&Frequency::DEFAULT)[0], None);
        // The sibling's copy still reads 64; its merge shows the store, so it can hand out nothing.
        let sibling = sync(&before, Some(&before), &mut store, &Frequency::DEFAULT);
        assert_eq!(sibling.items[0], None);
        assert_eq!(
            store.slots(&Frequency::DEFAULT)[0],
            None,
            "the store is unchanged by a sibling that has nothing of its own to commit"
        );
    }

    #[test]
    fn only_an_operator_or_a_nearby_player_uses_a_channel_command() {
        let pos = [10, 64, 10];
        assert!(may_read(true, Some([500.0, 64.0, 500.0]), pos));
        assert!(may_read(false, Some([13.0, 64.5, 10.5]), pos));
        assert!(!may_read(false, Some([10.5, 64.5, 17.0]), pos));
        assert!(!may_read(false, None, pos));
        assert!(may_read(false, Some([10.5 + READ_REACH, 64.5, 10.5]), pos));
        // A withdrawal also needs someone who can hold the items.
        assert!(!take_allowed(false, 0, 1), "a spectator receives nothing");
        assert!(take_allowed(true, 0, 1));
    }

    #[test]
    fn default_frequency_is_public_channel_one() {
        assert_eq!(Frequency::DEFAULT, Frequency::public(1));
        assert_eq!(Frequency::DEFAULT.text(), "public,1");
        assert_eq!(Frequency::DEFAULT.label(), "public / 1");
    }

    #[test]
    fn frequency_text_round_trips() {
        let public = Frequency::public(7);
        assert_eq!(Frequency::parse(&public.text()), Some(public));

        let owned = Frequency::new(Some("3e1f".to_owned()), 12);
        assert_eq!(Frequency::parse(&owned.text()), Some(owned));
        assert_eq!(Frequency::parse("owner-without-channel"), None);
        assert_eq!(Frequency::parse("public,nope"), None);
    }

    #[test]
    fn deposit_tops_up_then_uses_empty_slots() {
        let mut slots = empty_slots();
        let left = deposit(&mut slots, &it("diamond", 70), MAX_STACK);
        assert_eq!(left, 0);
        assert_eq!(slots[0], Some(it("diamond", 64)));
        assert_eq!(slots[1], Some(it("diamond", 6)));
        assert!(slots[2..].iter().all(Option::is_none));
    }

    #[test]
    fn deposit_returns_what_does_not_fit() {
        let mut slots = empty_slots();
        for slot in &mut slots {
            *slot = Some(it("stone", 64));
        }
        assert_eq!(deposit(&mut slots, &it("stone", 5), MAX_STACK), 5);
        assert_eq!(used(&slots), SLOTS);
    }

    #[test]
    fn capacity_is_54_slots() {
        assert_eq!(SLOTS, 54);
        let mut slots = empty_slots();
        let left = deposit(
            &mut slots,
            &it("cobblestone", SLOTS as u32 * MAX_STACK + 1),
            MAX_STACK,
        );
        assert_eq!(left, 1);
        assert_eq!(used(&slots), SLOTS);
        assert!(
            slots
                .iter()
                .all(|slot| slot.as_ref().is_some_and(|s| s.count == 64))
        );
    }

    #[test]
    fn withdraw_splits_a_stack() {
        let mut slots = empty_slots();
        slots[3] = Some(it("diamond", 10));
        assert_eq!(withdraw(&mut slots, 3, 4), Some(it("diamond", 4)));
        assert_eq!(slots[3], Some(it("diamond", 6)));
    }

    #[test]
    fn withdraw_empties_a_slot() {
        let mut slots = empty_slots();
        slots[3] = Some(it("diamond", 2));
        assert_eq!(withdraw(&mut slots, 3, 9), Some(it("diamond", 2)));
        assert_eq!(slots[3], None);
    }

    #[test]
    fn withdraw_refuses_empty_and_bad_slots() {
        let mut slots = empty_slots();
        assert_eq!(withdraw(&mut slots, 0, 1), None);
        assert_eq!(withdraw(&mut slots, SLOTS, 1), None);
        slots[0] = Some(it("diamond", 2));
        assert_eq!(withdraw(&mut slots, 0, 0), None);
        assert_eq!(slots[0], Some(it("diamond", 2)));
    }

    #[test]
    fn store_is_keyed_by_frequency() {
        let mut store = Store::new();
        store.deposit(&Frequency::public(1), &it("diamond", 3), MAX_STACK);
        assert_eq!(store.count(&Frequency::public(1)), 1);
        assert_eq!(store.slots(&Frequency::public(2)), empty_slots());
        assert_eq!(store.withdraw(&Frequency::public(2), 0, 1), None);
    }

    #[test]
    fn first_sync_adopts_the_store() {
        let frequency = Frequency::public(1);
        let mut store = Store::new();
        let mut stored = empty_slots();
        stored[0] = Some(it("diamond", 5));
        store.set(&frequency, stored.clone());

        let synced = sync(&empty_slots(), None, &mut store, &frequency);
        assert_eq!(synced.how, Tick::Adopted);
        assert_eq!(synced.items, stored);
        assert!(!synced.store_changed);
    }

    #[test]
    fn unknown_frequency_bootstraps_from_the_chest() {
        let frequency = Frequency::public(1);
        let mut store = Store::new();
        let mut chest = empty_slots();
        chest[2] = Some(it("emerald", 3));

        let synced = sync(&chest, None, &mut store, &frequency);
        assert_eq!(synced.how, Tick::Bootstrapped);
        assert_eq!(synced.items, chest);
        assert!(synced.store_changed);
        assert_eq!(store.slots(&frequency), chest);
    }

    #[test]
    fn local_change_is_written_and_remote_change_adopted() {
        let frequency = Frequency::public(1);
        let mut store = Store::new();
        let old = empty_slots();
        store.set(&frequency, old.clone());

        let mut chest = old.clone();
        chest[0] = Some(it("diamond", 2));
        let synced = sync(&chest, Some(&old), &mut store, &frequency);
        assert_eq!(synced.items, chest);
        assert!(synced.store_changed);
        assert_eq!(store.slots(&frequency)[0], Some(it("diamond", 2)));

        let mut remote = chest.clone();
        remote[1] = Some(it("gold", 4));
        store.set(&frequency, remote.clone());
        let synced = sync(&chest, Some(&chest), &mut store, &frequency);
        assert_eq!(synced.items[1], Some(it("gold", 4)));
        assert_eq!(synced.items[0], Some(it("diamond", 2)));
    }

    #[test]
    fn a_slot_changed_on_both_sides_keeps_the_chest() {
        let frequency = Frequency::public(1);
        let old = empty_slots();
        let mut chest = old.clone();
        chest[0] = Some(it("diamond", 2));
        let mut stored = old.clone();
        stored[0] = Some(it("diamond", 7));
        let mut store = Store::new();
        store.set(&frequency, stored);

        let synced = sync(&chest, Some(&old), &mut store, &frequency);
        assert_eq!(synced.items[0], Some(it("diamond", 2)));
        assert_eq!(store.slots(&frequency)[0], Some(it("diamond", 2)));
    }

    #[test]
    fn commit_writes_local_changes_only() {
        let frequency = Frequency::public(1);
        let old = empty_slots();
        let mut store = Store::new();
        store.set(&frequency, old.clone());
        let mut chest = old.clone();
        chest[4] = Some(it("coal", 9));

        assert!(commit(&chest, Some(&old), &mut store, &frequency));
        assert_eq!(store.slots(&frequency)[4], Some(it("coal", 9)));
        assert!(!commit(&chest, None, &mut store, &frequency));
    }

    #[test]
    fn switch_commits_the_old_frequency() {
        let old = Frequency::public(1);
        let new = Frequency::public(2);
        let snapshot = empty_slots();
        let mut chest = empty_slots();
        chest[0] = Some(it("diamond", 3));
        let mut store = Store::new();
        store.set(&old, snapshot.clone());
        let mut other = empty_slots();
        other[1] = Some(it("emerald", 2));
        store.set(&new, other.clone());

        let (items, changed) = switch(&chest, Some(&snapshot), &mut store, &old, &new);
        assert!(changed);
        assert_eq!(store.slots(&old)[0], Some(it("diamond", 3)));
        assert_eq!(items, other);
    }

    #[test]
    fn store_text_round_trips_and_reports_bad_lines() {
        let mut store = Store::new();
        let mut slots = empty_slots();
        slots[0] = Some(it("diamond", 5));
        slots[53] = Some(it("emerald", 1));
        store.set(&Frequency::public(1), slots.clone());
        let text = store.encode();
        let (decoded, warnings) = Store::<It>::decode(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(decoded.slots(&Frequency::public(1)), slots);

        let (decoded, warnings) = Store::<It>::decode(
            "# dimstorage 1\nf public,1\ni 0 diamond:2\ni 99 diamond:1\nnonsense\n",
        );
        assert_eq!(
            decoded.slots(&Frequency::public(1))[0],
            Some(it("diamond", 2))
        );
        assert_eq!(warnings.len(), 2);
    }

    // -- the tank (slice 2) ------------------------------------------------

    fn water(amount: i32) -> Tank {
        Some(TankFluid::new("minecraft:water", amount))
    }

    fn lava(amount: i32) -> Tank {
        Some(TankFluid::new("minecraft:lava", amount))
    }

    #[test]
    fn a_tank_channel_holds_at_most_sixteen_buckets() {
        // Upstream's DimTankStorage.CAPACITY is 16000 mB. A fill past it takes only
        // what is free, and a different fluid never mixes into a full tank.
        let mut store = TankStore::new();
        assert_eq!(
            store.fill(&Frequency::public(1), "minecraft:water", 17_000),
            TANK_CAPACITY
        );
        assert_eq!(store.get(&Frequency::public(1)), water(TANK_CAPACITY));
        assert_eq!(
            store.fill(&Frequency::public(1), "minecraft:lava", 1_000),
            0
        );
        assert_eq!(store.get(&Frequency::public(1)), water(TANK_CAPACITY));
    }

    #[test]
    fn two_tanks_each_fill_a_bucket_into_one_shared_tank() {
        // Two loaded tanks of one channel each empty a water bucket before their
        // next tick. Both fills are real, so the store must go 0 -> 1000 -> 2000
        // instead of storing one tank's absolute amount twice.
        let mut store = TankStore::new();
        let empty: Tank = None;
        let snapshot = empty.clone();
        let first = tank_sync(
            &water(1000),
            Some(&snapshot),
            &mut store,
            &Frequency::DEFAULT,
        );
        assert_eq!(first.tank, water(1000));
        assert!(first.store_changed);
        let second = tank_sync(
            &water(1000),
            Some(&snapshot),
            &mut store,
            &Frequency::DEFAULT,
        );
        assert_eq!(
            store.get(&Frequency::DEFAULT),
            water(2000),
            "both bucket fills add: 1000 + 1000"
        );
        assert_eq!(second.tank, water(2000));
    }

    #[test]
    fn an_untouched_sibling_adopts_the_channels_fluid_with_its_id() {
        // A tank that has never held anything shows the channel's fluid, taken
        // from the store: its id comes with it, never an empty string (the e2e
        // read "1000 / 16000 mB of " when the mirroring tank built the value
        // from its own empty copy).
        let mut store = TankStore::new();
        store.fill(&Frequency::DEFAULT, "minecraft:water", 1000);
        let empty: Tank = None;
        let snapshot = empty.clone();
        let synced = tank_sync(&empty, Some(&snapshot), &mut store, &Frequency::DEFAULT);
        assert_eq!(synced.tank, water(1000));
        assert!(!synced.store_changed);
    }

    #[test]
    fn a_tank_that_has_not_changed_adopts_a_sibling_drain() {
        // A sibling empties a bucket from the channel between this tank's ticks; its
        // mirrored copy still reads the old amount but must show the store's.
        let mut store = TankStore::new();
        store.fill(&Frequency::DEFAULT, "minecraft:water", 2000);
        let snapshot = water(2000);
        assert_eq!(
            store.drain(&Frequency::DEFAULT, "minecraft:water", 1000),
            water(1000)
        );
        let synced = tank_sync(
            &water(2000),
            Some(&snapshot),
            &mut store,
            &Frequency::DEFAULT,
        );
        assert_eq!(synced.tank, water(1000));
        assert!(!synced.store_changed);
    }

    #[test]
    fn a_tank_holding_another_fluid_replaces_rather_than_adds() {
        // A tank whose fluid changed is a replacement: lava in a water channel is
        // never summed with the water it replaced.
        let mut store = TankStore::new();
        store.fill(&Frequency::DEFAULT, "minecraft:water", 2000);
        let snapshot = water(2000);
        let synced = tank_sync(
            &lava(1000),
            Some(&snapshot),
            &mut store,
            &Frequency::DEFAULT,
        );
        assert_eq!(synced.tank, lava(1000));
        assert_eq!(store.get(&Frequency::DEFAULT), lava(1000));
    }

    #[test]
    fn draining_more_than_a_tank_holds_takes_what_it_has() {
        let mut store = TankStore::new();
        store.fill(&Frequency::public(3), "minecraft:water", 500);
        assert_eq!(
            store.drain(&Frequency::public(3), "minecraft:water", 1000),
            water(500)
        );
        assert_eq!(store.get(&Frequency::public(3)), None);
        assert!(
            !store.contains(&Frequency::public(3)),
            "an emptied tank channel is dropped, not kept as 0 mB"
        );
        assert_eq!(
            store.drain(&Frequency::public(3), "minecraft:water", 1),
            None
        );
    }

    #[test]
    fn an_empty_tank_channel_is_not_retained() {
        let mut store = TankStore::new();
        store.set(&Frequency::public(9), None);
        assert!(!store.contains(&Frequency::public(9)));
        store.set(&Frequency::public(9), water(1));
        assert!(store.contains(&Frequency::public(9)));
        store.set(
            &Frequency::public(9),
            Some(TankFluid::new("minecraft:water", 0)),
        );
        assert!(
            !store.contains(&Frequency::public(9)),
            "a zero amount is an empty tank, not an entry"
        );
    }

    #[test]
    fn the_tank_store_refuses_more_channels_than_its_cap() {
        let mut store = TankStore::new();
        for channel in 0..MAX_CHANNELS {
            let channel = i32::try_from(channel).expect("the cap fits an i32");
            assert!(store.set(&Frequency::public(channel), water(1)));
        }
        let one_too_many = i32::try_from(MAX_CHANNELS).expect("the cap fits an i32");
        assert!(!store.set(&Frequency::public(one_too_many), water(1)));
        assert_eq!(store.channels(), MAX_CHANNELS);
    }

    #[test]
    fn the_tank_store_text_round_trips_and_reports_bad_lines() {
        let mut store = TankStore::new();
        store.fill(&Frequency::public(1), "minecraft:water", 3000);
        store.fill(&Frequency::public(2), "minecraft:lava", 1000);
        let text = store.encode();
        assert!(text.contains("t public,1 minecraft:water 3000"), "{text}");
        let (decoded, warnings) = TankStore::decode(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(decoded.get(&Frequency::public(1)), water(3000));
        assert_eq!(decoded.get(&Frequency::public(2)), lava(1000));

        let (decoded, warnings) =
            TankStore::decode("# dimstorage-tanks 1\nt public,1 minecraft:water 500\nt bogus\n");
        assert_eq!(decoded.get(&Frequency::public(1)), water(500));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn switching_a_tank_commits_the_old_channel() {
        let old = Frequency::public(1);
        let new = Frequency::public(2);
        let mut store = TankStore::new();
        store.fill(&new, "minecraft:lava", 500);
        let snapshot: Tank = None;
        let (tank, changed) = tank_switch(&lava(2000), Some(&snapshot), &mut store, &old, &new);
        assert!(changed);
        assert_eq!(store.get(&old), lava(2000));
        assert_eq!(tank, lava(500));
    }

    // -- the reviewer's four tank-transfer cases (0.2.4) --------------------

    #[test]
    fn a_transfer_through_one_tank_is_in_the_channel_before_its_sibling_reads_it() {
        // Two tanks of one channel both mirror 16,000. The first drains a bucket
        // through its own copy; its sibling is read before either tank's next
        // tick. The bucket that came out is real, so the channel must already
        // hold 15,000; a channel that still says 16,000 serves the same fluid
        // twice.
        let mut store = TankStore::new();
        store.fill(&Frequency::DEFAULT, "minecraft:water", TANK_CAPACITY);
        let transfer = tank_transfer(
            &water(TANK_CAPACITY),
            &water(TANK_CAPACITY - 1000),
            &mut store,
            &Frequency::DEFAULT,
        );
        assert!(transfer.committed, "the access commits before it answers");
        assert_eq!(transfer.tank, water(TANK_CAPACITY - 1000));
        assert_eq!(
            store.get(&Frequency::DEFAULT),
            water(TANK_CAPACITY - 1000),
            "the drained bucket is out of the channel, not only out of one tank"
        );
        let sibling = tank_sync(
            &water(TANK_CAPACITY),
            Some(&water(TANK_CAPACITY)),
            &mut store,
            &Frequency::DEFAULT,
        );
        assert_eq!(
            sibling.tank,
            water(TANK_CAPACITY - 1000),
            "the sibling shows the channel, never its own stale copy"
        );
    }

    #[test]
    fn a_transfer_into_a_tank_removed_before_its_next_tick_is_not_lost() {
        // A tank empties a bucket into the channel and the block is broken (or
        // its chunk saved and unloaded) before its next tick. The transfer is
        // finished, so the channel must keep the 1,000 even though no tick runs.
        let mut store = TankStore::new();
        let transfer = tank_transfer(&None, &water(1000), &mut store, &Frequency::DEFAULT);
        assert!(
            transfer.committed,
            "a finished transfer is committed at once"
        );
        assert_eq!(transfer.tank, water(1000));
        assert_eq!(store.get(&Frequency::DEFAULT), water(1000));
    }

    #[test]
    fn a_restart_does_not_restore_a_drained_channel_from_a_stale_tank() {
        // The channel held 1,000 and was drained to nothing, so its fluid entry is
        // dropped; only the fact that the channel is known survives in the file.
        let mut store = TankStore::new();
        store.fill(&Frequency::DEFAULT, "minecraft:water", 1000);
        assert_eq!(
            store.drain(&Frequency::DEFAULT, "minecraft:water", 1000),
            water(1000)
        );
        let (mut restarted, warnings) = TankStore::decode(&store.encode());
        assert!(warnings.is_empty(), "{warnings:?}");
        // A tank saved before the drain still carries water 1,000 in its own
        // compound; on the first tick after the restart it must not re-seed the
        // channel it no longer owns.
        let synced = tank_sync(&water(1000), None, &mut restarted, &Frequency::DEFAULT);
        assert_eq!(
            synced.tank, None,
            "a drained channel is not re-seeded by a stale tank"
        );
        assert_eq!(restarted.get(&Frequency::DEFAULT), None);
    }

    #[test]
    fn a_fill_that_does_not_fit_is_refused_before_any_of_it_lands() {
        // 15,500 mB held; a 1,000 mB bucket does not fit. The channel must refuse
        // the whole transfer rather than take the 500 free, so the container is
        // left full and both sides keep exactly what they had.
        let mut store = TankStore::new();
        store.fill(&Frequency::DEFAULT, "minecraft:water", 15_500);
        let accepted = store.transfer_in(&Frequency::DEFAULT, "minecraft:water", 1000);
        assert!(!accepted, "a fill that does not fit is refused whole");
        assert_eq!(store.get(&Frequency::DEFAULT), water(15_500));
    }
}
