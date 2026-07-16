use egui::{Color32, CornerRadius, Frame, Margin, Stroke, Style, Vec2, Visuals};

pub const BG: Color32 = Color32::from_rgb(9, 14, 24);
pub const PANEL: Color32 = Color32::from_rgb(15, 23, 42);
pub const PANEL_ALT: Color32 = Color32::from_rgb(21, 32, 52);
pub const CARD: Color32 = Color32::from_rgb(25, 36, 58);
pub const TEXT: Color32 = Color32::from_rgb(226, 232, 240);
pub const MUTED: Color32 = Color32::from_rgb(148, 163, 184);
pub const TEAL: Color32 = Color32::from_rgb(45, 212, 191);
pub const BLUE: Color32 = Color32::from_rgb(96, 165, 250);
pub const AMBER: Color32 = Color32::from_rgb(251, 191, 36);
pub const RED: Color32 = Color32::from_rgb(248, 113, 113);

pub fn apply(ctx: &egui::Context) {
    let mut style = Style {
        visuals: Visuals::dark(),
        ..Style::default()
    };
    style.visuals.window_fill = PANEL;
    style.visuals.panel_fill = BG;
    style.visuals.faint_bg_color = PANEL_ALT;
    style.visuals.extreme_bg_color = BG;
    style.visuals.widgets.inactive.bg_fill = CARD;
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(32, 48, 76);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(38, 58, 92);
    style.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 8.0);
    style.spacing.interact_size.y = 44.0;
    ctx.set_global_style(style);
}

pub fn top_bar_frame() -> Frame {
    Frame::new()
        .fill(Color32::from_rgb(8, 13, 23))
        .inner_margin(Margin::symmetric(14, 8))
        .stroke(Stroke::new(1.0, Color32::from_rgb(30, 41, 59)))
}

pub fn side_frame() -> Frame {
    Frame::new()
        .fill(PANEL)
        .inner_margin(Margin::symmetric(14, 14))
        .stroke(Stroke::new(1.0, Color32::from_rgb(30, 41, 59)))
}

pub fn central_frame() -> Frame {
    Frame::new()
        .fill(BG)
        .inner_margin(Margin::symmetric(18, 18))
}

pub fn card_frame() -> Frame {
    Frame::new()
        .fill(CARD)
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(12, 10))
        .stroke(Stroke::new(1.0, Color32::from_rgb(39, 53, 82)))
}
