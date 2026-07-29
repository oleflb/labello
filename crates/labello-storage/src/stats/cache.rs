use super::*;

#[derive(Debug)]
pub(super) struct CachedStats {
    pub(super) generation: u64,
    stats: DatasetStats,
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(super) struct StatsScanPause {
    pub(super) started: Notify,
    pub(super) resume: Notify,
}

#[derive(Debug, Default)]
pub(crate) struct StatsCache {
    pub(super) generation: AtomicU64,
    pub(super) value: Mutex<Option<CachedStats>>,
    refresh: Mutex<()>,
    #[cfg(test)]
    scans: AtomicU64,
    #[cfg(test)]
    refresh_attempts: AtomicU64,
    #[cfg(test)]
    refresh_attempted: Notify,
    #[cfg(test)]
    scan_pause: Mutex<Option<Arc<StatsScanPause>>>,
}

impl StatsCache {
    pub(crate) fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    #[cfg(test)]
    pub(super) async fn pause_after_next_scan(&self) -> Arc<StatsScanPause> {
        let pause = Arc::new(StatsScanPause::default());
        *self.scan_pause.lock().await = Some(pause.clone());
        pause
    }

    #[cfg(test)]
    pub(super) async fn wait_for_refresh_attempts(&self, target: u64) {
        loop {
            let attempted = self.refresh_attempted.notified();
            if self.refresh_attempts.load(Ordering::Acquire) >= target {
                return;
            }
            attempted.await;
        }
    }
}

impl DatasetRepository {
    pub async fn dataset_stats(&self) -> StorageResult<DatasetStats> {
        let requested_generation = self.stats_cache.generation.load(Ordering::Acquire);
        {
            let cached = self.stats_cache.value.lock().await;
            if let Some(cached) = cached.as_ref()
                && cached.generation == requested_generation
            {
                return Ok(cached.stats.clone());
            }
        }

        // Keep the scan in the requesting task. Cancellation stops the scan and releases
        // the permit instead of leaving detached blocking scans behind.
        #[cfg(test)]
        {
            self.stats_cache
                .refresh_attempts
                .fetch_add(1, Ordering::Release);
            self.stats_cache.refresh_attempted.notify_one();
        }
        let _refresh = self.stats_cache.refresh.lock().await;
        let generation = self.stats_cache.generation.load(Ordering::Acquire);
        {
            let cached = self.stats_cache.value.lock().await;
            if let Some(cached) = cached.as_ref()
                && cached.generation == generation
            {
                return Ok(cached.stats.clone());
            }
        }

        #[cfg(test)]
        self.stats_cache.scans.fetch_add(1, Ordering::Relaxed);

        let stats = self.compute_dataset_stats().await?;
        #[cfg(test)]
        if let Some(pause) = self.stats_cache.scan_pause.lock().await.take() {
            pause.started.notify_one();
            pause.resume.notified().await;
        }
        let mut cached = self.stats_cache.value.lock().await;
        *cached = Some(CachedStats {
            generation,
            stats: stats.clone(),
        });
        // A concurrent write may make this snapshot stale, but returning it bounds request
        // completion. The next request observes the generation mismatch and refreshes it.
        Ok(stats)
    }

    #[cfg(test)]
    pub(crate) fn stats_scan_count(&self) -> u64 {
        self.stats_cache.scans.load(Ordering::Relaxed)
    }
}
