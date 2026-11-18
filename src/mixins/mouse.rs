// A mouse press over the mod's screen: everything on it is the mod's, so a
// press there is always spent (the click goes no further).
fn t3craft_mouse(
    game: &Game<'_>,
    button: i32,
    canceled: &mut bool,
) {
    if game.mod_screen() != Some(t3craft_engine::ID) {
        return;
    }
    t3craft_engine::screen_click(game.cursor(), button == 1);
    *canceled = true;
}
