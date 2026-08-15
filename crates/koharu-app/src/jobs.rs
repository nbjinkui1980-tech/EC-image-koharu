//! Bounded job registry.
//!
//! DashMap for O(1) lookup plus a completion-order index: terminal entries
//! (Completed/CompletedWithErrors/Failed/Cancelled) beyond
//! `MAX_COMPLETED_JOBS` evict the oldest terminal entry; running jobs are
//! never tracked by the index and therefore never evicted.

use std::collections::VecDeque;

use dashmap::DashMap;
use koharu_core::JobSummary;
use parking_lot::Mutex;

pub const MAX_COMPLETED_JOBS: usize = 256;

#[derive(Default)]
pub struct BoundedJobRegistry {
    map: DashMap<String, JobSummary>,
    completion_order: Mutex<VecDeque<String>>,
}

impl BoundedJobRegistry {
    /// Insert or update a job. Terminal entries are tracked in completion
    /// order; beyond `MAX_COMPLETED_JOBS` the oldest terminal entry is
    /// evicted. Running jobs are never tracked, hence never evicted.
    pub fn insert(&self, id: String, job: JobSummary) {
        let terminal = !matches!(job.status, koharu_core::JobStatus::Running);
        self.map.insert(id.clone(), job);
        if !terminal {
            return;
        }
        let mut order = self.completion_order.lock();
        if !order.iter().any(|tracked| tracked == &id) {
            order.push_back(id);
        }
        while order.len() > MAX_COMPLETED_JOBS {
            let Some(oldest) = order.pop_front() else {
                break;
            };
            let still_terminal = self
                .map
                .get(&oldest)
                .map(|entry| !matches!(entry.status, koharu_core::JobStatus::Running))
                .unwrap_or(false);
            if still_terminal {
                self.map.remove(&oldest);
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<dashmap::mapref::one::Ref<'_, String, JobSummary>> {
        self.map.get(id)
    }

    pub fn get_mut(
        &self,
        id: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, String, JobSummary>> {
        self.map.get_mut(id)
    }

    pub fn iter(&self) -> dashmap::iter::Iter<'_, String, JobSummary> {
        self.map.iter()
    }

    pub fn contains_key(&self, id: &str) -> bool {
        self.map.contains_key(id)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_core::JobStatus;

    fn insert(registry: &BoundedJobRegistry, id: &str, status: JobStatus) {
        registry.insert(
            id.to_string(),
            JobSummary {
                id: id.into(),
                kind: "test".into(),
                status,
                error: None,
            },
        );
    }

    // AR06-T01 RED: terminal entries beyond 256 evict the oldest; running
    // jobs are never evicted. The RED-0 scaffold has no eviction, so the
    // capacity assertions fail until GREEN.
    #[test]
    fn jobs_completed_beyond_256_evicts_oldest() {
        let registry = BoundedJobRegistry::default();
        for i in 0..257 {
            insert(&registry, &format!("done-{i}"), JobStatus::Completed);
        }
        assert_eq!(registry.len(), MAX_COMPLETED_JOBS);
        assert!(
            !registry.contains_key("done-0"),
            "oldest completed must be evicted first"
        );
        assert!(registry.contains_key("done-256"));
    }

    #[test]
    fn jobs_running_never_evicted() {
        let registry = BoundedJobRegistry::default();
        insert(&registry, "running-1", JobStatus::Running);
        for i in 0..300 {
            insert(&registry, &format!("done-{i}"), JobStatus::Completed);
        }
        assert!(
            registry.contains_key("running-1"),
            "running jobs are never evicted"
        );
        assert_eq!(registry.len(), MAX_COMPLETED_JOBS + 1);
    }

    #[test]
    fn jobs_terminal_reinsert_keeps_single_eviction_slot() {
        let registry = BoundedJobRegistry::default();
        insert(&registry, "dup", JobStatus::Completed);
        insert(&registry, "dup", JobStatus::CompletedWithErrors);
        for i in 0..256 {
            insert(&registry, &format!("done-{i}"), JobStatus::Failed);
        }
        assert_eq!(
            registry.len(),
            MAX_COMPLETED_JOBS,
            "re-inserted terminal id must not occupy a second eviction slot"
        );
    }
}
