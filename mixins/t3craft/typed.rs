// Printable text over the mod's screen, into the reply box when focused.
fn t3craft_typed(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    c: char,
    modifiers: u32,
    canceled: &mut bool,
) {
    let _ = screen;
    if game.native_mod_screen() != Some(t3craft_engine::ID) {
        return;
    }
    if t3craft_engine::screen_char(c, modifiers) {
        *canceled = true;
    }
}
