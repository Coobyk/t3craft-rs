// A key went down: the player's chosen bindings of the mod's two keys.
fn t3craft_key(game: &Game<'_>, key: &str, action: mod_api::KeyAction) {
    if action != mod_api::KeyAction::Press {
        return;
    }
    let bound = |name: &str| game.mod_key(t3craft_engine::ID, name).map(|(key, _)| key);
    if bound(t3craft_engine::KEY_OPEN).as_deref() == Some(key) {
        t3craft_engine::open_threads();
    } else if bound(t3craft_engine::KEY_HUD).as_deref() == Some(key) {
        t3craft_engine::toggle_hud();
    }
}
