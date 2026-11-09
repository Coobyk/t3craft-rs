// A click on the mod's settings page: change the setting and keep it.
fn t3craft_click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id != t3craft_engine::ID {
        return;
    }
    let _ = game;
    t3craft_engine::settings_click(button);
}
