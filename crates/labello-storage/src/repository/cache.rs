use super::*;

const ASSIGNMENT_AVAILABILITY_CACHE_TTL: Duration = Duration::from_secs(30);

pub(crate) type AssignmentAvailabilityCacheKey = (UserId, String);

#[derive(Clone, Debug)]
struct CachedAssignmentAvailability {
    generation: u64,
    cached_at: Instant,
    tasks: BTreeMap<TaskId, bool>,
}

#[derive(Debug, Default)]
pub(crate) struct AssignmentAvailabilityCache {
    generation: AtomicU64,
    values: AsyncMutex<BTreeMap<AssignmentAvailabilityCacheKey, CachedAssignmentAvailability>>,
    refresh: AsyncMutex<()>,
    #[cfg(test)]
    scans: AtomicU64,
    #[cfg(test)]
    lookup_before_final_generation: Mutex<Option<LookupBatchTestHook>>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct LookupBatchTestHook {
    pub(crate) reached: Arc<tokio::sync::Barrier>,
    pub(crate) resume: Arc<tokio::sync::Barrier>,
}

impl AssignmentAvailabilityCache {
    pub(crate) fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub(crate) async fn lookup_batch(
        &self,
        keys: &[AssignmentAvailabilityCacheKey],
        expected_generation: u64,
    ) -> Option<Vec<BTreeMap<TaskId, bool>>> {
        let initial_generation = self.generation();
        if initial_generation != expected_generation {
            tracing::debug!(
                target: "labello_storage::assignment_availability",
                reason = "generation_changed_before_lookup",
                "assignment availability cache miss"
            );
            return None;
        }

        let values = self.values.lock().await;
        let mut batch = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(cached) = values.get(key).filter(|cached| {
                cached.generation == expected_generation
                    && cached.cached_at.elapsed() < ASSIGNMENT_AVAILABILITY_CACHE_TTL
            }) else {
                tracing::debug!(
                    target: "labello_storage::assignment_availability",
                    reason = "entry_missing_expired_or_obsolete",
                    "assignment availability cache miss"
                );
                return None;
            };
            batch.push(cached.tasks.clone());
        }

        #[cfg(test)]
        let lookup_hook = { self.lookup_before_final_generation.lock().clone() };
        #[cfg(test)]
        if let Some(hook) = lookup_hook {
            hook.reached.wait().await;
            hook.resume.wait().await;
        }

        if self.generation() != expected_generation {
            tracing::debug!(
                target: "labello_storage::assignment_availability",
                reason = "generation_changed_during_lookup",
                "assignment availability cache miss"
            );
            return None;
        }
        Some(batch)
    }

    pub(crate) async fn lock_refresh(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.refresh.lock().await
    }

    pub(crate) async fn store_batch_if_current(
        &self,
        scan_generation: u64,
        batch: Vec<(AssignmentAvailabilityCacheKey, BTreeMap<TaskId, bool>)>,
    ) -> bool {
        let mut values = self.values.lock().await;
        if self.generation() != scan_generation {
            tracing::debug!(
                target: "labello_storage::assignment_availability",
                reason = "scan_invalidated",
                "assignment availability cache publication skipped"
            );
            return false;
        }
        let cached_at = Instant::now();
        for (key, tasks) in batch {
            values.insert(
                key,
                CachedAssignmentAvailability {
                    generation: scan_generation,
                    cached_at,
                    tasks,
                },
            );
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn record_scan(&self) {
        self.scans.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn scan_count(&self) -> u64 {
        self.scans.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn set_lookup_before_final_generation_hook(
        &self,
        hook: Option<LookupBatchTestHook>,
    ) {
        *self.lookup_before_final_generation.lock() = hook;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(kind: &str) -> AssignmentAvailabilityCacheKey {
        (UserId::from("user"), kind.to_string())
    }

    fn tasks(available: bool) -> BTreeMap<TaskId, bool> {
        BTreeMap::from([(TaskId::from("bounding_box:person"), available)])
    }

    #[tokio::test]
    async fn assignment_availability_lookup_rejects_generation_invalidated_before_entry() {
        let cache = AssignmentAvailabilityCache::default();
        let generation = cache.generation();
        assert!(
            cache
                .store_batch_if_current(generation, vec![(key("annotation"), tasks(true))])
                .await
        );

        cache.invalidate();

        assert!(
            cache
                .lookup_batch(&[key("annotation")], generation)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn assignment_availability_lookup_rejects_generation_change_before_linearization() {
        let cache = Arc::new(AssignmentAvailabilityCache::default());
        let generation = cache.generation();
        assert!(
            cache
                .store_batch_if_current(generation, vec![(key("annotation"), tasks(true))])
                .await
        );
        let hook = LookupBatchTestHook {
            reached: Arc::new(tokio::sync::Barrier::new(2)),
            resume: Arc::new(tokio::sync::Barrier::new(2)),
        };
        cache.set_lookup_before_final_generation_hook(Some(hook.clone()));
        let lookup_cache = cache.clone();
        let lookup = tokio::spawn(async move {
            lookup_cache
                .lookup_batch(&[key("annotation")], generation)
                .await
        });

        hook.reached.wait().await;
        cache.invalidate();
        hook.resume.wait().await;

        assert!(lookup.await.unwrap().is_none());
    }

    #[tokio::test]
    async fn assignment_availability_invalidated_scan_is_not_published_as_a_batch() {
        let cache = AssignmentAvailabilityCache::default();
        let generation = cache.generation();
        let annotation = key("annotation");
        let review = key("review");
        assert!(
            cache
                .store_batch_if_current(
                    generation,
                    vec![
                        (annotation.clone(), tasks(true)),
                        (review.clone(), tasks(false)),
                    ],
                )
                .await
        );
        {
            let values = cache.values.lock().await;
            assert_eq!(
                values.get(&annotation).unwrap().cached_at,
                values.get(&review).unwrap().cached_at
            );
        }

        cache.invalidate();
        assert!(
            !cache
                .store_batch_if_current(generation, vec![(annotation, tasks(false))])
                .await
        );
        assert!(
            cache
                .lookup_batch(&[review], cache.generation())
                .await
                .is_none()
        );
    }
}
