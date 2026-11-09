// Every client tick: the engine's first start, its event queue into the
// game (open or close the mod's screen, print its chat lines).
fn t3craft_tick(game: &mut crate::game::Game<'_>) {
    if !game.native_on(t3craft_engine::ID) {
        return;
    }
    let Ok(config_dir) = crate::game::Game::native_config_dir() else {
        return;
    };
    t3craft_engine::ensure_started(config_dir);
    let ours = game.native_mod_screen() == Some(t3craft_engine::ID);
    let effects = t3craft_engine::tick(ours);
    if effects.open_screen {
        game.native_mod_screen_open(t3craft_engine::ID);
    }
    if effects.close_screen {
        game.native_mod_screen_close(t3craft_engine::ID);
    }
    for line in effects.chat {
        game.native_chat(&line);
    }
}
