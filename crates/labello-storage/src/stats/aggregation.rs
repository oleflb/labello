use super::*;

pub(super) struct StatsAggregation {
    stats: DatasetStats,
    throughput: BTreeMap<String, (usize, usize)>,
}

impl StatsAggregation {
    pub(super) fn new(metadata: &labello_domain::DatasetMetadata) -> Self {
        let stats = DatasetStats {
            total_images: metadata.images.len(),
            per_task: metadata
                .tasks
                .iter()
                .filter(|task| task.enabled)
                .map(|task| (task.task_id.clone(), TaskStats::default()))
                .collect(),
            per_class: metadata
                .label_classes
                .iter()
                .map(|class| (class.class_id.clone(), ClassStats::default()))
                .collect(),
            ..DatasetStats::default()
        };
        Self {
            stats,
            throughput: BTreeMap::new(),
        }
    }

    pub(super) fn record_image(
        &mut self,
        metadata: &labello_domain::DatasetMetadata,
        state: &ImageState,
    ) {
        let stats = &mut self.stats;
        let throughput = &mut self.throughput;
        for task in &metadata.tasks {
            if !task.enabled {
                continue;
            }
            let task_stats = stats.per_task.entry(task.task_id.clone()).or_default();
            if let Some(coverage) = state.import_coverage.get(&task.task_id) {
                match coverage {
                    ImportCoverage::Complete => stats.import_coverage.complete += 1,
                    ImportCoverage::VerifiedEmpty => {
                        stats.import_coverage.verified_empty += 1;
                    }
                    ImportCoverage::Incomplete => stats.import_coverage.incomplete += 1,
                    ImportCoverage::Excluded => stats.import_coverage.excluded += 1,
                }
            }
            if let Some(target_set) = state.migration_target_sets.get(&task.task_id) {
                stats.migration.expected += target_set.targets.len();
                task_stats.migration.expected += target_set.targets.len();
                for target in &target_set.targets {
                    match &state.migration_dispositions[&task.task_id][&target.object_group_id]
                        .status
                    {
                        MigrationDispositionStatus::Annotated { .. } => {
                            stats.migration.annotated += 1;
                            task_stats.migration.annotated += 1;
                        }
                        MigrationDispositionStatus::Excluded { .. } => {
                            stats.migration.excluded += 1;
                            task_stats.migration.excluded += 1;
                        }
                        MigrationDispositionStatus::Pending => {
                            stats.migration.pending += 1;
                            task_stats.migration.pending += 1;
                        }
                    }
                }
            }
            if !state.included_in_completion_denominator(&task.task_id) {
                continue;
            }
            match state
                .task_states
                .get(&task.task_id)
                .map(|state| &state.status)
                .unwrap_or(&TaskStatus::Pending)
            {
                TaskStatus::Completed => {
                    stats.completed_tasks += 1;
                    task_stats.completed += 1;
                    for class_id in &task.class_ids {
                        stats
                            .per_class
                            .entry(class_id.clone())
                            .or_default()
                            .completed_tasks += 1;
                    }
                }
                TaskStatus::Submitted => {
                    stats.unreviewed_tasks += 1;
                    task_stats.unreviewed += 1;
                }
                _ => {
                    stats.pending_tasks += 1;
                    task_stats.pending += 1;
                }
            }
            let reviewer_corrected =
                state
                    .task_states
                    .get(&task.task_id)
                    .is_some_and(|task_state| {
                        task_state.outcome == Some(TaskOutcome::ReviewerCorrected)
                    });
            let review_decision = (!reviewer_corrected)
                .then(|| current_task_review_decision(state, &task.task_id))
                .flatten();
            if review_decision == Some(ReviewDecision::Approved) {
                stats.reviewed_tasks += 1;
                task_stats.reviewed += 1;
            }
            match review_decision.as_ref() {
                Some(ReviewDecision::Approved) => {
                    stats.approved_tasks += 1;
                    task_stats.approved += 1;
                }
                Some(ReviewDecision::Rejected) => {
                    stats.rejected_tasks += 1;
                    task_stats.rejected += 1;
                }
                None => {}
            }
            if let Some(outcome) = state.task_states.get(&task.task_id).and_then(|task_state| {
                (task_state.status == TaskStatus::Completed)
                    .then_some(task_state.outcome.as_ref())
                    .flatten()
            }) {
                stats.finalized_tasks += 1;
                task_stats.finalized += 1;
                if outcome == &TaskOutcome::ReviewerCorrected {
                    stats.rejected_tasks += 1;
                    task_stats.rejected += 1;
                    stats.reviewer_corrected_tasks += 1;
                    task_stats.reviewer_corrected += 1;
                }
            }
        }
        for annotation in state.active_annotations() {
            stats.provenance.record_annotation(annotation);
            stats
                .per_task
                .entry(annotation.task_id.clone())
                .or_default()
                .provenance
                .record_annotation(annotation);
            let class_stats = stats
                .per_class
                .entry(annotation.class_id.clone())
                .or_default();
            class_stats.annotations += 1;
            class_stats.provenance.record_annotation(annotation);
            if matches!(annotation.revision_source, RevisionSource::Human { .. }) {
                let day = annotation.created_at.date_naive().to_string();
                throughput.entry(day).or_default().0 += 1;
            }
        }
        for review in &state.reviews {
            let day = review.timestamp.date_naive().to_string();
            throughput.entry(day).or_default().1 += 1;
        }
    }

    pub(super) fn finish(mut self) -> DatasetStats {
        self.stats.throughput = self
            .throughput
            .into_iter()
            .map(
                |(day, (annotations, reviews))| labello_domain::ThroughputPoint {
                    day,
                    annotations,
                    reviews,
                },
            )
            .collect();
        self.stats
    }
}

pub(super) fn current_task_review_decision(
    state: &ImageState,
    task_id: &TaskId,
) -> Option<ReviewDecision> {
    let task_state = state.task_states.get(task_id)?;
    let task_reviews = state.reviews.iter().filter(|review| {
        matches!(
            &review.target,
            ReviewTarget::Task { task_id: reviewed } if reviewed == task_id
        )
    });

    match task_state.status {
        TaskStatus::Submitted => {
            let round_started_at = task_state.completed_at?;
            task_reviews
                .filter(|review| review.timestamp >= round_started_at)
                .max_by_key(|review| review.timestamp)
                .map(|review| review.decision.clone())
        }
        // Completing an approval round replaces the submitted TaskState timestamp, so the
        // final review, rather than that timestamp, identifies the current round.
        TaskStatus::Completed if task_state.outcome == Some(TaskOutcome::Approved) => task_reviews
            .max_by_key(|review| review.timestamp)
            .map(|review| review.decision.clone()),
        TaskStatus::Completed => None,
        TaskStatus::NeedsCorrection => task_reviews
            .max_by_key(|review| review.timestamp)
            .filter(|review| review.decision == ReviewDecision::Rejected)
            .map(|review| review.decision.clone()),
        _ => None,
    }
}
