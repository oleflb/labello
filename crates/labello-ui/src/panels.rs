use eframe::egui::{self, RichText};
use labello_domain::{
    AdjudicationDecision, AnnotationGeometry, AnnotationType, KeypointState, ReviewDecision,
};

use crate::{
    app::{
        AppView, Drawer, LabelloApp, LayoutMode, PendingTransition, SaveStatus, Tool,
        annotation_type_label,
    },
    theme,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppBarAction {
    Setup,
    Tutorial,
    Settings,
    SignOut,
}

impl AppBarAction {
    fn label(self) -> &'static str {
        match self {
            Self::Setup => "Setup",
            Self::Tutorial => "Tutorial",
            Self::Settings => "Settings",
            Self::SignOut => "Sign out",
        }
    }

    fn accessible_label(self) -> &'static str {
        match self {
            Self::Setup => "Open setup",
            Self::Tutorial => "Open tutorial",
            Self::Settings => "Open settings",
            Self::SignOut => "Sign out",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Setup => "Open dataset setup.",
            Self::Tutorial => "Show or hide workflow instructions.",
            Self::Settings => "Open keyboard shortcut settings.",
            Self::SignOut => "Sign out of Labello.",
        }
    }
}

include!("panels/app_bar.rs");
include!("panels/workspace_actions.rs");
include!("panels/task_selector.rs");
include!("panels/inspector.rs");
include!("panels/workspace.rs");
include!("panels/overlays.rs");
include!("panels/prelabels.rs");

fn centered_scroll(ui: &mut egui::Ui, max_width: f32, add_contents: impl FnOnce(&mut egui::Ui)) {
    let available_width = (ui.available_width() - ui.spacing().scroll.allocated_width()).max(0.0);
    egui::ScrollArea::vertical().show(ui, |ui| {
        let width = available_width.min(max_width);
        let inset = ((available_width - width) * 0.5).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(inset);
            ui.vertical(|ui| {
                ui.set_width(width);
                add_contents(ui);
            });
        });
    });
}

pub(crate) fn keypoint_placement_mode(
    ui: &mut egui::Ui,
    keypoint_name: &str,
    occluded: &mut bool,
    shortcut: &str,
) {
    ui.label(RichText::new("Placement").color(theme::TEXT_MUTED));
    ui.horizontal_wrapped(|ui| {
        let enabled = ui.is_enabled();
        let visible_selected = !*occluded;
        let visible_label = format!("Place {keypoint_name} as visible");
        let visible = ui
            .add(
                egui::Button::new("Visible")
                    .selected(visible_selected)
                    .min_size(egui::vec2(88.0, 44.0)),
            )
            .on_hover_text("Click the exact keypoint position.");
        visible.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                enabled,
                visible_selected,
                &visible_label,
            )
        });
        if visible.clicked() {
            *occluded = false;
        }

        let occluded_selected = *occluded;
        let occluded_label = format!("Place {keypoint_name} as occluded");
        let occluded_response = ui
            .add(
                egui::Button::new("Occluded")
                    .selected(occluded_selected)
                    .shortcut_text(shortcut)
                    .min_size(egui::vec2(88.0, 44.0)),
            )
            .on_hover_text("Click the estimated keypoint position.");
        occluded_response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                enabled,
                occluded_selected,
                &occluded_label,
            )
        });
        if occluded_response.clicked() {
            *occluded = true;
        }
    });
    ui.small(if *occluded {
        "Occluded: click the estimated position."
    } else {
        "Visible: click the exact position."
    });
}

fn action_label(action: &labello_domain::UserAction) -> &'static str {
    use labello_domain::UserAction;
    match action {
        UserAction::NextImage => "Submit and next",
        UserAction::UndoEdit => "Undo annotation edit",
        UserAction::RedoEdit => "Redo annotation edit",
        UserAction::SkipAssignment => "Skip assignment",
        UserAction::ToggleWorkflowPanel => "Toggle Workflow panel",
        UserAction::ToggleInspectorPanel => "Toggle Inspector panel",
        UserAction::OpenSettings => "Open shortcut settings",
        UserAction::SelectPreviousWorkflow => "Previous workflow",
        UserAction::SelectNextWorkflow => "Next workflow",
        UserAction::SelectPreviousObject => "Previous object",
        UserAction::SelectNextObject => "Next object",
        UserAction::SelectPreviousPrelabel => "Previous prelabel",
        UserAction::SelectNextPrelabel => "Next prelabel",
        UserAction::AcceptPrelabel => "Accept active prelabel",
        UserAction::DiscardPrelabel => "Discard active prelabel",
        UserAction::ToggleKeypointHidden => "Toggle occluded keypoint placement",
        UserAction::MarkKeypointAbsent => "Mark keypoint as not present",
        UserAction::AddMissingObject => "Add or cancel missing migration object",
        UserAction::RetryImageLoad => "Retry image load",
        UserAction::TogglePanMode => "Toggle Pan mode",
        UserAction::ZoomIn => "Zoom in",
        UserAction::ZoomOut => "Zoom out",
        UserAction::FitImage => "Fit image",
        UserAction::RefocusObject => "Refocus active object",
        UserAction::PreviousImage => "Previous assignment",
        UserAction::SaveAnnotations => "Save annotations",
        UserAction::DeleteAnnotation => "Delete annotation",
        UserAction::SelectBoundingBoxTool => "Bounding-box tool",
        UserAction::SelectKeypointTool => "Keypoint tool",
        UserAction::AcceptReviewObject => "Approve review object",
        UserAction::RejectReviewObject => "Reject review object",
        UserAction::OpenTutorial => "Open tutorial",
        UserAction::ToggleOfflineMode => "Offline mode",
    }
}

fn action_category(action: labello_domain::UserAction) -> &'static str {
    use labello_domain::UserAction;
    match action {
        UserAction::NextImage
        | UserAction::UndoEdit
        | UserAction::RedoEdit
        | UserAction::SaveAnnotations
        | UserAction::SkipAssignment
        | UserAction::PreviousImage => "Assignment",
        UserAction::SelectPreviousWorkflow
        | UserAction::SelectNextWorkflow
        | UserAction::SelectPreviousObject
        | UserAction::SelectNextObject
        | UserAction::DeleteAnnotation
        | UserAction::ToggleKeypointHidden
        | UserAction::MarkKeypointAbsent
        | UserAction::AddMissingObject => "Annotation",
        UserAction::SelectPreviousPrelabel
        | UserAction::SelectNextPrelabel
        | UserAction::AcceptPrelabel
        | UserAction::DiscardPrelabel => "Prelabels",
        UserAction::TogglePanMode
        | UserAction::ZoomIn
        | UserAction::ZoomOut
        | UserAction::FitImage
        | UserAction::RefocusObject => "Canvas",
        UserAction::OpenTutorial
        | UserAction::ToggleWorkflowPanel
        | UserAction::ToggleInspectorPanel
        | UserAction::OpenSettings
        | UserAction::RetryImageLoad => "Workspace",
        UserAction::AcceptReviewObject | UserAction::RejectReviewObject => "Review",
        UserAction::SelectBoundingBoxTool
        | UserAction::SelectKeypointTool
        | UserAction::ToggleOfflineMode => "Legacy",
    }
}

fn action_description(action: labello_domain::UserAction) -> &'static str {
    use labello_domain::UserAction;
    match action {
        UserAction::NextImage => "Save, complete, and claim another image.",
        UserAction::UndoEdit => "Reverse the last annotation edit.",
        UserAction::RedoEdit => "Restore the last undone edit.",
        UserAction::SaveAnnotations => "Save without leaving the assignment.",
        UserAction::SkipAssignment => "Release this image and claim another.",
        UserAction::DeleteAnnotation => "Delete the selected object.",
        UserAction::OpenTutorial => "Show or hide workflow instructions.",
        UserAction::ToggleWorkflowPanel => "Open or close workflow navigation.",
        UserAction::ToggleInspectorPanel => "Open or close object controls.",
        UserAction::OpenSettings => "Open this keyboard shortcut editor.",
        UserAction::SelectPreviousWorkflow => "Cycle to the previous enabled workflow.",
        UserAction::SelectNextWorkflow => "Cycle to the next enabled workflow.",
        UserAction::SelectPreviousObject => "Select the previous annotation.",
        UserAction::SelectNextObject => "Select the next annotation.",
        UserAction::SelectPreviousPrelabel => "Highlight the previous suggestion.",
        UserAction::SelectNextPrelabel => "Highlight the next suggestion.",
        UserAction::AcceptPrelabel => "Convert the active suggestion to an annotation.",
        UserAction::DiscardPrelabel => "Hide the active suggestion.",
        UserAction::ToggleKeypointHidden => "Toggle occluded placement for the next keypoint.",
        UserAction::MarkKeypointAbsent => "Record an allowed optional keypoint without a position.",
        UserAction::AddMissingObject => {
            "Begin or cancel a skeleton for an object missing from the imported data."
        }
        UserAction::RetryImageLoad => "Try to claim and load an image again.",
        UserAction::TogglePanMode => "Use primary drag to move a zoomed image.",
        UserAction::ZoomIn => "Increase canvas magnification.",
        UserAction::ZoomOut => "Decrease canvas magnification.",
        UserAction::FitImage => "Fit and center the image.",
        UserAction::RefocusObject => "Center and zoom to the active review or migration object.",
        UserAction::AcceptReviewObject => "Approve the current review object.",
        UserAction::RejectReviewObject => "Reject the current review object.",
        UserAction::PreviousImage => "Return to the last skipped or submitted assignment.",
        UserAction::SelectBoundingBoxTool
        | UserAction::SelectKeypointTool
        | UserAction::ToggleOfflineMode => "No longer used.",
    }
}

fn workflow_type_icon(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    annotation_type: &AnnotationType,
) {
    let color = match annotation_type {
        AnnotationType::BoundingBox => theme::INFO,
        AnnotationType::Skeleton => theme::PRELABEL,
    };
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(7),
        egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 36),
    );
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(7),
        egui::Stroke::new(1.0, color.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    match annotation_type {
        AnnotationType::BoundingBox => {
            painter.rect_stroke(
                rect.shrink2(egui::vec2(6.0, 7.0)),
                egui::CornerRadius::same(2),
                egui::Stroke::new(1.75, color),
                egui::StrokeKind::Inside,
            );
        }
        AnnotationType::Skeleton => {
            let center = rect.center();
            let head = center + egui::vec2(0.0, -8.0);
            let neck = center + egui::vec2(0.0, -3.0);
            let left_hand = center + egui::vec2(-7.0, 0.0);
            let right_hand = center + egui::vec2(7.0, 0.0);
            let hip = center + egui::vec2(0.0, 4.0);
            let left_foot = center + egui::vec2(-5.0, 9.0);
            let right_foot = center + egui::vec2(5.0, 9.0);
            for [start, end] in [
                [head, neck],
                [left_hand, neck],
                [neck, right_hand],
                [neck, hip],
                [hip, left_foot],
                [hip, right_foot],
            ] {
                painter.line_segment([start, end], egui::Stroke::new(1.5, color));
            }
            for point in [head, left_hand, right_hand, left_foot, right_foot] {
                painter.circle_filled(point, 1.75, color);
            }
        }
    }
    ui.interact(rect, id.with("semantics"), egui::Sense::hover())
        .widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                format!("{} annotation type", annotation_type_label(annotation_type)),
            )
        });
}

fn format_chord(ctx: &egui::Context, chord: &labello_domain::KeyChord) -> String {
    let Some(key) = egui::Key::from_name(&chord.key) else {
        return chord.to_string();
    };
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.command = chord.ctrl || chord.command;
    modifiers.shift = chord.shift;
    modifiers.alt = chord.alt;
    ctx.format_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
}

fn view_label(view: AppView) -> &'static str {
    match view {
        AppView::Setup => "Setup",
        AppView::Annotate => "Annotate",
        AppView::Review => "Review",
        AppView::Adjudicate => "Adjudicate",
        AppView::Admin => "Admin",
        AppView::Stats => "Stats",
    }
}

fn status_text(status: SaveStatus) -> &'static str {
    match status {
        SaveStatus::Idle => "Idle",
        SaveStatus::Dirty => "Unsaved",
        SaveStatus::Saved => "Saved",
        SaveStatus::Saving => "Saving",
        SaveStatus::Retry => "Retry",
    }
}

fn compact_status_text(status: SaveStatus) -> &'static str {
    match status {
        SaveStatus::Idle => "Idle",
        SaveStatus::Dirty => "Edit",
        SaveStatus::Saved => "Done",
        SaveStatus::Saving => "Wait",
        SaveStatus::Retry => "Retry",
    }
}

fn status_intent(status: SaveStatus) -> theme::Intent {
    match status {
        SaveStatus::Idle => theme::Intent::Neutral,
        SaveStatus::Dirty => theme::Intent::Warning,
        SaveStatus::Saved => theme::Intent::Success,
        SaveStatus::Saving => theme::Intent::Info,
        SaveStatus::Retry => theme::Intent::Error,
    }
}

fn keypoint_state_label(state: &KeypointState) -> &'static str {
    match state {
        KeypointState::Visible => "visible",
        KeypointState::Hidden => "occluded",
        KeypointState::Absent => "not present",
    }
}
