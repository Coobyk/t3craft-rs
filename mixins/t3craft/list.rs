// The mod's row in Options > Mods, with its two rebindable keys.
fn t3craft_listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::native_hooks::NativeMod {
        id: t3craft_engine::ID.into(),
        title: format!("{} {}", t3craft_engine::NAME, t3craft_engine::VERSION),
        description: t3craft_engine::DESCRIPTION.into(),
        keys: vec![
            [
                t3craft_engine::KEY_OPEN.into(),
                t3craft_engine::KEY_OPEN_LABEL.into(),
                t3craft_engine::KEY_OPEN_DEFAULT.into(),
            ],
            [
                t3craft_engine::KEY_HUD.into(),
                t3craft_engine::KEY_HUD_LABEL.into(),
                t3craft_engine::KEY_HUD_DEFAULT.into(),
            ],
        ],
    });
}
