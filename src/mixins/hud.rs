// The T3 Code HUD: the corner box of live threads, and the toast card for
// the newest transition while it is fresh.
fn t3craft_hud(
    game: &Game<'_>,
    batch: &mut HudBatch,
    corners: &mut Corners,
) {
    let Some(model) = t3craft_engine::hud_model(&|text| game.native_text_width(batch, text)) else {
        return;
    };
    let width_of = |rows: &[(String, u32)]| {
        rows.iter()
            .map(|(text, _)| game.native_text_width(batch, text))
            .fold(0.0, f32::max)
            + 8.0
    };

    // Title plus one line a thread. Measure everything before drawing: the
    // measurer borrows the batch read-only, drawing needs it mutably.
    let mut rows: Vec<(String, u32)> = Vec::with_capacity(model.rows.len() + 1);
    rows.push((t3craft_engine::TITLE.to_owned(), t3craft_engine::TEXT_PRIMARY));
    rows.extend(model.rows.iter().cloned());
    let w = width_of(&rows) * model.scale;
    let h = (rows.len() as f32 * t3craft_engine::LINE_HEIGHT + 4.0) * model.scale;
    let toast = model.toast.as_ref().map(|(title, line, color)| {
        let tw = width_of(&[(title.clone(), 0), (line.clone(), 0)]) + 8.0;
        (title, line, color, tw)
    });

    let corner = match model.anchor {
        t3craft_engine::HudAnchor::TopLeft => horizon_client::extras::Corner::TopLeft,
        t3craft_engine::HudAnchor::TopRight => horizon_client::extras::Corner::TopRight,
        t3craft_engine::HudAnchor::BottomLeft => horizon_client::extras::Corner::BottomLeft,
        t3craft_engine::HudAnchor::BottomRight => horizon_client::extras::Corner::BottomRight,
    };
    if !game.native_corner_shown(corner) {
        return;
    }

    let (x, y) = corners.place(corner, w, h);
    game.native_fill(batch, [x, y, w, h], t3craft_engine::PANEL_BG);
    let mut ty = y + 2.0 * model.scale;
    for (text, color) in &rows {
        let tx = x + 4.0 * model.scale;
        game.native_text(batch, text, [tx, ty], *color | 0xFF00_0000, true);
        ty += t3craft_engine::LINE_HEIGHT * model.scale;
    }

    // The toast card, while a transition is fresh: under the box for top
    // anchors, above it for bottom ones so it stays on screen.
    if let Some((title, line, color, tw)) = toast {
        let ty0 = if matches!(
            model.anchor,
            t3craft_engine::HudAnchor::BottomLeft | t3craft_engine::HudAnchor::BottomRight
        ) {
            y - 27.0
        } else {
            y + h + 3.0
        };
        game.native_fill(batch, [x, ty0, tw, 24.0], 0xE0_10_10_14);
        game.native_text(batch, title, [x + 4.0, ty0 + 4.0], *color | 0xFF00_0000, true);
        game.native_text(
            batch,
            line,
            [x + 4.0, ty0 + 13.0],
            t3craft_engine::TEXT_PRIMARY | 0xFF00_0000,
            true,
        );
    }
}
