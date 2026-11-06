//! Painted buttons, chips and markers with vector icons, sized so they line up with text:
//! icon buttons, ghost and filled buttons, dropdown chips, kind badges and status dots.

use eframe::egui::{
    Color32, CursorIcon, Id, Popup, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui,
    WidgetInfo, pos2, vec2,
};

use crate::icons::{self, Icon};
use crate::theme::{self, BORDER, BORDER_STRONG, CARD, DIM, FIELD, HOVER, RADIUS, TEXT, TEXT_DIM};

/// Height of chips and ghost buttons.
const CHIP_H: f32 = 24.0;

/// 0 → 1 while `response` is hovered.
fn hover(ui: &Ui, response: &Response) -> f32 {
    ui.ctx()
        .animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.1)
}

/// An icon `size` points wide, for use beside text.
pub fn icon(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    icons::paint(ui.painter(), rect, icon, color);
    response
}

/// A small borderless icon button with a tooltip; it lights up on hover.
pub fn icon_button(ui: &mut Ui, icon: Icon, color: Color32, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    let t = hover(ui, &response);
    if t > 0.0 {
        ui.painter()
            .rect_filled(rect.expand(2.0), RADIUS, HOVER.gamma_multiply(t));
    }
    let color = if enabled {
        theme::lerp(color, TEXT, t)
    } else {
        color.gamma_multiply(0.4)
    };
    icons::paint(ui.painter(), rect.shrink(3.0), icon, color);
    response
        .on_hover_text(label)
        .on_hover_cursor(CursorIcon::PointingHand)
}

/// An outlined uppercase button with an optional leading icon, such as `+ ADD TASK`.
pub fn ghost_button(ui: &mut Ui, leading: Option<Icon>, text: &str) -> Response {
    let text = text.to_uppercase();
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), theme::font(theme::SMALL), TEXT));
    let icon_w = if leading.is_some() { 16.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 20.0, CHIP_H);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &text));
    let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response));
    let t = if open { 1.0 } else { hover(ui, &response) };
    let p = ui.painter();
    p.rect(
        rect,
        RADIUS,
        theme::lerp(CARD, HOVER, t),
        Stroke::new(1.0, theme::lerp(BORDER_STRONG, DIM, t)),
        StrokeKind::Inside,
    );
    let color = theme::lerp(TEXT_DIM, TEXT, t);
    let mut x = rect.left() + 10.0;
    if let Some(icon) = leading {
        let at = Rect::from_center_size(pos2(x + 5.0, rect.center().y), vec2(11.0, 11.0));
        icons::paint(p, at, icon, color);
        x += icon_w;
    }
    p.galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A filled button in `fill` with dark text, such as Run job; dimmed when disabled.
pub fn filled_button(ui: &mut Ui, icon: Icon, text: &str, fill: Color32) -> Response {
    let galley =
        ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), theme::font(theme::BODY), TEXT));
    let size = vec2(galley.size().x + 16.0 + 24.0, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, text));
    let t = hover(ui, &response);
    let fill = match enabled {
        true => theme::lerp(fill, Color32::WHITE, 0.12 * t),
        false => fill.gamma_multiply(0.35),
    };
    let ink = theme::BG;
    let p = ui.painter();
    p.rect_filled(rect, RADIUS, fill);
    let at = Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(11.0, 11.0));
    icons::paint(p, at, icon, ink);
    p.galley(
        pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    match enabled {
        true => response.on_hover_cursor(CursorIcon::PointingHand),
        false => response,
    }
}

/// A clickable dropdown chip, `[badge label ⌄]`; open a menu on it with [`Popup::menu`].
pub fn chip(ui: &mut Ui, id: Id, leading: Option<(Icon, Color32)>, text: &str) -> Response {
    let galley =
        ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), theme::font(theme::BODY), TEXT));
    let icon_w = if leading.is_some() { 22.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 30.0, CHIP_H);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, text));
    let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response));
    let t = if open { 1.0 } else { hover(ui, &response) };
    let p = ui.painter();
    p.rect(
        rect,
        RADIUS,
        theme::lerp(FIELD, HOVER, t),
        Stroke::new(1.0, theme::lerp(BORDER, BORDER_STRONG, t)),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 5.0;
    if let Some((icon, accent)) = leading {
        let at = Rect::from_min_size(pos2(x, rect.center().y - 8.0), vec2(16.0, 16.0));
        paint_kind(ui, at, icon, accent);
        x += icon_w;
    }
    p.galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT,
    );
    let at = Rect::from_center_size(pos2(rect.right() - 11.0, rect.center().y), vec2(9.0, 9.0));
    icons::paint(p, at, Icon::Chevron, theme::lerp(DIM, TEXT_DIM, t));
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A colored square with a kind's icon, `size` points wide.
pub fn kind_badge(ui: &mut Ui, icon: Icon, accent: Color32, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_kind(ui, rect, icon, accent);
    response
}

fn paint_kind(ui: &Ui, rect: Rect, icon: Icon, accent: Color32) {
    let p = ui.painter();
    p.rect(
        rect,
        RADIUS,
        theme::tint(accent, 0.2),
        Stroke::new(1.0, theme::tint(accent, 0.45)),
        StrokeKind::Inside,
    );
    icons::paint(p, rect.shrink(rect.width() * 0.2), icon, accent);
}

/// A status dot: filled when `on`, a ring otherwise.
pub fn dot(ui: &mut Ui, color: Color32, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(8.0, 12.0), Sense::hover());
    if on {
        ui.painter().circle_filled(rect.center(), 3.5, color);
    } else {
        ui.painter()
            .circle_stroke(rect.center(), 3.0, Stroke::new(1.2, color));
    }
    response
}

/// A `⋯` button that opens a menu of `add_contents`.
pub fn more_menu(ui: &mut Ui, label: &str, add_contents: impl FnOnce(&mut Ui)) {
    let response = icon_button(ui, Icon::More, DIM, label);
    Popup::menu(&response).show(add_contents);
}
