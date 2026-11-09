// A mouse press over the mod's screen: everything on it is the mod's, so a
// press there is always spent (the click goes no further).
fn t3craft_mouse(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    button: i32,
    _modifiers: u32,
    canceled: &mut bool,
) {
    let _ = screen;
    if game.native_mod_screen() != Some(t3craft_engine::ID) {
        return;
    }
    t3craft_engine::screen_click(game.native_cursor(), button);
    *canceled = true;
}
