// The mod's settings page: its current values as button labels.
fn t3craft_settings(game: &mut crate::game::Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id != t3craft_engine::ID {
        return;
    }
    let _ = game;
    buttons.extend(t3craft_engine::settings_labels());
}
