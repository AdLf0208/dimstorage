// Declare `/dimstorage channel|info`.
fn observe_register(commands: &mut Vec<NativeCommand>) {
    crate::game::dimstorage_state::on_register_commands(commands);
}
