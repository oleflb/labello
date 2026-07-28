use std::sync::Arc;

use egui::{
    Align, Button, Color32, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily,
    FontId, Frame, Id, Margin, Modal, Response, RichText, Shadow, Stroke, Style, TextEdit,
    TextStyle, Ui, Vec2, Visuals,
    style::{ScrollAnimation, ScrollStyle, WidgetVisuals},
};

pub const APP_BG: Color32 = Color32::from_rgb(9, 14, 24);
pub const PANEL: Color32 = Color32::from_rgb(15, 23, 42);
pub const SURFACE: Color32 = Color32::from_rgb(21, 32, 52);
pub const SURFACE_ELEVATED: Color32 = Color32::from_rgb(25, 36, 58);
pub const BUTTON_BG: Color32 = Color32::from_rgb(29, 43, 68);
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
pub const COMPACT_TEXT_FIELD_HEIGHT: f32 = 28.0;
pub const MENU_WIDTH: f32 = 220.0;

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

#[derive(Clone, Copy)]
pub enum Intent {
    Neutral,
    Accent,
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy)]
enum ButtonKind {
    Primary,
    Quiet,
    Danger,
}

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
    style.visuals.widgets.inactive = widget(INPUT_BG, BUTTON_BG, Stroke::new(1.0, BORDER), TEXT);
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
        bar_width: 6.0,
        handle_min_length: 32.0,
        floating_width: 2.0,
        floating_allocated_width: 0.0,
        dormant_handle_opacity: 0.0,
        active_handle_opacity: 0.45,
        interact_handle_opacity: 0.75,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
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

pub fn inset_frame() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .corner_radius(CornerRadius::same(INSET_RADIUS))
        .inner_margin(Margin::symmetric(SPACE_3 as i8, 6))
        .stroke(Stroke::new(1.0, BORDER))
}

pub fn selected_card_frame(selected: bool) -> Frame {
    if selected {
        card_frame()
            .fill(Color32::from_rgb(32, 48, 76))
            .stroke(Stroke::new(1.5, ACCENT.gamma_multiply(0.75)))
    } else {
        card_frame()
    }
}

pub fn prelabel_card_frame(selected: bool) -> Frame {
    let alpha = if selected { 48 } else { 24 };
    card_frame()
        .fill(Color32::from_rgba_unmultiplied(
            PRELABEL.r(),
            PRELABEL.g(),
            PRELABEL.b(),
            alpha,
        ))
        .stroke(Stroke::new(if selected { 2.0 } else { 1.0 }, PRELABEL))
}

pub fn modal(ctx: &egui::Context, id: Id) -> Modal {
    Modal::new(id).frame(Frame::window(&ctx.style_of(ctx.theme())))
}

pub fn primary_button(ui: &mut Ui, enabled: bool, button: Button<'_>) -> Response {
    semantic_button(ui, enabled, button, ButtonKind::Primary, None)
}

pub fn primary_button_sized(ui: &mut Ui, size: Vec2, button: Button<'_>) -> Response {
    semantic_button(ui, true, button, ButtonKind::Primary, Some(size))
}

pub fn quiet_button(ui: &mut Ui, enabled: bool, button: Button<'_>) -> Response {
    semantic_button(ui, enabled, button, ButtonKind::Quiet, None)
}

pub fn danger_button(ui: &mut Ui, enabled: bool, button: Button<'_>) -> Response {
    semantic_button(ui, enabled, button, ButtonKind::Danger, None)
}

fn semantic_button(
    ui: &mut Ui,
    enabled: bool,
    button: Button<'_>,
    kind: ButtonKind,
    size: Option<Vec2>,
) -> Response {
    let original_style = ui.style().clone();
    let (inactive, hovered, active, foreground, border) = match kind {
        ButtonKind::Primary => (ACCENT, ACCENT_HOVER, ACCENT_PRESSED, APP_BG, ACCENT_PRESSED),
        ButtonKind::Quiet => (
            BUTTON_BG,
            Color32::from_rgb(35, 51, 80),
            SURFACE_ELEVATED,
            TEXT,
            BORDER,
        ),
        ButtonKind::Danger => (
            DANGER.gamma_multiply(0.16),
            DANGER.gamma_multiply(0.24),
            DANGER.gamma_multiply(0.32),
            DANGER,
            DANGER.gamma_multiply(0.55),
        ),
    };
    {
        let widgets = &mut ui.style_mut().visuals.widgets;
        for (visuals, fill, stroke) in [
            (&mut widgets.inactive, inactive, Stroke::new(1.0, border)),
            (&mut widgets.hovered, hovered, Stroke::new(1.0, border)),
            (&mut widgets.active, active, Stroke::new(1.5, FOCUS_RING)),
            (&mut widgets.open, hovered, Stroke::new(1.5, FOCUS_RING)),
        ] {
            visuals.bg_fill = fill;
            visuals.weak_bg_fill = fill;
            visuals.bg_stroke = stroke;
            visuals.fg_stroke = Stroke::new(1.0, foreground);
        }
    }
    let response = if let Some(size) = size {
        ui.add_sized(size, button)
    } else {
        ui.add_enabled(enabled, button)
    };
    ui.set_style(original_style);
    response
}

impl Intent {
    pub fn color(self) -> Color32 {
        match self {
            Self::Neutral => TEXT_MUTED,
            Self::Accent => ACCENT,
            Self::Info => INFO,
            Self::Success => SUCCESS,
            Self::Warning => WARNING,
            Self::Error => DANGER,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Neutral => "Note",
            Self::Accent => "Update",
            Self::Info => "Info",
            Self::Success => "Success",
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }
}

pub fn badge(ui: &mut Ui, text: &str, intent: Intent) -> Response {
    badge_inner(ui, text, intent, None)
}

pub fn bounded_badge(ui: &mut Ui, text: &str, intent: Intent, width: f32) -> Response {
    badge_inner(ui, text, intent, Some(width))
}

fn badge_inner(ui: &mut Ui, text: &str, intent: Intent, width: Option<f32>) -> Response {
    let color = intent.color();
    let frame = Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            36,
        ))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.55)))
        .corner_radius(CornerRadius::same(BADGE_RADIUS))
        .inner_margin(Margin::symmetric(9, SPACE_1 as i8));
    let response = if let Some(width) = width {
        frame
            .show(ui, |ui| {
                ui.add_sized(
                    [width, 24.0],
                    egui::Label::new(RichText::new(text).color(color).strong()).truncate(),
                )
            })
            .inner
    } else {
        ui.add(
            egui::AtomLayout::new(RichText::new(text).color(color).strong())
                .frame(frame)
                .wrap_mode(egui::TextWrapMode::Extend),
        )
    };
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.to_owned()));
    response
}

pub fn metric(ui: &mut Ui, label: &str, value: impl Into<String>) {
    metric_inner(ui, label, value.into(), false);
}

pub fn compact_metric(ui: &mut Ui, label: &str, value: impl Into<String>) {
    metric_inner(ui, label, value.into(), true);
}

fn metric_inner(ui: &mut Ui, label: &str, value: String, compact: bool) {
    let response = inset_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        if compact {
            let label_text = RichText::new(label).color(TEXT_MUTED);
            let value_text = RichText::new(value)
                .size(SECTION_HEADING_SIZE)
                .strong()
                .color(TEXT);
            let label_width = egui::WidgetText::from(label_text.clone())
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    TextStyle::Body,
                )
                .size()
                .x;
            let value_width = egui::WidgetText::from(value_text.clone())
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    TextStyle::Body,
                )
                .size()
                .x;
            if label_width + ui.spacing().item_spacing.x + value_width <= ui.available_width() {
                ui.horizontal(|ui| {
                    ui.label(label_text);
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.label(value_text);
                    });
                });
            } else {
                ui.label(label_text);
                ui.add(egui::Label::new(value_text).wrap().halign(Align::RIGHT));
            }
        } else {
            ui.set_min_height(72.0);
            ui.label(RichText::new(label).color(TEXT_MUTED));
            ui.label(RichText::new(value).size(METRIC_SIZE).strong().color(TEXT));
        }
    });
    response.response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, format!("Metric {label}"))
    });
}

pub fn labeled_text_field(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    field_height: f32,
) -> Response {
    if ui.available_width() < 520.0 {
        ui.vertical(|ui| {
            let label = ui.label(label);
            ui.add_sized(
                [ui.available_width(), field_height],
                singleline_text_edit(value),
            )
            .labelled_by(label.id)
        })
        .inner
    } else {
        ui.horizontal(|ui| {
            let label = ui.add_sized([140.0, 44.0], egui::Label::new(label));
            ui.add_sized(
                [ui.available_width(), field_height],
                singleline_text_edit(value),
            )
            .labelled_by(label.id)
        })
        .inner
    }
}

pub fn singleline_text_edit(value: &mut String) -> TextEdit<'_> {
    TextEdit::singleline(value).vertical_align(Align::Center)
}

pub fn resizable_multiline_text_edit(
    ui: &mut Ui,
    id: Id,
    value: &mut String,
    desired_rows: usize,
    hint_text: Option<&str>,
) -> Response {
    let row_height = ui.text_style_height(&TextStyle::Body);
    let default_height = row_height * desired_rows as f32 + 8.0;
    let width = ui.available_width();
    egui::Resize::default()
        .id(id)
        .default_size(egui::vec2(width, default_height))
        .min_size(egui::vec2(width, default_height))
        .max_size(egui::vec2(width, 400.0))
        .resizable([false, true])
        .with_stroke(false)
        .show(ui, |ui| {
            let mut edit = TextEdit::multiline(value)
                .desired_width(f32::INFINITY)
                .desired_rows(desired_rows);
            if let Some(hint_text) = hint_text {
                edit = edit.hint_text(hint_text);
            }
            ui.add_sized(ui.available_size(), edit)
        })
}

pub fn inline_message(ui: &mut Ui, intent: Intent, message: impl Into<String>) -> Response {
    let color = intent.color();
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            20,
        ))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(10, SPACE_2 as i8))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(intent.label()).strong().color(color));
                ui.label(RichText::new(message.into()).color(TEXT))
            })
            .inner
        })
        .inner
}

pub fn empty_state(
    ui: &mut Ui,
    title: &str,
    explanation: &str,
    action: Option<Button<'_>>,
) -> bool {
    inset_frame()
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(title).strong().color(TEXT));
            ui.label(RichText::new(explanation).color(TEXT_MUTED));
            action.is_some_and(|button| primary_button(ui, true, button).clicked())
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

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
        assert_eq!(style.visuals.widgets.inactive.weak_bg_fill, BUTTON_BG);
        assert_eq!(style.visuals.disabled_alpha, 0.55);
        assert_eq!(style.spacing.interact_size, Vec2::splat(44.0));
    }

    #[test]
    fn components_keep_accessible_labels_states_and_touch_targets() {
        let mut value = String::new();
        let mut compact_value = String::new();
        let harness = Harness::builder()
            .with_size(Vec2::new(320.0, 400.0))
            .build_ui(move |ui| {
                let inactive = ui.visuals().widgets.inactive;
                primary_button(ui, false, Button::new("Primary action"));
                assert_eq!(ui.visuals().widgets.inactive, inactive);
                primary_button_sized(
                    ui,
                    Vec2::new(180.0, 44.0),
                    Button::new("Bounded primary action").truncate(),
                );
                quiet_button(ui, true, Button::new("Quiet action"));
                danger_button(ui, true, Button::new("Danger action"));
                labeled_text_field(ui, "Field label", &mut value, 44.0);
                let compact_label = ui.label("Compact field");
                ui.add_sized(
                    [ui.available_width(), COMPACT_TEXT_FIELD_HEIGHT],
                    singleline_text_edit(&mut compact_value),
                )
                .labelled_by(compact_label.id);
                empty_state(
                    ui,
                    "Nothing here",
                    "Create the first item to continue.",
                    Some(Button::new("Create item")),
                );
            });

        let primary =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, "Primary action");
        let danger = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Danger action");
        let field = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Field label");
        let bounded =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, "Bounded primary action");
        let compact =
            harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Compact field");
        assert!(primary.accesskit_node().is_disabled());
        assert!(!danger.accesskit_node().is_disabled());
        assert!(field.rect().height() >= 44.0);
        assert!((compact.rect().height() - COMPACT_TEXT_FIELD_HEIGHT).abs() <= 1.0);
        assert_eq!(bounded.rect().width(), 180.0);
        assert!(harness.query_by_label("Create item").is_some());
    }

    #[test]
    fn badges_wrap_as_whole_items() {
        let harness = Harness::builder()
            .with_size(Vec2::new(150.0, 120.0))
            .build_ui(|ui| {
                ui.horizontal_wrapped(|ui| {
                    badge(ui, "Annotator", Intent::Info);
                    badge(ui, "Adjudicator", Intent::Info);
                });
            });

        let annotator = harness.get_by_label("Annotator").rect();
        let adjudicator = harness.get_by_label("Adjudicator").rect();
        assert!(adjudicator.top() >= annotator.bottom());
        assert!(annotator.height() <= 32.0);
        assert!(adjudicator.height() <= 32.0);
    }

    #[test]
    fn compact_metrics_wrap_long_values_without_overlapping_labels() {
        let harness = Harness::builder()
            .with_size(Vec2::new(289.0, 120.0))
            .build_ui(|ui| {
                compact_metric(ui, "Review target", "1 of 2 | Skeleton annotated");
            });

        let label = harness.get_by_label("Review target").rect();
        let value = harness.get_by_label("1 of 2 | Skeleton annotated").rect();
        assert!(
            !label.intersects(value),
            "compact metric label and value overlap: label={label:?} value={value:?}"
        );
    }

    #[test]
    fn prelabel_cards_reuse_the_canvas_prelabel_color() {
        let idle = prelabel_card_frame(false);
        let selected = prelabel_card_frame(true);

        assert_eq!(idle.stroke.color, PRELABEL);
        assert_eq!(selected.stroke.color, PRELABEL);
        assert_eq!(idle.fill.a(), 24);
        assert_eq!(selected.fill.a(), 48);
        assert!(selected.fill.a() > idle.fill.a());
        assert!(selected.stroke.width > idle.stroke.width);
    }
}
