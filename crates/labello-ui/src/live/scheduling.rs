impl LabelloApp {
    pub(crate) fn refresh_stats_if_due(&mut self) {
        if self.runtime.api.is_none()
            || self.datasets.metadata.is_none()
            || self.loading.stats
            || matches!(self.view, AppView::Setup)
            || !matches!(self.view, AppView::Stats)
        {
            return;
        }
        let due = self
            .datasets
            .last_stats_attempt
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(3));
        if due {
            self.request_stats();
        }
    }

    pub(crate) fn refresh_assignment_availability_if_due(&mut self) {
        if self.work.availability.loading {
            return;
        }
        let Some(kind) = self.assignment_kind() else {
            return;
        };
        let context_changed = self.work.availability.dataset_id.as_ref()
            != Some(&self.config.dataset_id)
            || self.work.availability.kind.as_ref() != Some(&kind);
        let due = context_changed
            || if self.work.availability.checked_at.is_some() {
                self.assignment_availability_cache_age()
                    .is_none_or(|age| age >= ASSIGNMENT_AVAILABILITY_CACHE_TTL)
            } else {
                self.work
                    .availability
                    .last_attempt
                    .is_none_or(|last| last.elapsed() >= ASSIGNMENT_AVAILABILITY_CACHE_TTL)
            };
        if due {
            self.request_assignment_availability();
        }
    }

    pub(crate) fn request_assignment_availability(&mut self) {
        let Some(kind) = self.assignment_kind() else {
            return;
        };
        if self.runtime.api.is_none() {
            return;
        }
        if self.work.availability.loading {
            self.work.availability.refresh_after_load = true;
            self.work.availability.last_attempt = None;
            return;
        }
        let dataset_id = self.config.dataset_id.clone();
        if self.work.availability.dataset_id.as_ref() != Some(&dataset_id)
            || self.work.availability.kind.as_ref() != Some(&kind)
        {
            self.work.availability.tasks.clear();
            self.work.availability.resolved = false;
            self.work.availability.checked_at = None;
            self.work.availability.error = None;
        }
        self.work.availability.dataset_id = Some(dataset_id.clone());
        self.work.availability.kind = Some(kind.clone());
        self.work.availability.loading = true;
        self.work.availability.last_attempt = Some(Instant::now());
        let request = self.request_identity(Some(dataset_id.clone()));
        self.queue_command(UiCommand::AssignmentAvailability {
            request,
            dataset_id,
            kind,
        });
    }

    pub(crate) fn refresh_ingest_if_due(&mut self) {
        if !self.loading.ingesting || self.loading.ingest_polling {
            return;
        }
        let Some(job_id) = self.loading.ingest_job_id.clone() else {
            return;
        };
        let due = self
            .loading
            .last_ingest_poll
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(500));
        if due {
            self.loading.ingest_polling = true;
            let request = self.request_identity(Some(self.config.dataset_id.clone()));
            self.queue_command(UiCommand::PollIngest {
                request,
                dataset_id: self.config.dataset_id.clone(),
                job_id,
            });
        }
    }

}
