impl LabelloApp {
    fn admin_automation(&mut self, ui: &mut egui::Ui) {
        ui.heading("Automation");
        ui.label(
            RichText::new("Configure prelabels and assignment balancing.").color(theme::TEXT_MUTED),
        );
        let enabled = !self.loading.admin
            && self.loading.roles_user.is_none()
            && !self.loading.uploading
            && !self.loading.ingesting;
        if let Some(config) = self.datasets.admin_config.as_mut() {
            ui.add_enabled_ui(enabled, |ui| {
                edit_prelabels(ui, &mut config.prelabel_configs, &mut config.tasks);
                edit_imbalance(ui, &mut config.imbalance);
            });
        }
    }
}
