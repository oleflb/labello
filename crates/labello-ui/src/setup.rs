use eframe::egui::{self, RichText};
use labello_domain::{DatasetId, DatasetRole, UserId};

use crate::{
    app::{AppView, LabelloApp, PendingTransition},
    theme,
};

impl LabelloApp {
    pub(crate) fn setup_view(&mut self, ui: &mut egui::Ui) {
        let signed_in = self.auth.account.is_some();
        ui.vertical_centered(|ui| {
            let (title, subtitle) = if signed_in {
                (
                    "Choose where to work",
                    "Continue with a recommended dataset or choose where to work.",
                )
            } else {
                (
                    "Welcome to Labello",
                    "Sign in, then open a dataset available to your account.",
                )
            };
            ui.heading(RichText::new(title).size(28.0));
            ui.label(RichText::new(subtitle).color(theme::MUTED));
        });
        ui.add_space(18.0);

        if signed_in {
            self.datasets_section(ui);
            ui.add_space(12.0);
            egui::CollapsingHeader::new("Advanced connection settings")
                .show(ui, |ui| self.connection_section(ui));
            ui.add_space(12.0);
            egui::CollapsingHeader::new("Create a dataset")
                .show(ui, |ui| self.create_dataset_section(ui));
        } else {
            self.connection_section(ui);
            ui.add_space(12.0);
            self.datasets_section(ui);
        }
    }

    fn connection_section(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Connection");
            form_row(ui, "API URL", |ui| {
                ui.add_sized(
                    [ui.available_width(), 44.0],
                    egui::TextEdit::singleline(&mut self.config.api_base_url),
                )
                .on_hover_text("Backend API base URL, for example http://127.0.0.1:8080.")
            });
            if self.loading.session {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking your session...");
                });
            } else if let Some(account) = &self.auth.account {
                ui.label(RichText::new(&account.display_name).strong());
                if let Some(login) = &account.github_login {
                    ui.small(format!("GitHub: @{login}"));
                }
                ui.small(format!("User ID: {}", account.user_id));
            } else if !self.setup.dev_auth && ui.button("Sign in with GitHub").clicked() {
                self.request_github_login();
            }

            ui.separator();
            ui.checkbox(&mut self.setup.dev_auth, "Development authentication")
                .on_hover_text(
                    "Use explicit local user headers only when server dev auth is enabled.",
                );
            if self.setup.dev_auth {
                form_row(ui, "Dev token", |ui| {
                    ui.add_sized(
                        [ui.available_width(), 44.0],
                        egui::TextEdit::singleline(&mut self.config.dev_token).password(true),
                    )
                    .on_hover_text("Must match the development authentication token on the server.")
                });
                let mut user_id = self.config.user_id.to_string();
                form_row(ui, "Development user ID", |ui| {
                    let response = ui.add_sized(
                        [ui.available_width(), 44.0],
                        egui::TextEdit::singleline(&mut user_id),
                    );
                    if response.changed() {
                        self.config.user_id = UserId::from(user_id);
                    }
                    response
                });
                if ui.button("Apply development login").clicked() {
                    if let Err(error) = self.config.user_id.validate_path_segment() {
                        self.runtime.error = Some(format!("User ID: {error}"));
                    } else {
                        self.auth.account = None;
                        self.current = None;
                        self.datasets.metadata = None;
                        self.datasets.admin_config = None;
                        self.datasets.admin_baseline = None;
                        self.datasets.summaries.clear();
                        self.rebuild_http_api();
                        self.request_session();
                    }
                }
            }
        });
    }

    fn datasets_section(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.heading("Datasets");
                if self.loading.datasets || self.loading.dataset {
                    ui.spinner();
                }
                if self.loading.dataset {
                    ui.small("Opening dataset...");
                }
                if self.auth.account.is_some()
                    && ui
                        .button("Refresh")
                        .on_hover_text("Reload the accessible dataset list.")
                        .clicked()
                {
                    self.request_dataset_list();
                }
            });
            if self.auth.account.is_some() && self.datasets.summaries.is_empty() {
                ui.label(RichText::new("No accessible datasets yet.").color(theme::MUTED));
            } else if self.auth.account.is_none() {
                ui.label(RichText::new("Sign in to view datasets.").color(theme::MUTED));
            }
            let datasets = self.datasets.summaries.clone();
            if let Some(dataset) = self.recommended_dataset()
                && ui
                    .add_sized(
                        [280.0_f32.min(ui.available_width()), 44.0],
                        egui::Button::new(format!("Continue with {}", dataset.name)),
                    )
                    .on_hover_text("Open this dataset and its recommended work queue.")
                    .clicked()
            {
                let view = recommended_view(&dataset.roles);
                self.open_dataset(dataset.dataset_id, view);
            }
            for dataset in datasets {
                theme::card_frame().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
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
    }

    fn create_dataset_section(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            form_row(ui, "Dataset ID", |ui| {
                ui.add_sized(
                    [ui.available_width(), 44.0],
                    egui::TextEdit::singleline(&mut self.setup.create_dataset_id),
                )
                .on_hover_text("Stable identifier used for this dataset.")
            });
            form_row(ui, "Dataset name", |ui| {
                ui.add_sized(
                    [ui.available_width(), 44.0],
                    egui::TextEdit::singleline(&mut self.setup.create_dataset_name),
                )
                .on_hover_text("Human-readable dataset name.")
            });
            let dataset_id = DatasetId::from(self.setup.create_dataset_id.trim());
            let id_error = dataset_id.validate_path_segment().err();
            let can_create = id_error.is_none()
                && !self.setup.create_dataset_name.trim().is_empty()
                && !self.loading.dataset;
            if ui
                .add_enabled(can_create, egui::Button::new("Create dataset"))
                .on_hover_text("Requires bootstrap administrator access.")
                .clicked()
            {
                self.request_create_dataset();
            }
            if let Some(error) = id_error
                && !self.setup.create_dataset_id.trim().is_empty()
            {
                ui.label(RichText::new(format!("Dataset ID: {error}")).color(theme::RED));
            }
            ui.small("Only bootstrap administrators can create datasets.");
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

    pub(crate) fn compact_navigation(&mut self, ui: &mut egui::Ui) {
        let current = match self.view {
            AppView::Setup => "Setup",
            AppView::Annotate => "Annotate",
            AppView::Review => "Review",
            AppView::Adjudicate => "Adjudicate",
            AppView::Admin => "Admin",
            AppView::Stats => "Stats",
        };
        ui.menu_button(format!("View: {current}"), |ui| {
            let mut destination = None;
            if ui.button("Setup").clicked() {
                destination = Some(AppView::Setup);
            }
            for (view, role, label) in [
                (AppView::Annotate, DatasetRole::Annotator, "Annotate"),
                (AppView::Review, DatasetRole::Reviewer, "Review"),
                (AppView::Adjudicate, DatasetRole::Adjudicator, "Adjudicate"),
            ] {
                if self.has_dataset_role(role) && ui.button(label).clicked() {
                    destination = Some(view);
                }
            }
            if self.datasets.metadata.is_some() && ui.button("Stats").clicked() {
                destination = Some(AppView::Stats);
            }
            if self.can_admin() && ui.button("Admin").clicked() {
                destination = Some(AppView::Admin);
            }
            if let Some(view) = destination {
                self.open_view(view);
                ui.close();
            }
        });
    }

    pub(crate) fn open_view(&mut self, view: AppView) {
        if self.view == AppView::Admin
            && view != AppView::Admin
            && self.datasets.users != self.datasets.users_baseline
        {
            self.runtime.error = Some(
                "Save or revert permission changes in People before leaving Admin.".to_string(),
            );
            return;
        }
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
        if self.view == AppView::Admin && self.datasets.users != self.datasets.users_baseline {
            self.runtime.error = Some(
                "Save or revert permission changes in People before switching datasets."
                    .to_string(),
            );
            return;
        }
        if self.loading.dataset {
            return;
        }
        if self.config.dataset_id != dataset_id {
            self.loading.stats = false;
            self.datasets.active_stats_request = None;
            self.datasets.last_stats_attempt = None;
            self.datasets.last_stats_completion = None;
            self.datasets.stats_error = None;
            self.datasets.stats = labello_domain::DatasetStats::default();
        }
        self.config.dataset_id = dataset_id;
        self.datasets.requested_view = Some(view);
        self.request_load_dataset();
    }

    pub(crate) fn can_admin(&self) -> bool {
        self.has_dataset_role(DatasetRole::DataAdmin)
    }

    fn recommended_dataset(&self) -> Option<labello_client::DatasetSummary> {
        let remembered = self
            .runtime
            .persistence
            .preference
            .as_ref()
            .map(|preference| preference.dataset_id.clone());
        remembered
            .and_then(|id| {
                self.datasets
                    .summaries
                    .iter()
                    .find(|item| item.dataset_id == id)
            })
            .or_else(|| self.datasets.summaries.first())
            .cloned()
    }
}

fn form_row(
    ui: &mut egui::Ui,
    label: &str,
    add_field: impl FnOnce(&mut egui::Ui) -> egui::Response,
) {
    if ui.available_width() < 520.0 {
        ui.vertical(|ui| {
            let label = ui.label(label);
            add_field(ui).labelled_by(label.id);
        });
    } else {
        ui.horizontal(|ui| {
            let label = ui.add_sized([150.0, 44.0], egui::Label::new(label));
            add_field(ui).labelled_by(label.id);
        });
    }
}

fn recommended_view(roles: &[DatasetRole]) -> AppView {
    if roles.contains(&DatasetRole::Annotator) {
        AppView::Annotate
    } else if roles.contains(&DatasetRole::Reviewer) {
        AppView::Review
    } else if roles.contains(&DatasetRole::Adjudicator) {
        AppView::Adjudicate
    } else {
        AppView::Stats
    }
}

fn role_badge(ui: &mut egui::Ui, role: &DatasetRole) {
    egui::Frame::new()
        .fill(theme::BLUE.gamma_multiply(0.16))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            let label = match role {
                DatasetRole::Annotator => "Annotator",
                DatasetRole::Reviewer => "Reviewer",
                DatasetRole::Adjudicator => "Adjudicator",
                DatasetRole::DataAdmin => "Data admin",
            };
            ui.label(RichText::new(label).color(theme::BLUE));
        });
}
