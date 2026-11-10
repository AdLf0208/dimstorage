// The world opened or closed: load or save the frequency store.
fn observe_world_lifecycle(game: &mut crate::game::Game<'_>, phase: WorldPhase) {
    crate::game::dimstorage_state::on_world_lifecycle(game, phase);
}
