use eframe::egui::{self, RichText};
use labello_domain::{DatasetRole, UserId};

use crate::{
    app::{AppView, LabelloApp, PendingTransition},
    theme,
};

impl LabelloApp {
    pub(crate) fn setup_view(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.heading(RichText::new("Connect To Labello").size(28.0));
            ui.label(
                RichText::new("Configure identity, open a dataset, or create one without curl.")
                    .color(theme::MUTED),
            );
        });
        ui.add_space(18.0);
        theme::card_frame().show(ui, |ui| {
            ui.heading("Connection");
            ui.horizontal(|ui| {
                ui.label("API URL");
                ui.text_edit_singleline(&mut self.config.api_base_url)
                    .on_hover_text("Backend API base URL, for example http://127.0.0.1:8080.");
            });
            ui.horizontal(|ui| {
                ui.label("Dev token");
                ui.add(egui::TextEdit::singleline(&mut self.config.dev_token).password(true))
                    .on_hover_text(
                        "Must match devAuth.token in labello.server.toml when dev auth is enabled.",
                    );
            });
            let mut user_id = self.config.user_id.to_string();
            ui.horizontal(|ui| {
                ui.label("User ID");
                if ui
                    .text_edit_singleline(&mut user_id)
                    .on_hover_text("User id sent to the backend for local testing.")
                    .changed()
                {
                    self.config.user_id = UserId::from(user_id);
                }
            });
            if ui
                .button("Apply and refresh datasets")
                .on_hover_text(
                    "Apply connection settings and list datasets accessible to this user.",
                )
                .clicked()
            {
                if let Err(error) = self.config.user_id.validate_path_segment() {
                    self.runtime.error = Some(format!("User ID: {error}"));
                } else {
                    self.current = None;
                    self.datasets.metadata = None;
                    self.datasets.admin_config = None;
                    self.datasets.admin_baseline = None;
                    self.datasets.summaries.clear();
                    self.rebuild_http_api();
                    self.setup.started = true;
                    self.request_dataset_list();
                }
            }
        });

        ui.add_space(12.0);
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Datasets");
                if self.loading.datasets || self.loading.dataset {
                    ui.spinner();
                }
                if self.loading.dataset {
                    ui.small("Opening dataset...");
                }
                if ui
                    .button("Refresh")
                    .on_hover_text("Reload the accessible dataset list.")
                    .clicked()
                {
                    self.request_dataset_list();
                }
            });
            if self.datasets.summaries.is_empty() {
                ui.label(RichText::new("No accessible datasets yet.").color(theme::MUTED));
            }
            let datasets = self.datasets.summaries.clone();
            for dataset in datasets {
                theme::card_frame().show(ui, |ui| {
                    ui.label(RichText::new(&dataset.name).strong());
                    ui.small(format!("{} images", dataset.total_images));
                    ui.horizontal_wrapped(|ui| {
                        for role in &dataset.roles {
                            role_badge(ui, role);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (role, view, label) in [
                            (DatasetRole::Annotator, AppView::Annotate, "Annotate"),
                            (DatasetRole::Reviewer, AppView::Review, "Review"),
                            (DatasetRole::Adjudicator, AppView::Adjudicate, "Adjudicate"),
                        ] {
                            if dataset.roles.contains(&role) && ui.button(label).clicked() {
                                self.open_dataset(dataset.dataset_id.clone(), view);
                            }
                        }
                        if dataset.roles.contains(&DatasetRole::DataAdmin)
                            && ui.button("Admin").clicked()
                        {
                            self.open_dataset(dataset.dataset_id.clone(), AppView::Admin);
                        }
                        if !dataset.roles.is_empty() && ui.button("Stats").clicked() {
                            self.open_dataset(dataset.dataset_id.clone(), AppView::Stats);
                        }
                    });
                });
                ui.add_space(8.0);
            }
        });

        ui.add_space(12.0);
        theme::card_frame().show(ui, |ui| {
            ui.heading("Create Dataset");
            ui.horizontal(|ui| {
                ui.label("ID");
                ui.text_edit_singleline(&mut self.setup.create_dataset_id)
                    .on_hover_text("Stable dataset id used as the dataset folder name.");
            });
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.setup.create_dataset_name)
                    .on_hover_text("Human-readable dataset name.");
            });
            if ui
                .button("Create as current user")
                .on_hover_text("Create a new dataset. Requires bootstrap admin access.")
                .clicked()
            {
                self.request_create_dataset();
            }
            ui.small("Dataset creation requires the current user to be a bootstrap admin in labello.server.toml.");
        });
    }

    pub(crate) fn mode_toolbar(&mut self, ui: &mut egui::Ui) {
        if ui
            .selectable_label(self.view == AppView::Setup, "Setup")
            .clicked()
        {
            self.open_view(AppView::Setup);
        }
        for (view, role, label) in [
            (AppView::Annotate, DatasetRole::Annotator, "Annotate"),
            (AppView::Review, DatasetRole::Reviewer, "Review"),
            (AppView::Adjudicate, DatasetRole::Adjudicator, "Adjudicate"),
        ] {
            if self.has_dataset_role(role)
                && ui.selectable_label(self.view == view, label).clicked()
            {
                self.open_view(view);
            }
        }
        if self.datasets.metadata.is_some()
            && ui
                .selectable_label(self.view == AppView::Stats, "Stats")
                .clicked()
        {
            self.open_view(AppView::Stats);
        }
        if self.can_admin()
            && ui
                .selectable_label(self.view == AppView::Admin, "Admin")
                .clicked()
        {
            self.open_view(AppView::Admin);
        }
    }

    pub(crate) fn open_view(&mut self, view: AppView) {
        if view == AppView::Setup {
            self.request_transition(PendingTransition::View(view));
            return;
        }
        if self.datasets.metadata.is_none() {
            self.datasets.requested_view = Some(view);
            self.request_load_dataset();
            return;
        }
        if !self.can_open_view(view) {
            self.runtime.error =
                Some("The current user is not authorized for that view.".to_string());
            return;
        }
        if matches!(
            view,
            AppView::Annotate | AppView::Review | AppView::Adjudicate
        ) && !self.ensure_valid_task_selection()
        {
            self.runtime.error = Some(
                "No enabled one-class workflow is configured. Ask a data admin to enable one."
                    .to_string(),
            );
            return;
        }
        self.request_transition(PendingTransition::View(view));
    }

    pub(crate) fn open_dataset(&mut self, dataset_id: labello_domain::DatasetId, view: AppView) {
        if self.loading.dataset {
            return;
        }
        self.config.dataset_id = dataset_id;
        self.datasets.requested_view = Some(view);
        self.request_load_dataset();
    }

    pub(crate) fn can_admin(&self) -> bool {
        self.has_dataset_role(DatasetRole::DataAdmin)
    }
}

fn role_badge(ui: &mut egui::Ui, role: &DatasetRole) {
    egui::Frame::new()
        .fill(theme::BLUE.gamma_multiply(0.16))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(role.to_string()).color(theme::BLUE));
        });
}
