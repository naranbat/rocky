//! Colors, fonts and spacing: dark, flat, monospace, with a yellow accent.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Sense, Stroke, StrokeKind,
    TextStyle, Visuals, pos2, vec2,
};

use crate::icons::{self, Icon};

pub const BG: Color32 = Color32::from_rgb(0x15, 0x15, 0x15);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x19, 0x19, 0x19);
pub const CARD: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
pub const FIELD: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
pub const HOVER: Color32 = Color32::from_rgb(0x31, 0x31, 0x31);
pub const BORDER: Color32 = Color32::from_rgb(0x2d, 0x2d, 0x2d);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x3d, 0x3d, 0x3d);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xe6);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xa8, 0xa8, 0xa8);
pub const DIM: Color32 = Color32::from_rgb(0x7a, 0x7a, 0x7a);
pub const FAINT: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
pub const ACCENT: Color32 = Color32::from_rgb(0xe8, 0xc8, 0x4f);
pub const ACCENT_BG: Color32 = Color32::from_rgb(0x2a, 0x27, 0x19);
pub const OK: Color32 = Color32::from_rgb(0x4c, 0xc2, 0x86);
pub const ERROR: Color32 = Color32::from_rgb(0xe5, 0x5a, 0x52);
pub const ERROR_BG: Color32 = Color32::from_rgb(0x2a, 0x1a, 0x19);
pub const NUMBER: Color32 = Color32::from_rgb(0xe8, 0x84, 0x4a);

pub const CMD: Color32 = Color32::from_rgb(0x4c, 0xc2, 0x86);
pub const FOR_EACH: Color32 = Color32::from_rgb(0x63, 0x8f, 0xf0);
pub const WHILE: Color32 = Color32::from_rgb(0x45, 0xbd, 0xcc);
pub const TRIGGER: Color32 = Color32::from_rgb(0xe8, 0x84, 0x4a);
pub const FILE: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);

/// Corner radius of widgets; cards round a little more.
pub const RADIUS: u8 = 3;
pub const BODY: f32 = 13.0;
pub const SMALL: f32 = 11.0;

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// `accent` mixed into the card color: `strength` 0 is the card, 1 the accent.
pub fn tint(accent: Color32, strength: f32) -> Color32 {
    lerp(CARD, accent, strength)
}

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}

/// Applies the dark theme, with every text style in the monospace font.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, font(16.0)),
            (TextStyle::Body, font(BODY)),
            (TextStyle::Button, font(BODY)),
            (TextStyle::Monospace, font(BODY)),
            (TextStyle::Small, font(SMALL)),
        ]
        .into();
        style.spacing.item_spacing = vec2(6.0, 6.0);
        style.spacing.button_padding = vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        style.spacing.menu_margin = Margin::same(4);
        style.interaction.selectable_labels = false;

        let v: &mut Visuals = &mut style.visuals;
        *v = Visuals::dark();
        v.override_text_color = Some(TEXT);
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
        v.window_corner_radius = CornerRadius::same(4);
        v.menu_corner_radius = CornerRadius::same(4);
        v.extreme_bg_color = BG;
        v.text_edit_bg_color = Some(FIELD);
        v.faint_bg_color = CARD;
        v.code_bg_color = BG;
        v.hyperlink_color = FOR_EACH;
        v.selection.bg_fill = Color32::from_rgb(0x2f, 0x46, 0x6e);
        v.selection.stroke = Stroke::new(1.0, TEXT);
        v.text_cursor.stroke = Stroke::new(1.5, ACCENT);
        v.error_fg_color = ERROR;
        v.warn_fg_color = ACCENT;
        v.popup_shadow = egui::epaint::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(140),
        };

        let w = &mut v.widgets;
        for (state, fill, stroke, text) in [
            (&mut w.noninteractive, CARD, BORDER, TEXT_DIM),
            (&mut w.inactive, FIELD, BORDER, TEXT),
            (&mut w.hovered, HOVER, BORDER_STRONG, TEXT),
            (&mut w.active, HOVER, DIM, TEXT),
            (&mut w.open, HOVER, BORDER_STRONG, TEXT),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0, stroke);
            state.fg_stroke = Stroke::new(1.0, text);
            state.corner_radius = CornerRadius::same(RADIUS);
            state.expansion = 0.0;
        }
    });
}

/// A bordered card, as used for tasks and form sections.
pub fn card() -> Frame {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(4)
        .inner_margin(Margin::same(12))
}

/// A flat inset box inside a card.
pub fn inset() -> Frame {
    Frame::new()
        .fill(BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(8))
}

/// A small uppercase section heading with an optional dim note after it.
pub fn section(ui: &mut egui::Ui, title: &str, note: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .strong()
                .size(SMALL)
                .color(TEXT_DIM),
        );
        if !note.is_empty() {
            ui.label(
                egui::RichText::new(format!("· {note}"))
                    .color(DIM)
                    .size(SMALL),
            );
        }
    });
}

/// A small colored label with a tinted background and an optional leading icon, such as
/// `VALID` or `WHEN`. Painted at a fixed height so it lines up with the text beside it.
pub fn badge(ui: &mut egui::Ui, icon: Option<Icon>, text: &str, color: Color32) -> egui::Response {
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font(SMALL), color));
    let icon_w = if icon.is_some() { 14.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 14.0, 20.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect(
        rect,
        RADIUS,
        tint(color, 0.14),
        Stroke::new(1.0, tint(color, 0.45)),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 7.0;
    if let Some(icon) = icon {
        let at = egui::Rect::from_center_size(pos2(x + 4.5, rect.center().y), vec2(10.0, 10.0));
        icons::paint(p, at, icon, color);
        x += icon_w;
    }
    p.galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response
}

/// Rocky's mark: a mountain on a yellow square `size` points wide.
pub fn logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().rect_filled(rect, size * 0.16, ACCENT);
    icons::paint(ui.painter(), rect.shrink(size * 0.2), Icon::Mountain, BG);
}

/// Dim helper text.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(DIM).size(12.0));
}

/// A validation problem, in red, inside the card or job it is about.
pub fn problem(ui: &mut egui::Ui, message: &str) {
    Frame::new()
        .fill(ERROR_BG)
        .stroke(Stroke::new(1.0, tint(ERROR, 0.5)))
        .corner_radius(RADIUS)
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (rect, _) = ui.allocate_exact_size(vec2(12.0, 15.0), Sense::hover());
                icons::paint(ui.painter(), rect, Icon::Warning, ERROR);
                ui.label(egui::RichText::new(message).color(ERROR).size(12.0));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character in the builder's strings is in the bundled fonts, so none is drawn as
    /// an empty box. Icons the fonts lack are painted with `icons` instead.
    #[test]
    fn strings_use_only_bundled_glyphs() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let source = std::fs::read_to_string(&path).unwrap();
            let code = source
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"));
            for c in code.flat_map(str::chars).filter(|c| !c.is_ascii()) {
                if !ctx.fonts_mut(|fonts| fonts.has_glyph(&font(BODY), c)) {
                    missing.push((path.file_name().unwrap().to_owned(), c));
                }
            }
        }
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }
}
