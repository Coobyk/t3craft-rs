// The wheel over the mod's screen scrolls the pane under the cursor.
fn t3craft_scroll(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    scroll: [f64; 2],
    _modifiers: u32,
    canceled: &mut bool,
) {
    let _ = screen;
    if game.native_mod_screen() != Some(t3craft_engine::ID) {
        return;
    }
    t3craft_engine::screen_scroll(game.native_cursor(), scroll[1]);
    *canceled = true;
}
