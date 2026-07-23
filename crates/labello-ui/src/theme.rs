use std::sync::Arc;

use egui::{
    Color32, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Frame,
    Margin, Shadow, Stroke, Style, TextStyle, Vec2, Visuals,
    style::{ScrollAnimation, ScrollStyle, WidgetVisuals},
};

pub const APP_BG: Color32 = Color32::from_rgb(9, 14, 24);
pub const PANEL: Color32 = Color32::from_rgb(15, 23, 42);
pub const SURFACE: Color32 = Color32::from_rgb(21, 32, 52);
pub const SURFACE_ELEVATED: Color32 = Color32::from_rgb(25, 36, 58);
pub const INPUT_BG: Color32 = Color32::from_rgb(12, 19, 32);
pub const BORDER: Color32 = Color32::from_rgb(30, 41, 59);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(51, 65, 85);
pub const FOCUS_RING: Color32 = Color32::from_rgb(96, 165, 250);
pub const TEXT: Color32 = Color32::from_rgb(226, 232, 240);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(148, 163, 184);
pub const TEXT_DISABLED: Color32 = Color32::from_rgb(100, 116, 139);
pub const ACCENT: Color32 = Color32::from_rgb(45, 212, 191);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(94, 234, 212);
pub const ACCENT_PRESSED: Color32 = Color32::from_rgb(20, 184, 166);
pub const SUCCESS: Color32 = Color32::from_rgb(52, 211, 153);
pub const WARNING: Color32 = Color32::from_rgb(251, 191, 36);
pub const DANGER: Color32 = Color32::from_rgb(248, 113, 113);
pub const INFO: Color32 = FOCUS_RING;

pub const ANNOTATION: Color32 = ACCENT_HOVER;
pub const SELECTION: Color32 = WARNING;
pub const DRAFT: Color32 = INFO;
pub const PRELABEL: Color32 = Color32::from_rgb(192, 132, 252);
pub const CANVAS_GRID: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 18);

pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_5: f32 = 24.0;
pub const SPACE_6: f32 = 32.0;

pub const CONTROL_RADIUS: u8 = 8;
pub const INSET_RADIUS: u8 = 10;
pub const SURFACE_RADIUS: u8 = 12;
pub const BADGE_RADIUS: u8 = u8::MAX;

pub const PAGE_TITLE_SIZE: f32 = 28.0;
pub const SECTION_HEADING_SIZE: f32 = 21.0;
pub const BODY_SIZE: f32 = 15.0;
pub const SUPPORTING_SIZE: f32 = 12.0;
pub const METRIC_SIZE: f32 = 30.0;
pub const MONOSPACE_SIZE: f32 = 13.0;

// Compatibility aliases while screens move to semantic names in later phases.
pub const BG: Color32 = APP_BG;
pub const PANEL_ALT: Color32 = SURFACE;
pub const CARD: Color32 = SURFACE_ELEVATED;
pub const MUTED: Color32 = TEXT_MUTED;
pub const TEAL: Color32 = ACCENT;
pub const BLUE: Color32 = INFO;
pub const AMBER: Color32 = WARNING;
pub const RED: Color32 = DANGER;

const INTER_REGULAR: &str = "Inter Regular";
const INTER_SEMIBOLD: &str = "Inter SemiBold";

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    ctx.set_global_style(app_style());
}

pub(crate) fn apply_fallback(ctx: &egui::Context) -> bool {
    let fonts_ready = ctx.fonts(|fonts| {
        fonts
            .definitions()
            .families
            .contains_key(&semibold_family())
    });
    if fonts_ready {
        ctx.set_global_style(app_style());
    } else {
        ctx.set_fonts(font_definitions());
        ctx.request_discard("install Labello fonts before layout");
    }
    fonts_ready
}

fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        INTER_REGULAR.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../../assets/fonts/inter/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        INTER_SEMIBOLD.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../../assets/fonts/inter/Inter-SemiBold.ttf"
        ))),
    );

    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, INTER_REGULAR.to_owned());
    let semibold = std::iter::once(INTER_SEMIBOLD.to_owned())
        .chain(proportional.iter().cloned())
        .collect();
    fonts.families.insert(semibold_family(), semibold);
    fonts
}

fn semibold_family() -> FontFamily {
    FontFamily::Name(INTER_SEMIBOLD.into())
}

fn app_style() -> Style {
    let mut style = Style {
        visuals: Visuals::dark(),
        ..Style::default()
    };
    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(SUPPORTING_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(BODY_SIZE, FontFamily::Proportional),
        ),
        (TextStyle::Button, FontId::new(BODY_SIZE, semibold_family())),
        (
            TextStyle::Heading,
            FontId::new(SECTION_HEADING_SIZE, semibold_family()),
        ),
        (
            TextStyle::Monospace,
            FontId::new(MONOSPACE_SIZE, FontFamily::Monospace),
        ),
    ]
    .into();

    let widget = |fill, weak_fill, border, foreground| WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: weak_fill,
        bg_stroke: border,
        corner_radius: CornerRadius::same(CONTROL_RADIUS),
        fg_stroke: Stroke::new(1.0, foreground),
        expansion: 0.0,
    };
    style.visuals.widgets.noninteractive = widget(PANEL, PANEL, Stroke::new(1.0, BORDER), TEXT);
    style.visuals.widgets.inactive =
        widget(INPUT_BG, SURFACE_ELEVATED, Stroke::new(1.0, BORDER), TEXT);
    style.visuals.widgets.hovered = widget(
        Color32::from_rgb(32, 48, 76),
        Color32::from_rgb(32, 48, 76),
        Stroke::new(1.0, BORDER_STRONG),
        TEXT,
    );
    style.visuals.widgets.active = widget(
        Color32::from_rgb(38, 58, 92),
        Color32::from_rgb(38, 58, 92),
        Stroke::new(1.5, FOCUS_RING),
        TEXT,
    );
    style.visuals.widgets.open = widget(
        Color32::from_rgb(32, 48, 76),
        Color32::from_rgb(32, 48, 76),
        Stroke::new(1.5, ACCENT),
        TEXT,
    );

    style.visuals.override_text_color = None;
    style.visuals.weak_text_color = Some(TEXT_MUTED);
    style.visuals.selection.bg_fill = Color32::from_rgb(17, 94, 89);
    style.visuals.selection.stroke = Stroke::new(1.5, TEXT);
    style.visuals.ime_composition.active_underline_stroke = Stroke::new(2.0, FOCUS_RING);
    style.visuals.ime_composition.inactive_underline_stroke = Stroke::new(1.0, BORDER_STRONG);
    style.visuals.hyperlink_color = INFO;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.extreme_bg_color = INPUT_BG;
    style.visuals.text_edit_bg_color = Some(INPUT_BG);
    style.visuals.code_bg_color = INPUT_BG;
    style.visuals.warn_fg_color = WARNING;
    style.visuals.error_fg_color = DANGER;
    style.visuals.window_corner_radius = CornerRadius::same(SURFACE_RADIUS);
    style.visuals.window_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    style.visuals.window_fill = SURFACE_ELEVATED;
    style.visuals.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    style.visuals.menu_corner_radius = CornerRadius::same(INSET_RADIUS);
    style.visuals.panel_fill = APP_BG;
    style.visuals.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    style.visuals.text_cursor.stroke = Stroke::new(2.0, FOCUS_RING);
    style.visuals.button_frame = true;
    style.visuals.interact_cursor = Some(CursorIcon::PointingHand);
    style.visuals.image_loading_spinners = true;
    style.visuals.disabled_alpha = 0.55;

    style.spacing.item_spacing = Vec2::new(10.0, SPACE_2);
    style.spacing.window_margin = Margin::same(SPACE_4 as i8);
    style.spacing.menu_margin = Margin::same(SPACE_2 as i8);
    style.spacing.button_padding = Vec2::new(SPACE_3, SPACE_2);
    style.spacing.interact_size = Vec2::splat(44.0);
    style.spacing.icon_width = 16.0;
    style.spacing.icon_width_inner = 10.0;
    style.spacing.icon_spacing = SPACE_2;
    style.spacing.scroll = ScrollStyle {
        bar_width: 8.0,
        handle_min_length: 32.0,
        floating_width: 3.0,
        floating_allocated_width: 4.0,
        dormant_handle_opacity: 0.35,
        active_handle_opacity: 0.65,
        interact_handle_opacity: 1.0,
        active_background_opacity: 0.2,
        interact_background_opacity: 0.35,
        ..ScrollStyle::floating()
    };
    style.animation_time = 0.12;
    style.scroll_animation = ScrollAnimation::duration(0.12);
    style
}

pub fn top_bar_frame() -> Frame {
    Frame::new()
        .fill(APP_BG)
        .inner_margin(Margin::symmetric(14, SPACE_2 as i8))
        .stroke(Stroke::new(1.0, BORDER))
}

pub fn side_frame() -> Frame {
    Frame::new()
        .fill(PANEL)
        .inner_margin(Margin::same(SPACE_4 as i8))
        .stroke(Stroke::new(1.0, BORDER))
}

pub fn central_frame() -> Frame {
    Frame::new()
        .fill(APP_BG)
        .inner_margin(Margin::same(SPACE_5 as i8))
}

pub fn card_frame() -> Frame {
    Frame::new()
        .fill(SURFACE_ELEVATED)
        .corner_radius(CornerRadius::same(SURFACE_RADIUS))
        .inner_margin(Margin::symmetric(SPACE_3 as i8, 10))
        .stroke(Stroke::new(1.0, BORDER_STRONG))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_installs_inter_and_complete_widget_states() {
        let fonts = font_definitions();
        assert_eq!(fonts.families[&FontFamily::Proportional][0], INTER_REGULAR);
        assert_eq!(fonts.families[&semibold_family()][0], INTER_SEMIBOLD);

        let style = app_style();
        assert_eq!(style.text_styles[&TextStyle::Body].size, BODY_SIZE);
        assert_eq!(
            style.text_styles[&TextStyle::Heading].family,
            semibold_family()
        );
        assert_eq!(
            style.visuals.widgets.inactive.corner_radius,
            CornerRadius::same(CONTROL_RADIUS)
        );
        assert_eq!(
            style.visuals.widgets.active.bg_stroke,
            Stroke::new(1.5, FOCUS_RING)
        );
        assert_eq!(
            style.visuals.widgets.open.bg_stroke,
            Stroke::new(1.5, ACCENT)
        );
        assert_eq!(style.visuals.text_edit_bg_color(), INPUT_BG);
        assert_eq!(style.visuals.disabled_alpha, 0.55);
        assert_eq!(style.spacing.interact_size, Vec2::splat(44.0));
    }
}
