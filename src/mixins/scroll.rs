// The wheel over the mod's screen scrolls the pane under the cursor.
fn t3craft_scroll(
    game: &Game<'_>,
    scroll: [f64; 2],
    canceled: &mut bool,
) {
    if game.mod_screen() != Some(t3craft_engine::ID) {
        return;
    }
    t3craft_engine::screen_scroll(game.cursor(), scroll[1]);
    *canceled = true;
}
