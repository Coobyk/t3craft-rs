// The mod's own screen (Screen::Mod draws nothing itself): render the
// engine's draw commands each frame and keep its hit regions fresh.
fn t3craft_draw(
    game: &Game<'_>,
    batch: &mut HudBatch,
) {
    if game.mod_screen() != Some(t3craft_engine::ID) {
        return;
    }
    let mouse = game.cursor();
    let rect = [0.0, 0.0, batch.width, batch.height];
    let commands = t3craft_engine::screen_commands(
        &|text| game.native_text_width(batch, text),
        rect,
        mouse,
    );
    for command in commands {
        match command {
            t3craft_engine::DrawCmd::Fill { rect, argb } => {
                game.native_fill(batch, rect, argb);
            }
            t3craft_engine::DrawCmd::Text { at, text, rgb, shadow } => {
                game.native_text(batch, &text, at, rgb, shadow);
            }
        }
    }
}
