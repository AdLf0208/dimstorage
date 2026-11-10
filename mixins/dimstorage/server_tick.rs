// End of a running tick: debounce the store write.
fn observe_server_tick(game: &mut crate::game::Game<'_>, phase: ServerTickPhase) {
    crate::game::dimstorage_state::on_server_tick(game, phase);
}
