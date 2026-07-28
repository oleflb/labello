use std::collections::BTreeSet;

use crate::{
    EventLogEntry, EventPayload, ReviewDecision, ReviewRecord, ReviewTarget, TaskId, TaskStatus,
    UserId,
};

pub fn current_task_reviews(events: &[EventLogEntry], task_id: &TaskId) -> Vec<ReviewRecord> {
    let Some(round_start) = events.iter().rposition(|event| {
        matches!(
            &event.payload,
            EventPayload::TaskStateChanged { task_state }
                if task_state.task_id == *task_id
                    && task_state.status == TaskStatus::Submitted
        )
    }) else {
        return Vec::new();
    };
    events
        .iter()
        .skip(round_start + 1)
        .filter_map(|event| match &event.payload {
            EventPayload::ReviewRecorded { review }
                if matches!(
                    &review.target,
                    ReviewTarget::Task { task_id: reviewed } if reviewed == task_id
                ) =>
            {
                Some(review.clone())
            }
            _ => None,
        })
        .collect()
}

pub fn has_task_review_by_user(
    reviews: &[ReviewRecord],
    task_id: &TaskId,
    user_id: &UserId,
) -> bool {
    reviews.iter().any(|review| {
        review.reviewer_user_id == *user_id
            && matches!(
                &review.target,
                ReviewTarget::Task {
                    task_id: reviewed_task_id
                } if reviewed_task_id == task_id
            )
    })
}

pub fn task_approval_count(reviews: &[ReviewRecord], task_id: &TaskId) -> u32 {
    reviews
        .iter()
        .filter_map(|review| match (&review.target, &review.decision) {
            (ReviewTarget::Task { task_id: reviewed }, ReviewDecision::Approved)
                if reviewed == task_id =>
            {
                Some(&review.reviewer_user_id)
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .len() as u32
}
