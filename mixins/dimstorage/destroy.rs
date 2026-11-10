// A broken chest commits its last changes and empties its slots.
fn observe_destroy_block(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    state: BlockStateId,
    canceled: &mut bool,
) {
    crate::game::dimstorage_state::on_destroy_block(game, pos, state, canceled);
}
