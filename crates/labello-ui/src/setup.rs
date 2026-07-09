use eframe::egui::{self, RichText};
use labello_domain::{DatasetRole, UserId};

use crate::{
    app::{AppView, LabelloApp},
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
                ui.text_edit_singleline(&mut self.config.dev_token)
                    .on_hover_text(
                        "Must match devAuth.token in labello.server.toml when dev auth is enabled.",
                    );
            });
            let mut user_id = self.config.user_id.to_string();
            if ui
                .text_edit_singleline(&mut user_id)
                .on_hover_text("User id sent to the backend for local testing.")
                .changed()
            {
                self.config.user_id = UserId::from(user_id);
            }
            egui::ComboBox::from_label("Role header")
                .selected_text(self.config.role.to_string())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.config.role, DatasetRole::Annotator, "annotator");
                    ui.selectable_value(&mut self.config.role, DatasetRole::Reviewer, "reviewer");
                    ui.selectable_value(
                        &mut self.config.role,
                        DatasetRole::Adjudicator,
                        "adjudicator",
                    );
                    ui.selectable_value(
                        &mut self.config.role,
                        DatasetRole::DataAdmin,
                        "data_admin",
                    );
                });
            if ui
                .button("Apply and refresh datasets")
                .on_hover_text(
                    "Apply connection settings and list datasets accessible to this user.",
                )
                .clicked()
            {
                self.rebuild_http_api();
                self.setup.started = true;
                self.request_dataset_list();
            }
        });

        ui.add_space(12.0);
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Datasets");
                if self.loading.datasets {
                    ui.spinner();
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
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&dataset.name).strong());
                    ui.small(format!("{} images", dataset.total_images));
                    ui.small(format!("roles: {}", roles_text(&dataset.roles)));
                    if ui
                        .button("Open")
                        .on_hover_text("Open this dataset for annotation/review work.")
                        .clicked()
                    {
                        self.config.dataset_id = dataset.dataset_id.clone();
                        self.request_load_dataset();
                    }
                    if dataset.roles.contains(&DatasetRole::DataAdmin)
                        && ui
                            .button("Admin")
                            .on_hover_text("Open role-protected admin settings for this dataset.")
                            .clicked()
                    {
                        self.config.dataset_id = dataset.dataset_id.clone();
                        self.request_admin_dataset();
                    }
                });
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
        ui.selectable_value(&mut self.view, AppView::Setup, "Setup");
        ui.selectable_value(&mut self.view, AppView::Annotate, "Work");
        ui.selectable_value(&mut self.view, AppView::Stats, "Stats");
        if self.can_admin()
            && ui
                .button("Admin")
                .on_hover_text("Open admin settings. Requires DataAdmin role.")
                .clicked()
        {
            self.request_admin_dataset();
        }
    }

    pub(crate) fn can_admin(&self) -> bool {
        self.datasets.summaries.iter().any(|dataset| {
            dataset.dataset_id == self.config.dataset_id
                && dataset.roles.contains(&DatasetRole::DataAdmin)
        }) || self.datasets.metadata.as_ref().is_some_and(|metadata| {
            metadata.role_assignments.iter().any(|assignment| {
                assignment.user_id == self.config.user_id
                    && assignment.roles.contains(&DatasetRole::DataAdmin)
            })
        })
    }
}

fn roles_text(roles: &[DatasetRole]) -> String {
    if roles.is_empty() {
        "none".to_string()
    } else {
        roles
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}
