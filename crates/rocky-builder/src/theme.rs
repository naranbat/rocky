//! Colors, fonts and spacing from the `ui-design/` mockups: dark, monospace, yellow accent.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Stroke, TextStyle,
};

pub const BG: Color32 = Color32::from_rgb(0x17, 0x18, 0x1b);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x1a, 0x1b, 0x1e);
pub const CARD: Color32 = Color32::from_rgb(0x1e, 0x1f, 0x23);
pub const FIELD: Color32 = Color32::from_rgb(0x25, 0x26, 0x2b);
pub const BORDER: Color32 = Color32::from_rgb(0x2e, 0x30, 0x36);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xe6);
pub const DIM: Color32 = Color32::from_rgb(0x8a, 0x8d, 0x93);
pub const ACCENT: Color32 = Color32::from_rgb(0xf2, 0xc9, 0x4c);
pub const ACCENT_BG: Color32 = Color32::from_rgb(0x2b, 0x26, 0x14);
pub const OK: Color32 = Color32::from_rgb(0x3d, 0xd6, 0x8c);
pub const ERROR: Color32 = Color32::from_rgb(0xf2, 0x55, 0x5a);
pub const ERROR_BG: Color32 = Color32::from_rgb(0x2e, 0x18, 0x1a);
pub const NUMBER: Color32 = Color32::from_rgb(0xf2, 0x9b, 0x5c);

pub const CMD: Color32 = Color32::from_rgb(0x1e, 0xcf, 0xa0);
pub const FOR_EACH: Color32 = Color32::from_rgb(0x9b, 0x7c, 0xf6);
pub const WHILE: Color32 = Color32::from_rgb(0x4c, 0x9e, 0xf5);
pub const TRIGGER: Color32 = Color32::from_rgb(0xf2, 0x5f, 0xa8);
pub const FILE: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xa6);

/// Applies the dark theme, with every text style in the monospace font.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        for (text_style, font) in [
            (TextStyle::Small, 11.0),
            (TextStyle::Body, 13.0),
            (TextStyle::Monospace, 13.0),
            (TextStyle::Button, 13.0),
            (TextStyle::Heading, 18.0),
        ] {
            style
                .text_styles
                .insert(text_style, FontId::new(font, FontFamily::Monospace));
        }
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(TEXT);
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.extreme_bg_color = FIELD;
        v.text_edit_bg_color = Some(FIELD);
        v.code_bg_color = FIELD;
        v.faint_bg_color = CARD;
        v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.error_fg_color = ERROR;
        v.warn_fg_color = ACCENT;
        let radius = CornerRadius::same(6);
        for widget in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            widget.corner_radius = radius;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.inactive.bg_fill = FIELD;
        v.widgets.inactive.weak_bg_fill = FIELD;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.hovered.bg_fill = BORDER;
        v.widgets.hovered.weak_bg_fill = BORDER;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, DIM);
        v.widgets.active.bg_fill = BORDER;
        v.widgets.active.weak_bg_fill = BORDER;
        v.widgets.open.weak_bg_fill = BORDER;
    });
}

/// A bordered card, as used for tasks and form sections.
pub fn card() -> Frame {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(8)
        .inner_margin(Margin::same(12))
}

/// A flat inset box inside a card.
pub fn inset() -> Frame {
    Frame::new()
        .fill(BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(6)
        .inner_margin(Margin::same(8))
}

/// A small uppercase section heading with an optional dim note after it.
pub fn section(ui: &mut egui::Ui, title: &str, note: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).strong().size(12.0));
        if !note.is_empty() {
            ui.label(
                egui::RichText::new(format!("· {note}"))
                    .color(DIM)
                    .size(12.0),
            );
        }
    });
}

/// A small colored label with a tinted background, such as `VALID` or `WHEN`.
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    Frame::new()
        .fill(color.gamma_multiply(0.15))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.5)))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(color).strong().size(11.0));
        })
        .response
}

/// Dim helper text.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(DIM).size(12.0));
}
