//! DimStorage for Horizon — frequency-linked dimensional chests.
//!
//! Ported from DimStorage 9.1.1 (`Edivad99/DimStorage`, AGPL-3.0-only). [`runtime`] is the pure
//! truth — the frequency, the 54-slot store, the three-way merge and the store file codec, with its
//! tests — and is byte-identical to the retired `.hnmod` port's file. [`support`] is the glue: the
//! chest's block entity Part, the store file, the break/explosion accounting and the commands. This
//! file is the one `impl Hooks`.
//!
//! The 2026-11-17 mod-API merge (#16941, launcher ≥ 0.2.70) retired `.hnmod`: the plan, its
//! injection points and the per-hook helper files are gone. A hook is a typed method here, and a
//! moment that belongs to one block entity is a **Part** attached to its declared type.
//!
//! # Where the retired container's hooks went
//!
//! | `.hnmod` hook (API) | here |
//! |---|---|
//! | the `content_root` injection (AE2 pattern) | gone: one flat `.hmod` carries `assets/` and `data/` itself |
//! | `block_entity` (15) | [`support::Chest::block_entity`] (`BlockEntityPart`) |
//! | `block_entity_ticks` (15) | [`support::Chest::block_entity_ticks`] |
//! | `block_entity_container` (16/17) | `Registrar::block_container` in [`Mod::register`] |
//! | `block_entity_tick` (17) | [`support::Chest::block_entity_tick`] |
//! | `destroy_block` (14) | [`Mod::break_block`] → `support::break_block` |
//! | `explosion_detonate` (22) | [`Mod::explosion_detonate`] → `support::explosion_detonate` |
//! | `world_lifecycle` (3) | [`Mod::world_lifecycle`] → `support::world_lifecycle` |
//! | `server_tick` (4) | [`Mod::server_tick`] → `support::server_tick` |
//! | `server_stopping` (44) | [`Mod::server_stopping`] → `support::server_stopping` |
//! | `register_commands` (8) | [`Mod::register_commands`] → `support::declare_commands` |
//! | `perform_command` (8) | [`Mod::run_mod_command`] → `support::run_command` (typed `CommandInvocation`) |
pub mod runtime;
pub mod support;

use mod_api::{
    self, ExplosionDetonateEvent, Flow, Hooks, RegisterCommandsEvent, Registrar,
    RegistrationContext, RunModCommandEvent, ServerHost, ServerStoppingEvent, ServerTickEvent,
    WorldLifecycleEvent,
};

/// The mod keeps its state in the shared store, not in this struct: one store serves every chest in
/// every dimension (see `support`).
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    /// Declare what the game answers for the chest (`EntityBlock` plus the container capability): a
    /// block entity type with its Part, its first saved compound, and the 54-slot 9x6 container.
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        support::declare(r);
    }

    /// `/dimstorage channel|info|put|take`.
    fn register_commands(&self, _host: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        support::declare_commands(event.commands);
    }

    /// Run one of the commands above and take it.
    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> Flow {
        support::run_command(host, event)
    }

    /// A player broke a chest: commit its last changes and empty its slots, so the store's only
    /// copy does not spill.
    fn break_block(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::BreakBlockEvent<'_>,
    ) -> Flow {
        support::break_block(host, event)
    }

    /// An explosion about to break chests: the same commit and empty for each one.
    fn explosion_detonate(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ExplosionDetonateEvent<'_>,
    ) {
        support::explosion_detonate(host, event);
    }

    /// The world closed: save the store and forget it. It is loaded lazily on the first server tick,
    /// because `WorldOpenReady` is not a phase at this pin.
    fn world_lifecycle(&self, host: &mut dyn ServerHost, event: &mut WorldLifecycleEvent<'_>) {
        support::world_lifecycle(host, event);
    }

    /// End of a running tick: the store write is debounced.
    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        support::server_tick(host, event);
    }

    /// The server is stopping: flush the store.
    fn server_stopping(&self, host: &mut dyn ServerHost, event: &mut ServerStoppingEvent) {
        support::server_stopping(host, event);
    }
}
