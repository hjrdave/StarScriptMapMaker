//! A short log of the Ops applied to a Map, so data derived from it can patch itself after a
//! small edit and rebuild only after a large one.

use std::collections::VecDeque;

use crate::id::Id;

/// How many Ops the log keeps. A Command that edits more nodes than this is a rebuild.
const CAPACITY: usize = 16_384;

/// What one Op did, as far as patching derived data is concerned.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Change {
    /// This node was edited in place: a property set, or its outline replaced. It is not
    /// moved, added or removed.
    Edited(Id),
    /// Anything else: nodes added, removed or moved, boards, Canvas, defaults, files.
    Other,
}

/// The last Ops applied to a Map, each with the revision it produced.
#[derive(Clone, Debug, Default)]
pub(crate) struct ChangeLog {
    entries: VecDeque<(u64, Change)>,
}

impl ChangeLog {
    /// Records that the Op which made the Map `revision` did `change`.
    pub(crate) fn record(&mut self, revision: u64, change: Change) {
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back((revision, change));
    }

    /// What changed after `revision`, or `None` if the log no longer reaches back that far.
    pub(crate) fn since(&self, revision: u64, current: u64) -> Option<Changes> {
        if revision > current {
            return None;
        }
        let wanted = usize::try_from(current - revision).ok()?;
        if wanted > self.entries.len() {
            return None;
        }
        let mut changes = Changes::default();
        let mut seen = std::collections::HashSet::new();
        for &(_, change) in self.entries.iter().skip(self.entries.len() - wanted) {
            match change {
                Change::Edited(id) if seen.insert(id) => changes.edited.push(id),
                Change::Edited(_) => {}
                Change::Other => changes.other = true,
            }
        }
        Some(changes)
    }
}

/// What changed in a Map between a revision and now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// The nodes edited in place (a property set, or an outline replaced), each once, in the
    /// order they were first edited.
    pub edited: Vec<Id>,
    /// Something else changed: a node was added, removed or moved, or a Map-wide setting did.
    /// Derived data should rebuild.
    pub other: bool,
}
