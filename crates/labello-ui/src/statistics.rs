use std::collections::BTreeMap;

use eframe::egui::{self, RichText};
use labello_domain::{ClassId, TaskId};

use crate::{
    app::{LabelloApp, LayoutMode},
    theme,
};

impl LabelloApp {
    pub(crate) fn stats_view(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let has_data = self.datasets.last_stats_completion.is_some();
        let initial_loading = self.loading.stats && !has_data;
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Live Statistics")
                    .size(theme::PAGE_TITLE_SIZE)
                    .strong(),
            );
            if has_data
                && theme::quiet_button(ui, !self.loading.stats, egui::Button::new("Refresh now"))
                    .on_hover_text(
                        "Refresh statistics immediately. They also refresh automatically.",
                    )
                    .clicked()
            {
                self.request_stats();
            }
        });
        if initial_loading {
            theme::card_frame().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading statistics...").strong());
                });
                ui.label(
                    RichText::new("Fetching the first dataset summary.").color(theme::TEXT_MUTED),
                );
            });
            return;
        }
        if !has_data {
            let (title, explanation, action) = if let Some(error) = &self.datasets.stats_error {
                (
                    "Statistics unavailable",
                    format!("The first statistics request failed: {error}"),
                    "Retry statistics",
                )
            } else {
                (
                    "Statistics have not loaded",
                    "Load the current dataset summary and activity history.".to_string(),
                    "Load statistics",
                )
            };
            if theme::empty_state(ui, title, &explanation, Some(egui::Button::new(action))) {
                self.request_stats();
            }
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if self.loading.stats {
                ui.label(RichText::new("Refreshing statistics").color(theme::TEXT_MUTED));
            }
            if let Some(completed) = self.datasets.last_stats_completion {
                let seconds = completed.elapsed().as_secs();
                ui.small(match seconds {
                    0 => "Updated just now".to_string(),
                    1 => "Updated 1 second ago".to_string(),
                    _ => format!("Updated {seconds} seconds ago"),
                });
            }
        });
        if let Some(error) = &self.datasets.stats_error {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                format!("Statistics may be stale. Last refresh failed: {error}"),
            );
        }
        let compact = layout == LayoutMode::Compact;
        let task_names = self
            .tasks
            .iter()
            .map(|task| (task.task_id.clone(), task.name.clone()))
            .collect::<BTreeMap<_, _>>();
        let class_names = self
            .classes
            .iter()
            .map(|class| (class.class_id.clone(), class.name.clone()))
            .collect::<BTreeMap<_, _>>();
        ui.add_space(8.0);
        let metrics = [
            ("Images", self.datasets.stats.total_images),
            ("Completed", self.datasets.stats.completed_tasks),
            ("Pending", self.datasets.stats.pending_tasks),
            ("Reviewed", self.datasets.stats.reviewed_tasks),
            ("Unreviewed", self.datasets.stats.unreviewed_tasks),
            ("Approved", self.datasets.stats.approved_tasks),
            ("Rejected", self.datasets.stats.rejected_tasks),
            (
                if compact {
                    "Corrected"
                } else {
                    "Reviewer corrected"
                },
                self.datasets.stats.reviewer_corrected_tasks,
            ),
            ("Finalized", self.datasets.stats.finalized_tasks),
        ];
        let minimum_card_width = if compact { 124.0 } else { 160.0 };
        let column_count = (((ui.available_width() + 10.0) / (minimum_card_width + 10.0)).floor()
            as usize)
            .clamp(1, 4);
        for row in metrics.chunks(column_count) {
            ui.columns(column_count, |columns| {
                for (column, (label, value)) in columns.iter_mut().zip(row) {
                    theme::metric(column, label, value.to_string());
                }
            });
        }
        ui.add_space(12.0);
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Per Task");
            let rows = &self.datasets.stats.per_task;
            if rows.is_empty() {
                theme::empty_state(
                    ui,
                    "No enabled tasks",
                    "Enable a labeling workflow to collect task statistics.",
                    None,
                );
            } else if compact {
                for (task_id, stats) in rows {
                    theme::inset_frame().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            RichText::new(
                                task_names
                                    .get(task_id)
                                    .map(String::as_str)
                                    .unwrap_or(task_id.as_str()),
                            )
                            .strong(),
                        );
                        ui.label(format!(
                            "Pending: {}  Unreviewed: {}  Reviewed: {}",
                            stats.pending, stats.unreviewed, stats.reviewed
                        ));
                        ui.label(format!(
                            "Approved: {}  Rejected: {}  Reviewer corrected: {}",
                            stats.approved, stats.rejected, stats.reviewer_corrected
                        ));
                        ui.label(format!(
                            "Finalized: {}  Done: {}",
                            stats.finalized, stats.completed
                        ));
                    });
                }
            } else {
                egui::ScrollArea::horizontal()
                    .id_salt("stats_tasks_horizontal")
                    .show(ui, |ui| {
                        stats_task_grid(ui, rows, &task_names);
                    });
            }
        });
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Per Class");
            let rows = &self.datasets.stats.per_class;
            if rows.is_empty() {
                theme::empty_state(
                    ui,
                    "No classes configured",
                    "Add a class to collect class-level statistics.",
                    None,
                );
            } else if compact {
                for (class_id, stats) in rows {
                    theme::inset_frame().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            RichText::new(
                                class_names
                                    .get(class_id)
                                    .map(String::as_str)
                                    .unwrap_or(class_id.as_str()),
                            )
                            .strong(),
                        );
                        ui.label(format!(
                            "Annotations: {}  Completed tasks: {}",
                            stats.annotations, stats.completed_tasks
                        ));
                    });
                }
            } else {
                egui::ScrollArea::horizontal()
                    .id_salt("stats_classes_horizontal")
                    .show(ui, |ui| {
                        stats_class_grid(ui, rows, &class_names);
                    });
            }
        });
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Throughput");
            if self.datasets.stats.throughput.is_empty() {
                theme::empty_state(
                    ui,
                    "No recorded activity",
                    "Throughput appears after annotations are created or reviews are recorded.",
                    None,
                );
            } else {
                stats_throughput_chart(ui, &self.datasets.stats.throughput);
            }
        });
    }
}

fn stats_task_grid(
    ui: &mut egui::Ui,
    rows: &BTreeMap<TaskId, labello_domain::TaskStats>,
    task_names: &BTreeMap<TaskId, String>,
) {
    egui::Grid::new("stats-task-grid")
        .num_columns(9)
        .striped(true)
        .spacing([theme::SPACE_3, theme::SPACE_1])
        .show(ui, |ui| {
            stats_name_cell(ui, "Task", 180.0, true);
            for heading in [
                "Pending",
                "Unreviewed",
                "Reviewed",
                "Approved",
                "Rejected",
                "Corrected",
                "Finalized",
                "Done",
            ] {
                stats_number_cell(ui, heading, 84.0, true);
            }
            ui.end_row();

            for (task_id, stats) in rows {
                stats_name_cell(
                    ui,
                    task_names
                        .get(task_id)
                        .map(String::as_str)
                        .unwrap_or(task_id.as_str()),
                    180.0,
                    false,
                );
                for value in [
                    stats.pending,
                    stats.unreviewed,
                    stats.reviewed,
                    stats.approved,
                    stats.rejected,
                    stats.reviewer_corrected,
                    stats.finalized,
                    stats.completed,
                ] {
                    stats_number_cell(ui, value, 84.0, false);
                }
                ui.end_row();
            }
        });
}

fn stats_class_grid(
    ui: &mut egui::Ui,
    rows: &BTreeMap<ClassId, labello_domain::ClassStats>,
    class_names: &BTreeMap<ClassId, String>,
) {
    egui::Grid::new("stats-class-grid")
        .num_columns(3)
        .striped(true)
        .spacing([theme::SPACE_3, theme::SPACE_1])
        .show(ui, |ui| {
            stats_name_cell(ui, "Class", 220.0, true);
            stats_number_cell(ui, "Annotations", 130.0, true);
            stats_number_cell(ui, "Completed tasks", 140.0, true);
            ui.end_row();

            for (class_id, stats) in rows {
                stats_name_cell(
                    ui,
                    class_names
                        .get(class_id)
                        .map(String::as_str)
                        .unwrap_or(class_id.as_str()),
                    220.0,
                    false,
                );
                stats_number_cell(ui, stats.annotations, 130.0, false);
                stats_number_cell(ui, stats.completed_tasks, 140.0, false);
                ui.end_row();
            }
        });
}

fn stats_name_cell(ui: &mut egui::Ui, value: &str, width: f32, header: bool) {
    let text = if header {
        RichText::new(value).strong().color(theme::TEXT_MUTED)
    } else {
        RichText::new(value).strong().color(theme::TEXT)
    };
    ui.add_sized(
        [width, 44.0],
        egui::Label::new(text).truncate().halign(egui::Align::Min),
    );
}

fn stats_number_cell(ui: &mut egui::Ui, value: impl ToString, width: f32, header: bool) {
    let text = if header {
        RichText::new(value.to_string())
            .strong()
            .color(theme::TEXT_MUTED)
    } else {
        RichText::new(value.to_string())
            .monospace()
            .color(theme::TEXT)
    };
    ui.add_sized(
        [width, 44.0],
        egui::Label::new(text).truncate().halign(egui::Align::Max),
    );
}

fn stats_throughput_chart(ui: &mut egui::Ui, points: &[labello_domain::ThroughputPoint]) {
    let points = points.iter().rev().take(14).rev().collect::<Vec<_>>();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Annotations").strong().color(theme::ACCENT));
        ui.label(RichText::new("Reviews").strong().color(theme::INFO));
        ui.label(
            RichText::new("Daily annotation and review activity")
                .size(theme::SUPPORTING_SIZE)
                .color(theme::TEXT_MUTED),
        );
    });
    let available_width = ui.available_width();
    egui::ScrollArea::horizontal()
        .id_salt("stats-throughput-chart-scroll")
        .show(ui, |ui| {
            let width = available_width.max(42.0 + points.len() as f32 * 48.0);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, 184.0), egui::Sense::hover());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Daily throughput chart")
            });

            let maximum = points
                .iter()
                .flat_map(|point| [point.annotations, point.reviews])
                .max()
                .unwrap_or(0)
                .max(1);
            let axis_width = stats_axis_width(maximum);
            let plot = egui::Rect::from_min_max(
                egui::pos2(rect.left() + axis_width, rect.top() + 8.0),
                egui::pos2(rect.right() - 8.0, rect.bottom() - 26.0),
            );
            let painter = ui.painter_at(rect);
            let font = egui::FontId::new(theme::SUPPORTING_SIZE, egui::FontFamily::Monospace);
            let tick_fractions: &[f32] = if maximum == 1 {
                &[0.0, 1.0]
            } else {
                &[0.0, 0.5, 1.0]
            };
            for &fraction in tick_fractions {
                let y = plot.bottom() - plot.height() * fraction;
                painter.line_segment(
                    [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                    egui::Stroke::new(1.0, theme::BORDER),
                );
                painter.text(
                    egui::pos2(plot.left() - 6.0, y),
                    egui::Align2::RIGHT_CENTER,
                    (maximum as f32 * fraction).round() as usize,
                    font.clone(),
                    theme::TEXT_MUTED,
                );
            }

            let group_width = plot.width() / points.len() as f32;
            let bar_width = (group_width * 0.26).clamp(4.0, 18.0);
            for (index, point) in points.iter().enumerate() {
                let left = plot.left() + index as f32 * group_width;
                let center = left + group_width * 0.5;
                for (value, x, color) in [
                    (point.annotations, center - bar_width - 1.0, theme::ACCENT),
                    (point.reviews, center + 1.0, theme::INFO),
                ] {
                    if value > 0 {
                        let height = plot.height() * value as f32 / maximum as f32;
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(x, plot.bottom() - height),
                                egui::pos2(x + bar_width, plot.bottom()),
                            ),
                            egui::CornerRadius::same(2),
                            color,
                        );
                    }
                }
                painter.text(
                    egui::pos2(center, plot.bottom() + 6.0),
                    egui::Align2::CENTER_TOP,
                    point.day.get(5..).unwrap_or(&point.day),
                    font.clone(),
                    theme::TEXT_MUTED,
                );

                let detail = format!(
                    "{}: {} {}, {} {}",
                    point.day,
                    point.annotations,
                    if point.annotations == 1 {
                        "annotation"
                    } else {
                        "annotations"
                    },
                    point.reviews,
                    if point.reviews == 1 {
                        "review"
                    } else {
                        "reviews"
                    }
                );
                let hit = egui::Rect::from_min_max(
                    egui::pos2(left, plot.top()),
                    egui::pos2(left + group_width, rect.bottom()),
                );
                let response = ui
                    .interact(
                        hit,
                        ui.id().with(("throughput-point", index)),
                        egui::Sense::hover(),
                    )
                    .on_hover_text(detail.clone());
                response.widget_info(move || {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, detail.clone())
                });
            }
        });
}

fn stats_axis_width(maximum: usize) -> f32 {
    (maximum.to_string().len() as f32 * 8.0 + 12.0).max(34.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_axis_gutter_scales_with_large_values() {
        assert_eq!(stats_axis_width(1), 34.0);
        assert!(stats_axis_width(12_345) >= 52.0);
    }
}
