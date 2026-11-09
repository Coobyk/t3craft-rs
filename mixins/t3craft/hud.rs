// The T3 Code HUD: the corner box of live threads, and the toast card for
// the newest transition while it is fresh.
fn t3craft_hud(
    game: &mut crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    use horizon_client::extras::Corner;
    // Turned off in Options > Mods: nothing until the next build leaves it out.
    if !game.native_on(t3craft_engine::ID) {
        return;
    }
    let game_ref = &*game;
    let Some(model) = t3craft_engine::hud_model(&|text| game_ref.native_text_width(text)) else {
        return;
    };
    let width_of = |rows: &[(String, u32)]| {
        rows.iter()
            .map(|(text, _)| game_ref.native_text_width(text))
            .fold(0.0, f32::max)
            + 8.0
    };

    let corner = match model.anchor {
        t3craft_engine::HudAnchor::TopLeft => Corner::TopLeft,
        t3craft_engine::HudAnchor::TopRight => Corner::TopRight,
        t3craft_engine::HudAnchor::BottomLeft => Corner::BottomLeft,
        t3craft_engine::HudAnchor::BottomRight => Corner::BottomRight,
    };
    if !game.native_corner_shown(corner) {
        return;
    }

    // Title plus one line a thread.
    let mut rows: Vec<(String, u32)> = Vec::with_capacity(model.rows.len() + 1);
    rows.push(("T3 Code".to_owned(), 0xF2_F2_F2));
    rows.extend(model.rows.iter().cloned());
    let w = width_of(&rows) * model.scale;
    let h = (rows.len() as f32 * 10.0 + 4.0) * model.scale;
    let (x, y) = corners.place(corner, w, h);
    game.native_fill(batch, [x, y, w, h], 0xB0_10_10_14);
    let mut ty = y + 2.0 * model.scale;
    for (text, color) in &rows {
        let tx = x + 4.0 * model.scale;
        if model.scale == 1.0 {
            game.native_text(batch, text, [tx, ty], *color, true);
        } else {
            // A scaled HUD draws its lines at the scale: approximate with the
            // plain drawer when the scale differs.
            game.native_text(batch, text, [tx, ty], *color, true);
        }
        ty += 10.0 * model.scale;
    }

    // The toast card, while a transition is fresh: under the box for top
    // anchors, above it for bottom ones so it stays on screen.
    if let Some((title, line, color)) = &model.toast {
        let tw = width_of(&[(title.clone(), 0), (line.clone(), 0)]) + 8.0;
        let ty0 = if matches!(
            model.anchor,
            t3craft_engine::HudAnchor::BottomLeft | t3craft_engine::HudAnchor::BottomRight
        ) {
            y - 27.0
        } else {
            y + h + 3.0
        };
        game.native_fill(batch, [x, ty0, tw, 24.0], 0xE0_10_10_14);
        game.native_text(batch, title, [x + 4.0, ty0 + 4.0], *color, true);
        game.native_text(batch, line, [x + 4.0, ty0 + 13.0], 0xF2_F2_F2, true);
    }
}
