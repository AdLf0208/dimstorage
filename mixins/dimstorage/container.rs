// The chest block entity has 54 slots and the vanilla 9x6 menu.
fn observe_block_entity_container(
    block: &str,
    container: &mut Option<horizon_client::containers::ModContainer>,
) {
    crate::game::dimstorage_state::on_block_entity_container(block, container);
}
