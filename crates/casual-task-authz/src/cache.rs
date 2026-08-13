//! Bounded epoch-keyed storage for read-side authorization answers.
//!
//! `docs/04` makes the epoch part of the key instead of broadcasting an
//! invalidation message. A changed grant or membership increments the epoch in
//! the same transaction; the old entry can remain allocated until eviction,
//! but no later request can address it.
//!
//! This module owns storage mechanics only. It does not load grants or decide
//! permissions, which keeps SQL and application types out of this crate.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use casual_task_model::{ActorType, ProjectId, UserId, WorkspaceId};

/// The complete identity of a cached read-side authorization answer.
///
/// `actor_type` is intentionally present in addition to the tuple originally
/// written in `docs/04`: user and service-account ids come from different
/// tables and may contain the same UUID. Omitting the type would let that
/// collision reuse the other principal kind's authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub workspace: WorkspaceId,
    pub actor: UserId,
    pub actor_type: ActorType,
    pub project: Option<ProjectId>,
    pub epoch: i64,
}

#[derive(Debug, Clone)]
struct Entry<V> {
    value: V,
    expires_at: Instant,
    generation: u64,
}

#[derive(Debug)]
struct Store<K, V> {
    entries: HashMap<K, Entry<V>>,
    order: VecDeque<(u64, K)>,
    generation: u64,
}

impl<K, V> Default for Store<K, V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            generation: 0,
        }
    }
}

/// A bounded, time-limited in-process cache.
///
/// Values are cloned out so no lock is held while a request evaluates a
/// permission. Capacity is an entry count rather than an unbounded map; zero is
/// normalized to one because a cache that accepts configuration and stores
/// nothing makes its hit metric misleading.
#[derive(Debug)]
pub struct EpochCache<K, V> {
    capacity: usize,
    ttl: Duration,
    store: Mutex<Store<K, V>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl<K, V> EpochCache<K, V>
where
    K: Clone + Eq + Hash,
    V: Clone,
{
    #[must_use]
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            capacity: capacity.max(1),
            ttl,
            store: Mutex::new(Store::default()),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Return a live value for `key`.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        let found = self.get_at(key, Instant::now());
        if found.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        found
    }

    /// Store a value, replacing an existing value for the same key.
    pub fn insert(&self, key: K, value: V) {
        self.insert_at(key, value, Instant::now());
    }

    /// Fraction of lookups served from this cache since process start.
    ///
    /// Before the first lookup the ratio is zero. The counters are monotonic,
    /// so publishing this as a gauge cannot produce a value outside `0..=1`.
    #[must_use]
    pub fn hit_ratio(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed);
        let total = hits.saturating_add(self.misses.load(Ordering::Relaxed));
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    fn get_at(&self, key: &K, now: Instant) -> Option<V> {
        let mut store = self
            .store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = store.entries.get(key)?;
        if entry.expires_at <= now {
            store.entries.remove(key);
            return None;
        }
        Some(entry.value.clone())
    }

    fn insert_at(&self, key: K, value: V, now: Instant) {
        let mut store = self
            .store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        store.generation = store.generation.wrapping_add(1);
        let generation = store.generation;
        store.entries.insert(
            key.clone(),
            Entry {
                value,
                expires_at: now + self.ttl,
                generation,
            },
        );
        store.order.push_back((generation, key));

        while store.entries.len() > self.capacity {
            let Some((queued_generation, queued_key)) = store.order.pop_front() else {
                break;
            };
            let is_current = store
                .entries
                .get(&queued_key)
                .is_some_and(|entry| entry.generation == queued_generation);
            if is_current {
                store.entries.remove(&queued_key);
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .entries
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(workspace: WorkspaceId, epoch: i64) -> CacheKey {
        CacheKey {
            workspace,
            actor: UserId::new(),
            actor_type: ActorType::User,
            project: Some(ProjectId::new()),
            epoch,
        }
    }

    #[test]
    fn an_epoch_bump_makes_the_old_answer_unaddressable() {
        let cache = EpochCache::new(8, Duration::from_secs(60));
        let workspace = WorkspaceId::new();
        let before = key(workspace, 4);
        cache.insert(before, "old");

        assert_eq!(cache.get(&before), Some("old"));
        assert_eq!(cache.get(&CacheKey { epoch: 5, ..before }), None);
    }

    #[test]
    fn identical_actor_and_project_ids_do_not_cross_workspaces() {
        let cache = EpochCache::new(8, Duration::from_secs(60));
        let alpha = key(WorkspaceId::new(), 1);
        let beta = CacheKey {
            workspace: WorkspaceId::new(),
            ..alpha
        };
        cache.insert(alpha, "alpha");
        cache.insert(beta, "beta");

        assert_eq!(cache.get(&alpha), Some("alpha"));
        assert_eq!(cache.get(&beta), Some("beta"));
    }

    #[test]
    fn principal_kinds_with_the_same_id_do_not_share_an_answer() {
        let cache = EpochCache::new(8, Duration::from_secs(60));
        let user = key(WorkspaceId::new(), 1);
        let service = CacheKey {
            actor_type: ActorType::ServiceAccount,
            ..user
        };
        cache.insert(user, "user");
        cache.insert(service, "service");

        assert_eq!(cache.get(&user), Some("user"));
        assert_eq!(cache.get(&service), Some("service"));
    }

    #[test]
    fn expired_entries_miss_without_sleeping() {
        let cache = EpochCache::new(8, Duration::from_secs(5));
        let start = Instant::now();
        let key = key(WorkspaceId::new(), 1);
        cache.insert_at(key, "answer", start);

        assert_eq!(
            cache.get_at(&key, start + Duration::from_secs(4)),
            Some("answer")
        );
        assert_eq!(cache.get_at(&key, start + Duration::from_secs(5)), None);
    }

    #[test]
    fn capacity_evicts_the_oldest_live_entry() {
        let cache = EpochCache::new(2, Duration::from_secs(60));
        let one = key(WorkspaceId::new(), 1);
        let two = key(WorkspaceId::new(), 1);
        let three = key(WorkspaceId::new(), 1);
        cache.insert(one, 1);
        cache.insert(two, 2);
        cache.insert(three, 3);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&one), None);
        assert_eq!(cache.get(&two), Some(2));
        assert_eq!(cache.get(&three), Some(3));
    }

    #[test]
    fn hit_ratio_counts_live_hits_and_all_misses() {
        let cache = EpochCache::new(2, Duration::from_secs(60));
        let present = key(WorkspaceId::new(), 1);
        let absent = key(WorkspaceId::new(), 1);
        cache.insert(present, "answer");

        assert_eq!(cache.hit_ratio(), 0.0);
        assert_eq!(cache.get(&present), Some("answer"));
        assert_eq!(cache.get(&absent), None);
        assert_eq!(cache.hit_ratio(), 0.5);
    }
}
