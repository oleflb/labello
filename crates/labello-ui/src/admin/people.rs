impl LabelloApp {
    fn people_section(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        ui.heading("People");
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("{} users", self.datasets.users.len()))
                        .color(theme::INFO),
                );
            });
            ui.label(
                RichText::new("Grant dataset roles to people who have signed in to this server.")
                    .color(theme::TEXT_MUTED),
            );
            let search_label = ui.label("Search people");
            ui.add_sized(
                [
                    ui.available_width().min(480.0),
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                ],
                theme::singleline_text_edit(&mut self.admin.people_search)
                    .hint_text("Name, login, or user ID"),
            )
            .labelled_by(search_label.id);
            if self.loading.admin && self.datasets.users.is_empty() {
                ui.spinner();
                return;
            }
            let search = self.admin.people_search.trim().to_lowercase();
            let current_user = self.config.user_id.clone();
            let admin_loading = self.loading.admin
                || self.loading.roles_user.is_some()
                || self.loading.uploading
                || self.loading.ingesting;
            let baseline = self.datasets.users_baseline.clone();
            let admin_count = self
                .datasets
                .users
                .iter()
                .filter(|user| user.roles.contains(&DatasetRole::DataAdmin))
                .count();
            let saving = self.loading.roles_user.clone();
            let mut visible_users = 0;
            if layout == LayoutMode::Wide {
                egui::Grid::new("admin-people-grid")
                    .num_columns(3)
                    .striped(true)
                    .spacing([theme::SPACE_4, theme::SPACE_2])
                    .show(ui, |ui| {
                        for heading in ["Person", "Roles", "Status"] {
                            ui.label(RichText::new(heading).strong().color(theme::TEXT_MUTED));
                        }
                        ui.end_row();
                        for user in self
                            .datasets
                            .users
                            .iter_mut()
                            .filter(|user| user_matches_search(user, &search))
                        {
                            visible_users += 1;
                            ui.allocate_ui_with_layout(
                                egui::vec2(180.0, 76.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.vertical(|ui| {
                                        ui.add_space(theme::SPACE_1);
                                        user_identity(ui, user);
                                    });
                                },
                            );
                            ui.allocate_ui_with_layout(
                                egui::vec2(420.0, 76.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                                    edit_user_roles(
                                        ui,
                                        user,
                                        &current_user,
                                        admin_count,
                                        admin_loading,
                                    );
                                },
                            );
                            let dirty = user_permissions_dirty(user, &baseline);
                            let this_saving = saving.as_ref() == Some(&user.account.user_id);
                            ui.allocate_ui_with_layout(
                                egui::vec2(80.0, 76.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(if this_saving {
                                            "Saving"
                                        } else if dirty {
                                            "Staged"
                                        } else {
                                            "Saved"
                                        })
                                        .color(
                                            if dirty || this_saving {
                                                theme::WARNING
                                            } else {
                                                theme::SUCCESS
                                            },
                                        ),
                                    );
                                },
                            );
                            ui.end_row();
                        }
                    });
            } else {
                for user in self
                    .datasets
                    .users
                    .iter_mut()
                    .filter(|user| user_matches_search(user, &search))
                {
                    visible_users += 1;
                    ui.add_space(theme::SPACE_1);
                    let card_label = format!("Person card {}", user.account.display_name);
                    let card = theme::inset_frame().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        user_identity(ui, user);
                        ui.add_space(theme::SPACE_1);
                        ui.horizontal_wrapped(|ui| {
                            edit_user_roles(ui, user, &current_user, admin_count, admin_loading);
                        });
                        let dirty = user_permissions_dirty(user, &baseline);
                        ui.horizontal_wrapped(|ui| {
                            let this_saving = saving.as_ref() == Some(&user.account.user_id);
                            ui.label(
                                RichText::new(if this_saving {
                                    "Saving"
                                } else if dirty {
                                    "Changes staged"
                                } else {
                                    "Permissions saved"
                                })
                                .color(if dirty {
                                    theme::WARNING
                                } else {
                                    theme::TEXT_MUTED
                                }),
                            );
                        });
                    });
                    card.response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, card_label.clone())
                    });
                }
            }
            if visible_users == 0 && !self.datasets.users.is_empty() {
                theme::empty_state(
                    ui,
                    "No matching people",
                    "Change the search to show more accounts.",
                    None,
                );
            }
            if self.datasets.users.is_empty() && !self.loading.admin {
                theme::empty_state(
                    ui,
                    "No people yet",
                    "People appear here after they sign in to this server.",
                    None,
                );
            }
        });
    }
}

fn user_identity(ui: &mut egui::Ui, user: &DatasetUser) {
    ui.label(RichText::new(&user.account.display_name).strong());
    if let Some(login) = &user.account.github_login {
        ui.label(RichText::new(format!("@{login}")).color(theme::MUTED));
    }
    ui.small(format!("ID: {}", user.account.user_id));
}

fn user_matches_search(user: &DatasetUser, search: &str) -> bool {
    search.is_empty()
        || user.account.display_name.to_lowercase().contains(search)
        || user
            .account
            .user_id
            .as_str()
            .to_lowercase()
            .contains(search)
        || user
            .account
            .github_login
            .as_deref()
            .is_some_and(|login| login.to_lowercase().contains(search))
}

fn edit_user_roles(
    ui: &mut egui::Ui,
    user: &mut DatasetUser,
    current_user: &UserId,
    admin_count: usize,
    admin_loading: bool,
) {
    for (role, label) in [
        (DatasetRole::Annotator, "Annotator"),
        (DatasetRole::Reviewer, "Reviewer"),
        (DatasetRole::Adjudicator, "Adjudicator"),
        (DatasetRole::DataAdmin, "Data admin"),
    ] {
        let is_admin_role = role == DatasetRole::DataAdmin;
        let role_enabled = !admin_loading
            && !(is_admin_role
                && user.roles.contains(&role)
                && (&user.account.user_id == current_user || admin_count == 1));
        let mut enabled = user.roles.contains(&role);
        let response = ui
            .add_enabled(role_enabled, egui::Checkbox::new(&mut enabled, label))
            .on_disabled_hover_text(if &user.account.user_id == current_user {
                "You cannot remove your own data admin role."
            } else {
                "At least one data admin must remain."
            });
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                role_enabled,
                enabled,
                format!(
                    "{label} role for {} ({})",
                    user.account.display_name, user.account.user_id
                ),
            )
        });
        if response.changed() {
            if enabled {
                user.roles.push(role);
                user.roles.sort();
                user.roles.dedup();
            } else {
                user.roles.retain(|existing| existing != &role);
            }
        }
    }
}

fn user_permissions_dirty(user: &DatasetUser, baseline: &[DatasetUser]) -> bool {
    baseline
        .iter()
        .find(|existing| existing.account.user_id == user.account.user_id)
        .is_none_or(|existing| existing.roles != user.roles)
}

fn task_statuses() -> [TaskStatus; 6] {
    [
        TaskStatus::Pending,
        TaskStatus::InProgress,
        TaskStatus::Submitted,
        TaskStatus::Completed,
        TaskStatus::NeedsCorrection,
        TaskStatus::AdjudicationRequired,
    ]
}

fn task_status_label(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "Pending",
        TaskStatus::InProgress => "In progress",
        TaskStatus::Submitted => "Submitted",
        TaskStatus::Completed => "Completed",
        TaskStatus::NeedsCorrection => "Needs correction",
        TaskStatus::AdjudicationRequired => "Adjudication required",
    }
}

fn task_status_summary(statuses: &[TaskStatus]) -> String {
    let summary = task_statuses()
        .into_iter()
        .filter_map(|status| {
            let count = statuses
                .iter()
                .filter(|current| **current == status)
                .count();
            (count > 0).then(|| format!("{} {count}", task_status_label(&status)))
        })
        .collect::<Vec<_>>()
        .join(" | ");
    if summary.is_empty() {
        "No workflow status".to_string()
    } else {
        summary
    }
}
