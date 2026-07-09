use std::collections::BTreeMap;

use labello_domain::{
    ClassStats, DatasetStats, ReviewDecision, ReviewTarget, TaskStats, TaskStatus,
};

use crate::{DatasetRepository, StorageResult};

impl DatasetRepository {
    pub async fn dataset_stats(&self) -> StorageResult<DatasetStats> {
        let metadata = self.load_dataset().await?;
        let mut stats = DatasetStats {
            total_images: metadata.images.len(),
            per_task: metadata
                .tasks
                .iter()
                .map(|task| (task.task_id.clone(), TaskStats::default()))
                .collect(),
            per_class: metadata
                .label_classes
                .iter()
                .map(|class| (class.class_id.clone(), ClassStats::default()))
                .collect(),
            ..DatasetStats::default()
        };
        let mut throughput: BTreeMap<String, (usize, usize)> = BTreeMap::new();

        for image_id in metadata.images.keys() {
            let state = self.load_image_state(image_id).await?;
            for task in &metadata.tasks {
                let task_stats = stats.per_task.entry(task.task_id.clone()).or_default();
                match state
                    .task_states
                    .get(&task.task_id)
                    .map(|state| &state.status)
                    .unwrap_or(&TaskStatus::Pending)
                {
                    TaskStatus::Completed => {
                        stats.completed_tasks += 1;
                        task_stats.completed += 1;
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
            }
            for annotation in state.active_annotations() {
                stats
                    .per_class
                    .entry(annotation.class_id.clone())
                    .or_default()
                    .annotations += 1;
                let day = annotation.created_at.date_naive().to_string();
                throughput.entry(day).or_default().0 += 1;
            }
            for review in &state.reviews {
                if review.decision == ReviewDecision::Approved {
                    stats.reviewed_tasks += 1;
                    if let ReviewTarget::Task { task_id } = &review.target {
                        stats.per_task.entry(task_id.clone()).or_default().reviewed += 1;
                    }
                }
                let day = review.timestamp.date_naive().to_string();
                throughput.entry(day).or_default().1 += 1;
            }
        }

        stats.throughput = throughput
            .into_iter()
            .map(
                |(day, (annotations, reviews))| labello_domain::ThroughputPoint {
                    day,
                    annotations,
                    reviews,
                },
            )
            .collect();
        Ok(stats)
    }
}
