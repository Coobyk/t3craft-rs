// A key went down or up: the player's chosen bindings of the mod's two keys.
fn t3craft_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    if !game.native_on(t3craft_engine::ID) || !pressed {
        return;
    }
    if key
        == game.native_mod_key(
            t3craft_engine::ID,
            t3craft_engine::KEY_OPEN,
            t3craft_engine::KEY_OPEN_DEFAULT,
        )
    {
        t3craft_engine::open_threads();
    } else if key
        == game.native_mod_key(
            t3craft_engine::ID,
            t3craft_engine::KEY_HUD,
            t3craft_engine::KEY_HUD_DEFAULT,
        )
    {
        t3craft_engine::toggle_hud();
    }
}
