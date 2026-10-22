//! The offers of Recovery journals of Maps never saved, made one at a time.

use crate::recovery::Found;

/// The journals of Maps never saved that a crash left, and the one being offered now.
#[derive(Default)]
pub(super) struct UnsavedOffer {
    /// Oldest first, offered one at a time: the next is the last.
    found: Vec<Found>,
    /// The journal being offered now; recovering it opens a new tab.
    prompt: Option<Found>,
}

impl UnsavedOffer {
    /// Offers `found`, given oldest first.
    pub(super) fn new(mut found: Vec<Found>) -> Self {
        found.reverse();
        Self {
            found,
            prompt: None,
        }
    }

    /// Whether a journal is being offered.
    pub(super) fn is_asking(&self) -> bool {
        self.prompt.is_some()
    }

    /// Starts offering the next journal, unless one is being offered already.
    pub(super) fn offer_next(&mut self) {
        if self.prompt.is_none() {
            self.prompt = self.found.pop();
        }
    }

    /// Takes the journal being offered, once it is answered.
    pub(super) fn take_prompt(&mut self) -> Option<Found> {
        self.prompt.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::file::{JournalBase, Recovered};
    use std::path::PathBuf;
    use uuid::Uuid;

    fn found(name: &str) -> Found {
        Found {
            path: PathBuf::from(name),
            recovered: Recovered {
                base: JournalBase::File {
                    generation: Uuid::nil(),
                },
                ops: Vec::new(),
                torn: false,
            },
        }
    }

    #[test]
    fn offers_oldest_first_one_at_a_time() {
        let mut offer = UnsavedOffer::new(vec![found("old"), found("new")]);
        assert!(!offer.is_asking());
        offer.offer_next();
        assert!(offer.is_asking());
        offer.offer_next();
        let first = offer.take_prompt().expect("offered");
        assert_eq!(first.path, PathBuf::from("old"));
        assert!(!offer.is_asking());
        offer.offer_next();
        let second = offer.take_prompt().expect("offered");
        assert_eq!(second.path, PathBuf::from("new"));
        offer.offer_next();
        assert!(!offer.is_asking());
    }
}
