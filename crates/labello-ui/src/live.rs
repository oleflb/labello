use std::{future::Future, rc::Rc};

use eframe::egui;
use labello_client::{
    HttpLabelloApi, IngestJob, IngestJobStatus, OAuthLoginRequest, SetDatasetRolesRequest,
    UpdateDatasetConfigRequest,
};
use web_time::{Duration, Instant};

use crate::app::{
    ASSIGNMENT_AVAILABILITY_CACHE_TTL, AppView, ImportRequestIdentity, LabelloApp, LoadedAdmin,
    LoadedDataset, LoadedImage, RequestIdentity, SaveStatus, SetupSection, UiCommand, UiMessage,
};

impl LabelloApp {
    pub(crate) fn rebuild_http_api(&mut self) {
        self.begin_auth_epoch();
        self.begin_import_epoch();
        self.import = Default::default();
        let api = HttpLabelloApi::new(&self.config.api_base_url).and_then(|api| {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(application_url) = &self.config.application_url {
                return api.with_origin(application_url);
            }
            Ok(api)
        });
        match api {
            Ok(api) => {
                self.runtime.api = Some(Rc::new(api));
                self.runtime.error = None;
            }
            Err(error) => {
                self.runtime.api = None;
                self.runtime.error = Some(error.to_string());
            }
        }
    }

    pub(crate) fn process_messages(&mut self, ctx: &egui::Context) {
        self.runtime.repaint_ctx = Some(ctx.clone());
        let mut processed = 0;
        for _ in 0..8 {
            let Ok(message) = self.runtime.rx.try_recv() else {
                break;
            };
            processed += 1;
            if let Some(request) = message.import_request().cloned()
                && !self.finish_import_request(&request)
            {
                continue;
            }
            let requires_current_dataset = !matches!(&message, UiMessage::DatasetCreated { .. });
            if let Some(request) = message.request().cloned()
                && !self.finish_request(&request, requires_current_dataset)
            {
                if let Some(dataset_id) = request.dataset_id {
                    match message {
                        UiMessage::PrefetchLoaded { result, .. } => {
                            if let Ok(Some(loaded)) = *result {
                                self.release_reservation(dataset_id, loaded.assignment);
                            }
                        }
                        UiMessage::ImageLoaded {
                            assignment: Some(assignment),
                            ..
                        } => self.release_reservation(dataset_id, assignment),
                        UiMessage::PreviousAssignmentLoaded {
                            assignment: Some(assignment),
                            ..
                        } => self.release_reservation(dataset_id, assignment),
                        UiMessage::PreparedReviewRevalidated { cached, .. } => {
                            self.release_reservation(dataset_id, cached.assignment)
                        }
                        _ => {}
                    }
                }
                continue;
            }
            let message = match self.reduce_import_message(ctx, message) {
                None => continue,
                Some(message) => message,
            };
            let message = match self.reduce_session_message(ctx, message) {
                None => continue,
                Some(message) => message,
            };
            let message = match self.reduce_workflow_message(ctx, message) {
                None => continue,
                Some(message) => message,
            };
            if let Some(message) = self.reduce_support_message(ctx, message) {
                unreachable!("unhandled UI message: {message:?}");
            }
        }
        if processed == 8 {
            ctx.request_repaint();
        }
    }

    pub(crate) fn start_next_command(&mut self) {
        let Some(command) = self.runtime.commands.pop_front() else {
            return;
        };
        let Some(api) = self.runtime.api.clone() else {
            self.rollback_command(&command, "API is not configured");
            return;
        };
        let command = match self.dispatch_import_command(api.clone(), command) {
            None => return,
            Some(command) => command,
        };
        let command = match self.dispatch_migration_command(api.clone(), command) {
            None => return,
            Some(command) => command,
        };
        let command = match self.dispatch_auth_command(api.clone(), command) {
            None => return,
            Some(command) => command,
        };
        let command = match self.dispatch_dataset_command(api.clone(), command) {
            None => return,
            Some(command) => command,
        };
        let command = match self.dispatch_support_command(api.clone(), command) {
            None => return,
            Some(command) => command,
        };
        self.start_workflow_command(api, command);
    }
}

include!("live/session.rs");
include!("live/scheduling.rs");
include!("live/ownership.rs");
include!("live/dataset.rs");
include!("live/workflow_state.rs");
include!("live/spawn.rs");
include!("live/reduce_import.rs");
include!("live/reduce_session.rs");
include!("live/reduce_workflow.rs");
include!("live/reduce_support.rs");
include!("live/dispatch_import.rs");
include!("live/dispatch_migration.rs");
include!("live/dispatch_auth.rs");
include!("live/dispatch_dataset.rs");
include!("live/dispatch_support.rs");

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn replace_dataset_user(
    users: &mut Vec<labello_client::DatasetUser>,
    updated: labello_client::DatasetUser,
) {
    if let Some(user) = users
        .iter_mut()
        .find(|user| user.account.user_id == updated.account.user_id)
    {
        *user = updated;
    } else {
        users.push(updated);
    }
}

fn view_label(view: AppView) -> &'static str {
    match view {
        AppView::Setup => "setup",
        AppView::Annotate => "annotation",
        AppView::Review => "review",
        AppView::Adjudicate => "adjudication",
        AppView::Admin => "administration",
        AppView::Stats => "statistics",
    }
}

#[cfg(all(not(target_arch = "wasm32"), test))]
fn poll_ready<F>(future: F) -> UiMessage
where
    F: Future<Output = UiMessage> + 'static,
{
    use std::{
        pin::Pin,
        task::{Context, Poll, Waker},
    };

    let mut future = Pin::from(Box::new(future));
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(message) => message,
        Poll::Pending => panic!("test fake API future did not complete immediately"),
    }
}
