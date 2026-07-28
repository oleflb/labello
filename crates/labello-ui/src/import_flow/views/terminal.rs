impl LabelloApp {
    fn import_running_step(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = &self.import_flow.job {
            status_row(ui, "Status", lifecycle_label(job.lifecycle));
            status_row(
                ui,
                "Images",
                format!(
                    "{} of {}",
                    job.progress.processed_images, job.progress.total_images
                ),
            );
            status_row(
                ui,
                "Objects",
                format!(
                    "{} of {}",
                    job.progress.processed_objects, job.progress.total_objects
                ),
            );
            if job.can_cancel
                && ui
                    .add_enabled(!self.import_flow.busy, egui::Button::new("Cancel import"))
                    .clicked()
            {
                self.request_cancel_import();
            }
        }
    }

    fn import_failure_step(&mut self, ui: &mut egui::Ui) {
        if let Some(failure) = self
            .import_flow
            .job
            .as_ref()
            .and_then(|job| job.failure.as_ref())
        {
            status_row(ui, "Failure code", failure.code.clone());
            theme::inline_message(ui, theme::Intent::Error, &failure.safe_summary);
            status_row(
                ui,
                "Retry",
                if failure.retryable {
                    "Available"
                } else {
                    "Not available"
                },
            );
            if failure.retryable
                && theme::primary_button(
                    ui,
                    !self.import_flow.busy,
                    egui::Button::new("Retry import"),
                )
                .clicked()
            {
                self.request_retry_import();
            }
        }
        if ui
            .add_enabled(!self.import_flow.busy, egui::Button::new("Cancel import"))
            .clicked()
        {
            self.request_cancel_import();
        }
        if ui.button("Start another import").clicked() {
            self.begin_import_epoch();
            self.import_flow.reset_job();
        }
    }

    fn import_success_step(&mut self, ui: &mut egui::Ui) {
        theme::inline_message(
            ui,
            theme::Intent::Success,
            "Import committed and verified successfully.",
        );
        if let Some(job) = &self.import_flow.job {
            status_row(ui, "Dataset ID", job.destination_dataset_id.to_string());
            status_row(ui, "Plan hash", job.plan_hash.clone().unwrap_or_default());
        }
        let dataset_ready = self.import_flow.job.as_ref().is_some_and(|job| {
            self.datasets
                .summaries
                .iter()
                .any(|dataset| dataset.dataset_id == job.destination_dataset_id)
        }) && !self.loading.datasets;
        if self.loading.datasets {
            ui.label("Refreshing the dataset catalog before navigation...");
        }
        if theme::primary_button(
            ui,
            dataset_ready,
            egui::Button::new("Open imported dataset Admin"),
        )
        .on_disabled_hover_text("Wait for the imported dataset to appear in the refreshed catalog.")
        .clicked()
            && let Some(job) = &self.import_flow.job
        {
            let dataset_id = job.destination_dataset_id.clone();
            self.import_flow.open = false;
            self.open_dataset(dataset_id, AppView::Admin);
        }
        if ui.button("Import another dataset").clicked() {
            self.begin_import_epoch();
            self.import_flow.reset_job();
        }
    }
}
