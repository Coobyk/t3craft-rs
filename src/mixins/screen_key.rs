// A named key over the mod's screen: the reply box's editing keys and the
// list's up/down; Escape falls through so the game closes the screen.
fn t3craft_screen_key(
    game: &Game<'_>,
    key: &str,
    repeat: bool,
    modifiers: u32,
    canceled: &mut bool,
) {
    if game.mod_screen() != Some(t3craft_engine::ID) {
        return;
    }
    if t3craft_engine::screen_key(key, repeat, modifiers) {
        *canceled = true;
    }
}
