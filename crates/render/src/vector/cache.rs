//! The tessellation cache, which a worker thread can fill while the editor keeps drawing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use ssmap_core::Id;

use super::chunk::{GroupMesh, Tessellators, mesh_group};
use super::{ObjectRef, VectorGroup};

/// Tessellated Objects, kept so only an edited Object is tessellated again.
///
/// Clones share one store, so a worker thread can fill it (see [`TessellationCache::rebuild`])
/// while the editor keeps drawing. While a rebuild is running, an Object that is not ready is
/// left out of the frame instead of being tessellated on the drawing thread.
#[derive(Default, Clone)]
pub struct TessellationCache {
    shared: Arc<TessellationShared>,
}

#[derive(Default)]
struct TessellationShared {
    inner: Mutex<TessellationInner>,
    /// Bumped whenever an Object lands or a rebuild ends, so a frame drawn before is out of date.
    generation: AtomicU64,
    /// How many rebuilds are still running.
    building: AtomicUsize,
    /// Held while a rebuild ends, so a waiter never misses the wake-up.
    ended: Mutex<()>,
    ended_signal: Condvar,
}

#[derive(Default)]
struct TessellationInner {
    entries: HashMap<Id, TessellationEntry>,
    /// The newest version of each Object a frame has asked for. A job that finishes with an
    /// older one is stale.
    current: HashMap<Id, u64>,
}

struct TessellationEntry {
    version: u64,
    /// What `meshes` were made from: a Layer's opacity changes a group without changing its
    /// Object's version, so the groups themselves are compared too.
    source: Vec<VectorGroup>,
    meshes: Vec<Arc<GroupMesh>>,
}

impl TessellationCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `other` is a handle to this same store, not just an equal-looking one.
    pub fn same_as(&self, other: &TessellationCache) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    fn inner(&self) -> MutexGuard<'_, TessellationInner> {
        self.shared.inner.lock().expect("tessellation cache lock")
    }

    pub(super) fn note_version(&self, id: Id, version: u64) {
        let mut inner = self.inner();
        let current = inner.current.entry(id).or_insert(version);
        *current = (*current).max(version);
    }

    pub(super) fn lookup(
        &self,
        object: ObjectRef,
        sources: &[&VectorGroup],
    ) -> Option<Vec<Arc<GroupMesh>>> {
        let inner = self.inner();
        let entry = inner.entries.get(&object.id)?;
        (entry.version == object.version
            && entry.source.len() == sources.len()
            && entry.source.iter().zip(sources).all(|(a, &b)| a == b))
        .then(|| entry.meshes.clone())
    }

    /// Keeps the meshes unless a newer version of the Object has been seen since.
    pub(super) fn store(
        &self,
        object: ObjectRef,
        sources: &[&VectorGroup],
        meshes: Vec<Arc<GroupMesh>>,
    ) -> bool {
        let mut inner = self.inner();
        if inner
            .current
            .get(&object.id)
            .is_some_and(|&current| current > object.version)
        {
            return false;
        }
        inner.entries.insert(
            object.id,
            TessellationEntry {
                version: object.version,
                source: sources.iter().map(|&group| group.clone()).collect(),
                meshes,
            },
        );
        true
    }

    pub(super) fn retain(&self, keep: impl Fn(Id) -> bool) {
        let mut inner = self.inner();
        inner.entries.retain(|&id, _| keep(id));
        inner.current.retain(|&id, _| keep(id));
    }

    /// Whether some tessellation of Object `id` at `version` is kept, whatever its groups.
    pub(super) fn has_version(&self, id: Id, version: u64) -> bool {
        self.inner()
            .entries
            .get(&id)
            .is_some_and(|entry| entry.version == version)
    }

    /// Whether Object `id` is tessellated for exactly these groups.
    pub fn is_ready(&self, id: Id, version: u64, groups: &[VectorGroup]) -> bool {
        let sources: Vec<&VectorGroup> = groups.iter().collect();
        self.lookup(ObjectRef { id, version }, &sources).is_some()
    }

    /// How many Objects of `groups` are not tessellated yet, as a rebuild would find. Starts
    /// nothing, so it is cheap to ask every frame.
    pub fn pending(&self, groups: &[VectorGroup]) -> usize {
        let mut by_object: HashMap<Id, (ObjectRef, Vec<&VectorGroup>)> = HashMap::new();
        for group in groups {
            if let Some(object) = group.object {
                by_object
                    .entry(object.id)
                    .or_insert_with(|| (object, Vec::new()))
                    .1
                    .push(group);
            }
        }
        by_object
            .values()
            .filter(|(object, list)| self.lookup(*object, list).is_none())
            .count()
    }

    /// How many Objects are tessellated.
    pub fn len(&self) -> usize {
        self.inner().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Empties the cache, as when another Map is opened.
    pub fn clear(&self) {
        let mut inner = self.inner();
        inner.entries.clear();
        inner.current.clear();
    }

    /// Whether a [`TessellationRebuild`] is still running.
    pub fn is_building(&self) -> bool {
        self.shared.building.load(Ordering::SeqCst) > 0
    }

    /// Blocks until no [`TessellationRebuild`] is running, so a frame drawn afterwards leaves
    /// out no Object. Export calls this before it renders. A rebuild that is neither run nor
    /// dropped on another thread would wait forever.
    pub fn wait_until_ready(&self) {
        let mut guard = self.shared.ended.lock().expect("tessellation wait lock");
        while self.is_building() {
            guard = self
                .shared
                .ended_signal
                .wait(guard)
                .expect("tessellation wait lock");
        }
    }

    /// Changes whenever a rebuild lands an Object or ends, which is when what was drawn from
    /// a partly filled cache is out of date.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    /// Starts tessellating every Object of `groups` that is not ready. Each job snapshots the
    /// Object's groups and version, so the worker never reads anything that is being edited.
    /// Until the returned rebuild is run or dropped, frames leave out Objects that are not
    /// ready. It can be sent to another thread.
    pub fn rebuild(&self, groups: &[VectorGroup]) -> TessellationRebuild {
        let mut order: Vec<ObjectRef> = Vec::new();
        let mut by_object: HashMap<Id, Vec<&VectorGroup>> = HashMap::new();
        for group in groups {
            if let Some(object) = group.object {
                let list = by_object.entry(object.id).or_default();
                if list.is_empty() {
                    order.push(object);
                }
                list.push(group);
            }
        }
        let jobs = order
            .into_iter()
            .filter(|object| self.lookup(*object, &by_object[&object.id]).is_none())
            .map(|object| TessellationJob {
                object,
                groups: by_object[&object.id].iter().map(|&g| g.clone()).collect(),
            })
            .collect();
        self.shared.building.fetch_add(1, Ordering::SeqCst);
        TessellationRebuild {
            cache: self.clone(),
            jobs,
        }
    }
}

struct TessellationJob {
    object: ObjectRef,
    groups: Vec<VectorGroup>,
}

/// Objects still to tessellate, from [`TessellationCache::rebuild`]. Owns its inputs, so it
/// can run on a thread of its own. Dropping it unrun ends the rebuild.
pub struct TessellationRebuild {
    cache: TessellationCache,
    jobs: Vec<TessellationJob>,
}

impl TessellationRebuild {
    /// How many Objects this will tessellate.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Tessellates each Object, calling `ready` after each lands in the cache (so the frame can
    /// be rebuilt and repainted) and once more when the rebuild is over. A result for an Object
    /// that has been edited meanwhile is discarded. Tessellating holds no lock, so drawing is
    /// never blocked by it.
    pub fn run(mut self, mut ready: impl FnMut()) {
        let mut tessellators = Tessellators::default();
        for job in std::mem::take(&mut self.jobs) {
            let meshes = job
                .groups
                .iter()
                .map(|group| Arc::new(mesh_group(group, &mut tessellators)))
                .collect();
            let sources: Vec<&VectorGroup> = job.groups.iter().collect();
            if self.cache.store(job.object, &sources, meshes) {
                self.cache.shared.generation.fetch_add(1, Ordering::SeqCst);
                ready();
            }
        }
        // Ends the rebuild first, so the last repaint sees it over.
        drop(self);
        ready();
    }
}

impl Drop for TessellationRebuild {
    fn drop(&mut self) {
        {
            let _guard = self
                .cache
                .shared
                .ended
                .lock()
                .expect("tessellation wait lock");
            self.cache.shared.building.fetch_sub(1, Ordering::SeqCst);
        }
        self.cache.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.cache.shared.ended_signal.notify_all();
    }
}
