use eframe::egui::{self, RichText};
use labello_domain::{DatasetId, DatasetRole};

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
            ui.heading(RichText::new(title).size(theme::PAGE_TITLE_SIZE));
            ui.label(RichText::new(subtitle).color(theme::TEXT_MUTED));
        });
        ui.add_space(theme::SPACE_4);

        if signed_in {
            self.datasets_section(ui);
            ui.add_space(theme::SPACE_5);
            egui::CollapsingHeader::new("Advanced connection settings")
                .show(ui, |ui| self.connection_section(ui));
            if self.auth.can_create_datasets {
                ui.add_space(theme::SPACE_3);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_sized(
                            [ui.available_width().min(260.0), 44.0],
                            egui::Button::selectable(self.setup.show_create, "Create a dataset"),
                        )
                        .clicked()
                    {
                        self.setup.show_create = !self.setup.show_create;
                        self.import_flow.open = false;
                    }
                });
                if self.setup.show_create {
                    self.create_dataset_section(ui);
                }
                ui.add_space(theme::SPACE_2);
                self.import_setup_section(ui);
            }
        } else {
            self.connection_section(ui);
            ui.add_space(theme::SPACE_4);
            self.datasets_section(ui);
        }
    }

    fn connection_section(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Connection");
            let response =
                theme::labeled_text_field(ui, "API URL", &mut self.setup.api_base_url_draft, 24.0)
                    .on_hover_text("Backend API base URL, for example http://127.0.0.1:8080.");
            let mut reconnect =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            reconnect |= theme::quiet_button(
                ui,
                self.setup.api_base_url_draft != self.config.api_base_url,
                egui::Button::new("Reconnect").min_size(egui::vec2(96.0, 44.0)),
            )
            .clicked();
            if reconnect && self.setup.api_base_url_draft != self.config.api_base_url {
                self.config.api_base_url = self.setup.api_base_url_draft.clone();
                self.rebuild_http_api();
                self.auth.options_checked = false;
                self.auth.checked = false;
            }
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
            } else {
                ui.horizontal_wrapped(|ui| {
                    if self.auth.options.local_admin_login
                        && theme::quiet_button(
                            ui,
                            true,
                            egui::Button::new("Continue as local admin"),
                        )
                        .clicked()
                    {
                        self.request_local_admin_login();
                    }
                    if self.auth.options.github_oauth
                        && theme::primary_button(ui, true, egui::Button::new("Sign in with GitHub"))
                            .clicked()
                    {
                        self.request_github_login();
                    }
                });
                if self.auth.options_checked
                    && !self.auth.options.local_admin_login
                    && !self.auth.options.github_oauth
                {
                    ui.label("No interactive sign-in method is enabled on this server.");
                }
            }
        });
    }

    fn datasets_section(&mut self, ui: &mut egui::Ui) {
        let signed_in = self.auth.account.is_some();
        if !self.auth.checked {
            ui.heading("Datasets");
            if self.runtime.api.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Checking dataset access...").color(theme::TEXT_MUTED));
                });
            } else {
                theme::empty_state(
                    ui,
                    "Connection unavailable",
                    "Enter a valid API URL to check dataset access.",
                    None,
                );
            }
            return;
        }
        let has_datasets = !self.datasets.summaries.is_empty();
        let summaries_error = self.datasets.summaries_error.clone();
        let datasets = self.datasets.summaries.clone();
        let recommended = if signed_in {
            self.recommended_dataset()
        } else {
            None
        };

        if let Some(dataset) = recommended.as_ref() {
            self.recommended_dataset_card(ui, dataset);
            ui.add_space(theme::SPACE_5);
        }

        ui.horizontal(|ui| {
            ui.heading(if signed_in && has_datasets {
                "All datasets"
            } else {
                "Datasets"
            });
            if signed_in {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::quiet_button(
                        ui,
                        !self.loading.datasets && !self.loading.dataset,
                        egui::Button::new("Refresh"),
                    )
                    .on_hover_text("Reload the accessible dataset list.")
                    .clicked()
                    {
                        self.request_dataset_list();
                    }
                });
            }
        });
        if self.loading.dataset || (self.loading.datasets && has_datasets) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small(if self.loading.dataset {
                    "Opening dataset..."
                } else {
                    "Refreshing..."
                });
            });
        }

        if !signed_in {
            theme::empty_state(
                ui,
                "Sign in to view datasets",
                "Available datasets will appear here after you sign in.",
                None,
            );
        } else if self.loading.datasets && !has_datasets {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Loading datasets...").color(theme::TEXT_MUTED));
            });
        } else if let Some(error) = summaries_error.as_ref()
            && !has_datasets
        {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                format!("Could not load datasets: {error}"),
            );
            if theme::quiet_button(
                ui,
                !self.loading.datasets && !self.loading.dataset,
                egui::Button::new("Retry"),
            )
            .clicked()
            {
                self.request_dataset_list();
            }
        } else if !has_datasets {
            let description = if self.auth.can_create_datasets {
                "Create a dataset below or ask a data admin for access."
            } else {
                "Ask a data admin for access."
            };
            theme::empty_state(ui, "No accessible datasets yet.", description, None);
        }

        if let Some(error) = summaries_error
            && has_datasets
        {
            ui.horizontal_wrapped(|ui| {
                theme::inline_message(
                    ui,
                    theme::Intent::Warning,
                    format!("Showing saved results. Refresh failed: {error}"),
                );
                if theme::quiet_button(
                    ui,
                    !self.loading.datasets && !self.loading.dataset,
                    egui::Button::new("Retry"),
                )
                .clicked()
                {
                    self.request_dataset_list();
                }
            });
        }

        for dataset in datasets {
            let recommended_destination = recommended
                .as_ref()
                .filter(|item| item.dataset_id == dataset.dataset_id)
                .map(|item| recommended_view(&item.roles));
            let card_label = format!("Dataset card {}", dataset.name);
            let response = theme::card_frame().show(ui, |ui| {
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
                        if dataset.roles.contains(&role)
                            && recommended_destination != Some(view)
                            && dataset_action(ui, !self.loading.dataset, label, &dataset.name)
                        {
                            self.open_dataset(dataset.dataset_id.clone(), view);
                        }
                    }
                    if dataset.roles.contains(&DatasetRole::DataAdmin)
                        && recommended_destination != Some(AppView::Admin)
                        && dataset_action(ui, !self.loading.dataset, "Admin", &dataset.name)
                    {
                        self.open_dataset(dataset.dataset_id.clone(), AppView::Admin);
                    }
                    if !dataset.roles.is_empty()
                        && recommended_destination != Some(AppView::Stats)
                        && dataset_action(ui, !self.loading.dataset, "Stats", &dataset.name)
                    {
                        self.open_dataset(dataset.dataset_id.clone(), AppView::Stats);
                    }
                });
            });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, true, card_label.clone())
            });
            ui.add_space(theme::SPACE_2);
        }
    }

    fn recommended_dataset_card(
        &mut self,
        ui: &mut egui::Ui,
        dataset: &labello_client::DatasetSummary,
    ) {
        ui.heading("Recommended");
        let label = format!("Recommended dataset {}", dataset.name);
        let description = if recommended_view(&dataset.roles) == AppView::Stats {
            "View statistics for this dataset."
        } else {
            "Open the suggested work queue for this dataset."
        };
        let response = theme::selected_card_frame(true).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(&dataset.name)
                    .size(theme::SECTION_HEADING_SIZE)
                    .strong(),
            );
            ui.label(RichText::new(description).color(theme::TEXT_MUTED));
            let width = 320.0_f32.min(ui.available_width());
            if theme::primary_button(
                ui,
                !self.loading.dataset,
                egui::Button::new(format!("Continue with {}", dataset.name))
                    .min_size(egui::vec2(width, 44.0))
                    .truncate(),
            )
            .on_hover_text(description)
            .clicked()
            {
                self.open_dataset(dataset.dataset_id.clone(), recommended_view(&dataset.roles));
            }
        });
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, label.clone())
        });
    }

    fn create_dataset_section(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::labeled_text_field(
                ui,
                "Dataset ID",
                &mut self.setup.create_dataset_id,
                theme::COMPACT_TEXT_FIELD_HEIGHT,
            )
            .on_hover_text("Stable identifier used for this dataset.");
            theme::labeled_text_field(
                ui,
                "Dataset name",
                &mut self.setup.create_dataset_name,
                theme::COMPACT_TEXT_FIELD_HEIGHT,
            )
            .on_hover_text("Human-readable dataset name.");
            let dataset_id = DatasetId::from(self.setup.create_dataset_id.trim());
            let id_error = dataset_id.validate_path_segment().err();
            let can_create = id_error.is_none()
                && !self.setup.create_dataset_name.trim().is_empty()
                && !self.loading.dataset;
            if theme::primary_button(ui, can_create, egui::Button::new("Create dataset"))
                .on_hover_text("Requires bootstrap administrator access.")
                .clicked()
            {
                self.request_create_dataset();
            }
            if let Some(error) = id_error
                && !self.setup.create_dataset_id.trim().is_empty()
            {
                ui.label(RichText::new(format!("Dataset ID: {error}")).color(theme::DANGER));
            }
            ui.small("Only bootstrap administrators can create datasets.");
        });
    }

    pub(crate) fn navigation_menu_contents(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(theme::MENU_WIDTH);
        for (view, label) in self.navigation_destinations() {
            if ui
                .add(
                    egui::Button::selectable(self.view == view, label)
                        .min_size(egui::vec2(theme::MENU_WIDTH, 44.0)),
                )
                .clicked()
            {
                self.open_view(view);
                ui.close();
            }
        }
    }

    pub(crate) fn desktop_navigation(&mut self, ui: &mut egui::Ui) {
        let account = self
            .auth
            .account
            .as_ref()
            .map(|account| account.display_name.clone());
        let footer_height = if account.is_some() { 132.0 } else { 0.0 };
        let footer_gap = if account.is_some() {
            ui.spacing().item_spacing.y
        } else {
            0.0
        };
        let navigation_height = (ui.available_height() - footer_height - footer_gap).max(0.0);
        let response = ui.vertical(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), navigation_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(navigation_height);
                    ui.set_max_height(navigation_height);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.heading("Navigate");
                        ui.add_space(theme::SPACE_1);
                        for (view, label) in self.navigation_destinations() {
                            if ui
                                .add_sized(
                                    [ui.available_width(), 44.0],
                                    egui::Button::selectable(self.view == view, label),
                                )
                                .clicked()
                            {
                                self.open_view(view);
                            }
                        }
                    });
                },
            );

            if let Some(account) = account {
                ui.label(RichText::new("Account").strong().color(theme::TEXT_MUTED));
                ui.separator();
                ui.add_sized(
                    [ui.available_width(), 44.0],
                    egui::Label::new(&account).truncate(),
                );
                if theme::quiet_button(
                    ui,
                    !self.loading.logout,
                    egui::Button::new("Sign out").min_size(egui::vec2(ui.available_width(), 44.0)),
                )
                .clicked()
                {
                    self.request_logout();
                }
            }
        });
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Desktop navigation")
        });
    }

    fn navigation_destinations(&self) -> Vec<(AppView, &'static str)> {
        let mut destinations = vec![(AppView::Setup, "Setup")];
        for (view, role, label) in [
            (AppView::Annotate, DatasetRole::Annotator, "Annotate"),
            (AppView::Review, DatasetRole::Reviewer, "Review"),
            (AppView::Adjudicate, DatasetRole::Adjudicator, "Adjudicate"),
        ] {
            if self.has_dataset_role(role) {
                destinations.push((view, label));
            }
        }
        if self.datasets.metadata.is_some() {
            destinations.push((AppView::Stats, "Stats"));
        }
        if self.can_admin() {
            destinations.push((AppView::Admin, "Admin"));
        }
        destinations
    }

    pub(crate) fn open_view(&mut self, view: AppView) {
        if self.view == AppView::Admin && view != AppView::Admin && self.admin_changes_dirty() {
            self.runtime.error =
                Some("Save or discard staged Admin changes before leaving Admin.".to_string());
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
        if self.view == AppView::Admin && self.admin_changes_dirty() {
            self.runtime.error =
                Some("Save or discard staged Admin changes before switching datasets.".to_string());
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
                    .find(|item| item.dataset_id == id && !item.roles.is_empty())
            })
            .or_else(|| {
                self.datasets
                    .summaries
                    .iter()
                    .find(|item| !item.roles.is_empty())
            })
            .cloned()
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
    let label = match role {
        DatasetRole::Annotator => "Annotator",
        DatasetRole::Reviewer => "Reviewer",
        DatasetRole::Adjudicator => "Adjudicator",
        DatasetRole::DataAdmin => "Data admin",
    };
    theme::badge(ui, label, theme::Intent::Info);
}

fn dataset_action(ui: &mut egui::Ui, enabled: bool, label: &str, dataset_name: &str) -> bool {
    let accessible_label = format!("{label} {dataset_name}");
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, accessible_label.clone())
    });
    response.clicked()
}
