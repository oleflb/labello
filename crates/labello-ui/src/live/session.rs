impl LabelloApp {
    pub(crate) fn start_setup_load(&mut self) {
        if self.runtime.api.is_some() && !self.auth.options_checked && !self.loading.session {
            self.request_auth_options();
        } else if self.runtime.api.is_some()
            && self.auth.options_checked
            && !self.auth.checked
            && !self.loading.session
        {
            self.request_session();
        }
    }

    fn clear_authenticated_state(&mut self) {
        self.begin_import_epoch();
        self.import = Default::default();
        self.auth.account = None;
        self.auth.can_create_datasets = false;
        self.datasets.summaries.clear();
        self.datasets.summaries_error = None;
        self.datasets.metadata = None;
        self.datasets.admin_config = None;
        self.datasets.admin_baseline = None;
        self.datasets.users.clear();
        self.datasets.users_baseline.clear();
        self.datasets.stats = Default::default();
        self.datasets.active_stats_request = None;
        self.datasets.last_stats_attempt = None;
        self.datasets.last_stats_completion = None;
        self.datasets.stats_error = None;
        self.datasets.requested_view = None;
        self.admin = Default::default();
        self.work.drawer = None;
        self.work.workflow_panel_collapsed = false;
        self.work.show_tutorial = false;
        self.work.shortcut_settings = Default::default();
        self.work.keybindings =
            labello_domain::KeybindingSet::defaults_for(self.config.user_id.clone());
        self.work.previous_annotation_assignment = None;
        self.clear_current_image();
        self.isolate_browser_workspace();
        self.runtime.storage_error = None;
        self.runtime.notice = None;
        self.view = AppView::Setup;
    }

    pub(crate) fn request_auth_options(&mut self) {
        if self.runtime.api.is_none() {
            return;
        }
        self.begin_auth_epoch();
        let request = self.request_identity(None);
        self.auth.options = labello_client::AuthOptions {
            github_oauth: false,
            local_admin_login: false,
        };
        self.auth.options_checked = false;
        self.auth.checked = false;
        self.loading.session = true;
        self.queue_command(UiCommand::AuthOptions { request });
    }

    pub(crate) fn request_logout(&mut self) {
        if self.loading.logout || self.runtime.api.is_none() {
            return;
        }
        if self.view == AppView::Admin && self.admin_changes_dirty() {
            self.runtime.error =
                Some("Save or discard staged Admin changes before signing out.".to_string());
            return;
        }
        self.clear_previous_annotation_assignment();
        self.begin_auth_epoch();
        self.loading.logout = true;
        let request = self.request_identity(None);
        self.queue_command(UiCommand::Logout { request });
    }

    pub(crate) fn request_github_login(&mut self) {
        if self.runtime.api.is_some() {
            let request = self.request_identity(None);
            self.queue_command(UiCommand::GithubLogin {
                request,
                return_to: self.config.application_url.clone(),
            });
        }
    }

    pub(crate) fn request_session(&mut self) {
        if self.runtime.api.is_none() {
            return;
        }
        self.begin_auth_epoch();
        let request = self.request_identity(None);
        self.auth.session_request_id = request.request_id;
        self.auth.active_session_request_id = Some(request.request_id);
        self.auth.local_admin_login_pending = false;
        self.auth.checked = false;
        self.loading.session = true;
        self.queue_command(UiCommand::Session { request });
    }

    pub(crate) fn request_local_admin_login(&mut self) {
        if self.loading.session || self.runtime.api.is_none() {
            return;
        }
        self.begin_auth_epoch();
        let request = self.request_identity(None);
        self.auth.session_request_id = request.request_id;
        self.auth.active_session_request_id = Some(request.request_id);
        self.auth.local_admin_login_pending = true;
        self.auth.checked = false;
        self.loading.session = true;
        self.queue_command(UiCommand::LocalAdminLogin { request });
    }

}
