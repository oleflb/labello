fn source_file_selector(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut String,
    paths: &[RegisteredImportPath],
    include: impl Fn(&str) -> bool,
) -> bool {
    let previous = selected.clone();
    egui::ComboBox::from_label(label)
        .selected_text(
            paths
                .iter()
                .find(|path| path.file_id == *selected || path.client_file_id == *selected)
                .map(|path| path.relative_path.as_str())
                .unwrap_or("Choose a registered file"),
        )
        .show_ui(ui, |ui| {
            for path in paths.iter().filter(|path| include(&path.relative_path)) {
                let reference = if path.file_id.is_empty() {
                    &path.client_file_id
                } else {
                    &path.file_id
                };
                ui.selectable_value(selected, reference.clone(), &path.relative_path);
            }
        });
    *selected != previous
}

fn server_source_file_picker(
    ui: &mut egui::Ui,
    label: &str,
    selected: &str,
    paths: &[RegisteredImportPath],
    button_label: &str,
) -> bool {
    let display = paths
        .iter()
        .find(|path| path.file_id == selected)
        .map(|path| path.relative_path.as_str())
        .unwrap_or(if selected.is_empty() {
            "Not selected"
        } else {
            "Selected staged file"
        });
    status_row(ui, label, display);
    ui.button(button_label).clicked()
}


fn current_import_stage(flow: &ImportFlowState) -> ImportStage {
    match flow.screen {
        ImportScreen::Source => ImportStage::Source,
        ImportScreen::Configure => ImportStage::Configure,
        ImportScreen::Preflight => ImportStage::Preflight,
        ImportScreen::Ready => ImportStage::Ready,
        ImportScreen::Running | ImportScreen::Success => ImportStage::Import,
        ImportScreen::Failure => failure_import_stage(flow),
    }
}

fn failure_import_stage(flow: &ImportFlowState) -> ImportStage {
    let phase = flow
        .job
        .as_ref()
        .and_then(|job| job.failure.as_ref().map(|failure| failure.phase))
        .or_else(|| {
            (flow.plan.is_none())
                .then(|| flow.job.as_ref().map(|job| job.progress.phase))
                .flatten()
        });
    match phase {
        Some(labello_client::ImportProgressPhase::Registration) => ImportStage::Source,
        Some(
            labello_client::ImportProgressPhase::Upload
            | labello_client::ImportProgressPhase::Sealing,
        ) => ImportStage::Configure,
        Some(labello_client::ImportProgressPhase::Preflight) => ImportStage::Preflight,
        Some(
            labello_client::ImportProgressPhase::Build
            | labello_client::ImportProgressPhase::Verification
            | labello_client::ImportProgressPhase::Commit,
        ) => ImportStage::Import,
        Some(labello_client::ImportProgressPhase::Cleanup) | None => {
            if flow.plan.is_some() {
                ImportStage::Import
            } else if flow
                .job
                .as_ref()
                .is_some_and(|job| job.preflight_report.is_some())
            {
                ImportStage::Preflight
            } else if flow.job.is_some() {
                ImportStage::Configure
            } else {
                ImportStage::Source
            }
        }
        Some(labello_client::ImportProgressPhase::Unknown) => ImportStage::Configure,
    }
}

fn import_stage_status(flow: &ImportFlowState, stage: ImportStage) -> ImportStageStatus {
    if flow.screen == ImportScreen::Success {
        return ImportStageStatus::Complete;
    }
    let current = current_import_stage(flow);
    if flow.screen == ImportScreen::Failure && stage == current {
        return ImportStageStatus::Failed;
    }
    match stage.index().cmp(&current.index()) {
        std::cmp::Ordering::Less => ImportStageStatus::Complete,
        std::cmp::Ordering::Equal => ImportStageStatus::Active,
        std::cmp::Ordering::Greater => ImportStageStatus::Pending,
    }
}

fn import_stage_pill(
    ui: &mut egui::Ui,
    stage: ImportStage,
    status: ImportStageStatus,
    fraction: Option<f32>,
) {
    let (color, fill, stroke) = match status {
        ImportStageStatus::Pending => (theme::TEXT_DISABLED, theme::SURFACE, theme::BORDER),
        ImportStageStatus::Active => (
            theme::ACCENT,
            egui::Color32::from_rgba_unmultiplied(
                theme::ACCENT.r(),
                theme::ACCENT.g(),
                theme::ACCENT.b(),
                60,
            ),
            theme::ACCENT,
        ),
        ImportStageStatus::Complete => (
            theme::SUCCESS,
            egui::Color32::from_rgba_unmultiplied(
                theme::SUCCESS.r(),
                theme::SUCCESS.g(),
                theme::SUCCESS.b(),
                24,
            ),
            theme::SUCCESS.gamma_multiply(0.6),
        ),
        ImportStageStatus::Failed => (
            theme::DANGER,
            egui::Color32::from_rgba_unmultiplied(
                theme::DANGER.r(),
                theme::DANGER.g(),
                theme::DANGER.b(),
                36,
            ),
            theme::DANGER,
        ),
    };
    let response = egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(egui::CornerRadius::same(theme::BADGE_RADIUS))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(78.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(format!("{} {}", stage.index() + 1, stage.label()))
                        .color(color)
                        .small()
                        .strong(),
                );
                let (track, _) =
                    ui.allocate_exact_size(egui::vec2(78.0, 3.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(track, 2.0, theme::BORDER_STRONG.gamma_multiply(0.55));
                match fraction {
                    Some(fraction) => {
                        let width = track.width() * fraction.clamp(0.0, 1.0);
                        if width > 0.0 {
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    track.min,
                                    egui::vec2(width, track.height()),
                                ),
                                2.0,
                                color,
                            );
                        }
                    }
                    None => {
                        ui.ctx().request_repaint();
                        let phase = ((ui.input(|input| input.time) as f32 * 0.7) % 1.35) - 0.35;
                        let segment = egui::Rect::from_min_size(
                            egui::pos2(track.left() + track.width() * phase, track.top()),
                            egui::vec2(track.width() * 0.35, track.height()),
                        );
                        ui.painter()
                            .with_clip_rect(track)
                            .rect_filled(segment, 2.0, color);
                    }
                }
            });
        })
        .response;
    let status_label = match status {
        ImportStageStatus::Pending => "pending",
        ImportStageStatus::Active => "current",
        ImportStageStatus::Complete => "complete",
        ImportStageStatus::Failed => "failed",
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ProgressIndicator,
            true,
            format!(
                "Step {} of 5: {}, {status_label}",
                stage.index() + 1,
                stage.label()
            ),
        )
    });
}

fn indeterminate_import_progress(ui: &mut egui::Ui, label: &str, color: egui::Color32) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(96.0), 18.0),
        egui::Sense::hover(),
    );
    ui.ctx().request_repaint();
    ui.painter()
        .rect_filled(rect, 12.0, ui.visuals().extreme_bg_color);
    let phase = ((ui.input(|input| input.time) as f32 * 0.55) % 1.4) - 0.4;
    let segment = egui::Rect::from_min_size(
        egui::pos2(rect.left() + rect.width() * phase, rect.top()),
        egui::vec2(rect.width() * 0.4, rect.height()),
    );
    ui.painter()
        .with_clip_rect(rect)
        .rect_filled(segment, 12.0, color.gamma_multiply(0.8));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, label.to_string())
    });
}

fn import_human_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub(crate) fn import_screen(job: &ImportJob, plan: Option<&ImportPlan>) -> ImportScreen {
    match job.lifecycle {
        ImportLifecycle::Registering | ImportLifecycle::Uploading | ImportLifecycle::Sealed => {
            ImportScreen::Configure
        }
        ImportLifecycle::Preflighting => ImportScreen::Preflight,
        ImportLifecycle::AwaitingDecision if plan.is_some_and(|plan| plan.commit_ready) => {
            ImportScreen::Ready
        }
        ImportLifecycle::AwaitingDecision => ImportScreen::Preflight,
        ImportLifecycle::Building | ImportLifecycle::Verifying | ImportLifecycle::Committing => {
            ImportScreen::Running
        }
        ImportLifecycle::Succeeded => ImportScreen::Success,
        ImportLifecycle::Failed | ImportLifecycle::Cancelled | ImportLifecycle::Expired => {
            ImportScreen::Failure
        }
        ImportLifecycle::Unknown => ImportScreen::Failure,
    }
}

fn import_step_label(screen: ImportScreen) -> &'static str {
    match screen {
        ImportScreen::Source => "1. Source, profile, transport, and attestations",
        ImportScreen::Configure => "2. Register files and configure the source",
        ImportScreen::Preflight => "3. Preflight diagnostics and mappings",
        ImportScreen::Ready => "4. Ready to commit",
        ImportScreen::Running => "5. Building and verifying the dataset",
        ImportScreen::Failure => "Import needs attention",
        ImportScreen::Success => "Import complete",
    }
}

fn profile_label(profile: ImportProfile) -> &'static str {
    match profile {
        ImportProfile::UltralyticsYoloDetectV1 => "Ultralytics YOLO detect v1",
        ImportProfile::UltralyticsYoloPoseV1 => "Ultralytics YOLO pose v1",
        ImportProfile::CocoInstancesGtV1 => "COCO instances ground truth v1",
        ImportProfile::CocoKeypointsGtV1 => "COCO keypoints ground truth v1",
        ImportProfile::Unknown => "Unknown profile",
    }
}

fn transport_label(transport: ImportTransport) -> &'static str {
    match transport {
        ImportTransport::BrowserFolder => "Browser folder upload",
        ImportTransport::ServerDirectory => "Server directory",
        ImportTransport::Unknown => "Unknown transport",
    }
}


fn intent_label(intent: ImportWorkflowIntent) -> &'static str {
    match intent {
        ImportWorkflowIntent::AuthoritativeGroundTruth => "Authoritative ground truth",
        ImportWorkflowIntent::RequireApproval => "Require approval",
        ImportWorkflowIntent::SeedFutureAnnotation => "Seed future annotation",
    }
}

fn lifecycle_label(lifecycle: ImportLifecycle) -> &'static str {
    match lifecycle {
        ImportLifecycle::Registering => "Registering files",
        ImportLifecycle::Uploading => "Uploading files",
        ImportLifecycle::Sealed => "Source sealed",
        ImportLifecycle::Preflighting => "Running preflight",
        ImportLifecycle::AwaitingDecision => "Awaiting decision",
        ImportLifecycle::Building => "Building dataset",
        ImportLifecycle::Verifying => "Verifying dataset",
        ImportLifecycle::Committing => "Committing dataset",
        ImportLifecycle::Succeeded => "Succeeded",
        ImportLifecycle::Failed => "Failed",
        ImportLifecycle::Cancelled => "Cancelled",
        ImportLifecycle::Expired => "Expired",
        ImportLifecycle::Unknown => "Unknown",
    }
}

#[derive(Default)]
struct ImportDiagnosticOverview {
    errors: usize,
    warnings: usize,
    information: usize,
    affected: u64,
    blocking: usize,
    unacknowledged: usize,
}

impl ImportDiagnosticOverview {
    fn from_diagnostics(
        diagnostics: &[labello_client::ImportDiagnosticSummary],
        acknowledgements: &std::collections::BTreeSet<String>,
    ) -> ImportDiagnosticOverview {
        let mut overview = Self::default();
        for diagnostic in diagnostics {
            match diagnostic.severity {
                ImportDiagnosticSeverity::Error => overview.errors += 1,
                ImportDiagnosticSeverity::WarningRequiresAck
                | ImportDiagnosticSeverity::Warning => overview.warnings += 1,
                ImportDiagnosticSeverity::Info | ImportDiagnosticSeverity::Unknown => {
                    overview.information += 1;
                }
            }
            overview.affected = overview.affected.saturating_add(diagnostic.count);
            overview.blocking += usize::from(diagnostic.impact.blocks_commit);
            overview.unacknowledged += usize::from(
                diagnostic.impact.requires_acknowledgement
                    && !acknowledgements.contains(&diagnostic.code),
            );
        }
        overview
    }

    fn disclosure_label(&self, compact: bool) -> String {
        let mut severities = Vec::new();
        if self.errors > 0 {
            severities.push(counted(self.errors, "error"));
        }
        if self.warnings > 0 {
            severities.push(counted(self.warnings, "warning"));
        }
        if self.information > 0 {
            severities.push(format!("{} info", self.information));
        }
        if severities.is_empty() {
            return "Diagnostics (none)".to_string();
        }

        if compact {
            let action = if self.blocking > 0 {
                " · commit blocked"
            } else if self.unacknowledged > 0 {
                " · action required"
            } else {
                ""
            };
            format!("Diagnostics ({}){action}", severities.join(", "))
        } else {
            let mut parts = vec![severities.join(", ")];
            if self.affected > 0 {
                parts.push(format!("{} affected", self.affected));
            }
            if self.blocking > 0 {
                parts.push(counted(self.blocking, "blocking diagnostic"));
            }
            if self.unacknowledged > 0 {
                parts.push(format!(
                    "{} acknowledgement{} required",
                    self.unacknowledged,
                    if self.unacknowledged == 1 { "" } else { "s" }
                ));
            }
            format!("Diagnostics — {}", parts.join(" · "))
        }
    }

    fn color(&self) -> egui::Color32 {
        if self.errors > 0 || self.blocking > 0 {
            theme::DANGER
        } else if self.warnings > 0 || self.unacknowledged > 0 {
            theme::WARNING
        } else if self.information > 0 {
            theme::INFO
        } else {
            theme::SUCCESS
        }
    }
}

fn counted(count: usize, singular: &str) -> String {
    format!("{count} {singular}{}", if count == 1 { "" } else { "s" })
}

fn diagnostic_severity_label(severity: ImportDiagnosticSeverity) -> &'static str {
    match severity {
        ImportDiagnosticSeverity::Error => "Error",
        ImportDiagnosticSeverity::WarningRequiresAck => "Warning requiring acknowledgement",
        ImportDiagnosticSeverity::Warning => "Warning",
        ImportDiagnosticSeverity::Info => "Information",
        ImportDiagnosticSeverity::Unknown => "Unknown-severity",
    }
}

fn status_row(ui: &mut egui::Ui, label: &str, value: impl ToString) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{label}:")).strong());
        ui.label(value.to_string());
    });
}

fn show_mapping_issues(
    ui: &mut egui::Ui,
    validation: &ImportMappingValidation,
    category_index: Option<usize>,
    field: ImportMappingField,
) {
    for issue in validation.for_field(category_index, field) {
        theme::inline_message(
            ui,
            match issue.severity {
                ImportMappingIssueSeverity::Error => theme::Intent::Error,
                ImportMappingIssueSeverity::Warning => theme::Intent::Warning,
            },
            &issue.message,
        );
    }
}
