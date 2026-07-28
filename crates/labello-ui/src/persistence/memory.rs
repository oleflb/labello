#[cfg(any(not(target_arch = "wasm32"), test))]
#[derive(Default)]
pub(crate) struct MemoryDraftStore {
    records: std::cell::RefCell<std::collections::BTreeMap<String, DraftRecord>>,
    failure: std::cell::RefCell<Option<(String, Option<usize>)>>,
}

#[cfg(any(not(target_arch = "wasm32"), test))]
impl MemoryDraftStore {
    #[cfg(test)]
    pub(crate) fn fail_with(&self, error: impl Into<String>) {
        *self.failure.borrow_mut() = Some((error.into(), None));
    }

    #[cfg(test)]
    pub(crate) fn fail_next(&self, count: usize, error: impl Into<String>) {
        *self.failure.borrow_mut() = Some((error.into(), Some(count)));
    }

    fn check_failure(&self) -> Result<(), String> {
        let mut failure = self.failure.borrow_mut();
        let Some((error, remaining)) = failure.as_mut() else {
            return Ok(());
        };
        let error = error.clone();
        if let Some(remaining) = remaining {
            *remaining = remaining.saturating_sub(1);
            if *remaining == 0 {
                *failure = None;
            }
        }
        Err(error)
    }
}

#[cfg(any(not(target_arch = "wasm32"), test))]
impl DraftStore for MemoryDraftStore {
    fn get<'a>(&'a self, key: &'a str) -> StoreFuture<'a, Option<DraftRecord>> {
        Box::pin(async move {
            self.check_failure()?;
            Ok(self.records.borrow().get(key).cloned())
        })
    }

    fn put<'a>(&'a self, record: DraftRecord) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            self.check_failure()?;
            record.validate_size()?;
            self.records
                .borrow_mut()
                .insert(record.key().to_string(), record);
            Ok(())
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            self.check_failure()?;
            self.records.borrow_mut().remove(key);
            Ok(())
        })
    }

    fn garbage_collect<'a>(&'a self, now: Timestamp) -> StoreFuture<'a, usize> {
        Box::pin(async move {
            self.check_failure()?;
            let cutoff = now - chrono::Duration::seconds(DRAFT_TTL_SECONDS);
            let before = self.records.borrow().len();
            self.records
                .borrow_mut()
                .retain(|_, record| record.updated_at() >= cutoff);
            Ok(before - self.records.borrow().len())
        })
    }
}
