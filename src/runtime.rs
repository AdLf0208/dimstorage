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

use std::collections::BTreeMap;

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

    pub fn set(&mut self, frequency: &Frequency, slots: Vec<Option<T>>) {
        self.frequencies.insert(frequency.clone(), pad(&slots));
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
pub fn sync<T: Slot>(
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
        let store_changed = !known;
        if store_changed {
            store.set(frequency, stored.clone());
        }
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
            // The chest changed this slot: it is the newer value.
            if stored[index] != current[index] {
                stored[index] = current[index].clone();
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
pub fn commit<T: Slot>(
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
        if current[index] != snapshot[index] && stored[index] != current[index] {
            stored[index] = current[index].clone();
            changed = true;
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
pub fn switch<T: Slot>(
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
}
