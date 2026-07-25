use eframe::egui::{self, RichText};

use crate::{
    app::{AppView, LabelloApp, Tool},
    canvas::{CanvasAction, CanvasInteraction, show_canvas_styled},
    theme,
};

impl LabelloApp {
    pub(crate) fn workspace_canvas(&mut self, ui: &mut egui::Ui) {
        if let Some(current) = self.current.clone() {
            let texture = self.current_texture.clone();
            let mut annotations = self
                .annotations
                .iter()
                .filter(|annotation| self.annotation_matches_selected_workflow(annotation))
                .cloned()
                .collect::<Vec<_>>();
            if let Some(draft) = self.correction_draft.as_ref()
                && let Some(annotation) = annotations
                    .iter_mut()
                    .find(|annotation| annotation.annotation_id == draft.annotation_id)
            {
                annotation.geometry = draft.edited_geometry.clone();
            }
            let skeleton_edges = self
                .selected_task()
                .and_then(|task| task.skeleton.as_ref())
                .map(|skeleton| {
                    skeleton
                        .edges
                        .iter()
                        .map(|edge| (edge.from.clone(), edge.to.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let prelabels = self.visible_prelabels();
            let annotator_editable =
                self.view == AppView::Annotate && self.pending_transition.is_none();
            let correction_interaction = self.correction_draft.as_ref().map(|draft| {
                let mut interaction = CanvasInteraction::correction(draft.selected_keypoint);
                interaction.editable = !self.loading.saving;
                interaction
            });
            let interaction = correction_interaction
                .unwrap_or_else(|| CanvasInteraction::annotations(annotator_editable));
            let bounding_box_tool = self.tool == Tool::BoundingBox;
            let selected_annotation = self.selected_annotation.clone();
            if self.view == AppView::Review {
                let review_annotation = selected_annotation.as_ref().and_then(|id| {
                    annotations
                        .iter()
                        .find(|annotation| !annotation.deleted && &annotation.annotation_id == id)
                });
                self.canvas.set_review_focus(review_annotation);
            } else {
                self.canvas.clear_review_focus();
            }
            let annotation_color = self
                .selected_class_id()
                .and_then(|class_id| {
                    self.classes
                        .iter()
                        .find(|class| &class.class_id == class_id)
                })
                .and_then(|class| parse_class_color(&class.color))
                .unwrap_or(theme::ANNOTATION);
            let action = show_canvas_styled(
                ui,
                &mut self.canvas,
                texture.as_ref(),
                &annotations,
                [current.image.width, current.image.height],
                bounding_box_tool,
                selected_annotation.as_ref(),
                interaction,
                &skeleton_edges,
                &prelabels,
                annotation_color,
            );
            if annotator_editable {
                match action {
                    Some(CanvasAction::CreateBoundingBox(bbox)) => self.create_bbox(bbox),
                    Some(CanvasAction::PlaceKeypoint(point)) => self.place_keypoint(point),
                    Some(CanvasAction::Select(id)) => self.selected_annotation = Some(id),
                    Some(CanvasAction::EditBoundingBox(edit)) => self.edit_bbox(edit),
                    Some(CanvasAction::SelectKeypoint(_)) | Some(CanvasAction::EditKeypoint(_)) => {
                    }
                    None => {}
                }
            } else if self.correction_draft.is_some() {
                match action {
                    Some(CanvasAction::EditBoundingBox(edit)) => self.edit_correction_bbox(edit),
                    Some(CanvasAction::SelectKeypoint(selection)) => {
                        if self
                            .correction_draft
                            .as_ref()
                            .is_some_and(|draft| draft.annotation_id == selection.annotation_id)
                        {
                            self.select_correction_keypoint(selection.keypoint_index);
                        }
                    }
                    Some(CanvasAction::EditKeypoint(edit)) => self.edit_correction_keypoint(edit),
                    Some(CanvasAction::CreateBoundingBox(_))
                    | Some(CanvasAction::PlaceKeypoint(_))
                    | Some(CanvasAction::Select(_))
                    | None => {}
                }
            }
        } else {
            ui.add_space(((ui.available_height() - 160.0) * 0.5).max(0.0));
            let width = ui.available_width().min(520.0);
            let inset = ((ui.available_width() - width) * 0.5).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(inset);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    if self.loading.dataset {
                        theme::inset_frame().show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Opening dataset").strong());
                            });
                            ui.label(
                                RichText::new("Loading workflows and dataset metadata.")
                                    .color(theme::TEXT_MUTED),
                            );
                        });
                    } else if self.loading.image {
                        theme::inset_frame().show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Loading assignment image").strong());
                            });
                            ui.label(
                                RichText::new("Decoding the image preview for the canvas.")
                                    .color(theme::TEXT_MUTED),
                            );
                        });
                    } else if let Some(error) = self.runtime.error.clone() {
                        let claimed = self.assignment.is_some();
                        let (title, retry) = if claimed {
                            ("Assignment image unavailable", "Retry image load")
                        } else {
                            ("Assignment unavailable", "Retry assignment")
                        };
                        let shortcut = self
                            .shortcut_text(ui.ctx(), labello_domain::UserAction::RetryImageLoad);
                        if theme::empty_state(
                            ui,
                            title,
                            &error,
                            Some(egui::Button::new(retry).shortcut_text(shortcut)),
                        ) {
                            self.retry_assignment_load();
                        }
                    } else {
                        let title = match self.view {
                            AppView::Annotate => "No annotation assignments",
                            AppView::Review => "No review assignments",
                            AppView::Adjudicate => "No adjudication assignments",
                            _ => "No assignments",
                        };
                        let shortcut = self
                            .shortcut_text(ui.ctx(), labello_domain::UserAction::RetryImageLoad);
                        if theme::empty_state(
                            ui,
                            title,
                            "No work is available right now. Retry to check again.",
                            Some(egui::Button::new("Retry image load").shortcut_text(shortcut)),
                        ) {
                            self.retry_assignment_load();
                        }
                    }
                });
            });
        }
    }
}

fn parse_class_color(value: &str) -> Option<egui::Color32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(egui::Color32::from_rgb(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_colors_parse_with_a_safe_fallback_boundary() {
        assert_eq!(
            parse_class_color("#5eead4"),
            Some(egui::Color32::from_rgb(94, 234, 212))
        );
        assert_eq!(parse_class_color("5eead4"), None);
        assert_eq!(parse_class_color("#invalid"), None);
    }
}
