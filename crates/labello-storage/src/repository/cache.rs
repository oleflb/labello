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
}

impl AssignmentAvailabilityCache {
    pub(crate) fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub(crate) async fn get(
        &self,
        key: &AssignmentAvailabilityCacheKey,
        generation: u64,
    ) -> Option<BTreeMap<TaskId, bool>> {
        self.values
            .lock()
            .await
            .get(key)
            .filter(|cached| {
                cached.generation == generation
                    && cached.cached_at.elapsed() < ASSIGNMENT_AVAILABILITY_CACHE_TTL
            })
            .map(|cached| cached.tasks.clone())
    }

    pub(crate) async fn lock_refresh(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.refresh.lock().await
    }

    pub(crate) async fn store(
        &self,
        key: AssignmentAvailabilityCacheKey,
        generation: u64,
        tasks: BTreeMap<TaskId, bool>,
    ) {
        self.values.lock().await.insert(
            key,
            CachedAssignmentAvailability {
                generation,
                cached_at: Instant::now(),
                tasks,
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn record_scan(&self) {
        self.scans.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn scan_count(&self) -> u64 {
        self.scans.load(Ordering::Relaxed)
    }
}
