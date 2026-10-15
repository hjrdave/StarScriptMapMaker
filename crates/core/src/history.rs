//! Transactions and in-memory undo/redo (ADR 0004).
//!
//! A tool opens a transaction, executes Commands live as a gesture goes on, and commits them
//! as one Undo step; rollback cancels the gesture. History lives in memory under a budget and
//! is never saved.

use std::collections::VecDeque;

use thiserror::Error;

use crate::command::{Command, CommandError};
use crate::geometry::MultiPolygon;
use crate::id::Id;
use crate::map::{Map, Subtree};
use crate::node::{Label, Node, ObjectKind};
use crate::op::{Op, Property};

/// The Objects and Layers picked in the editor. It is editor state, not part of the Map, but
/// each Undo step records it so undo and redo can restore it.
pub type Selection = Vec<Id>;

/// The default memory budget for undo history.
pub const DEFAULT_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// The Commands of one gesture or agent action, undone and redone together.
#[derive(Clone, Debug)]
pub struct UndoStep {
    pub label: String,
    pub selection_before: Selection,
    pub selection_after: Selection,
    /// Inverses in the order their Ops were applied.
    inverses: Vec<Op>,
    bytes: usize,
}

#[derive(Debug)]
struct Transaction {
    label: String,
    selection_before: Selection,
    inverses: Vec<Op>,
    /// The Ops applied so far, when recording.
    applied: Vec<Op>,
}

#[derive(Clone, PartialEq, Debug, Error)]
pub enum HistoryError {
    #[error("a transaction is already open")]
    TransactionOpen,
    #[error("no transaction is open")]
    NoTransaction,
}

#[derive(Debug)]
pub struct History {
    undo: VecDeque<UndoStep>,
    redo: Vec<UndoStep>,
    open: Option<Transaction>,
    budget: usize,
    used: usize,
    /// Ops that committed transactions, undo and redo applied, in order, since the last
    /// `take_applied`. `None` unless recording.
    applied: Option<Vec<Op>>,
}

impl Default for History {
    fn default() -> Self {
        Self::new(DEFAULT_BUDGET_BYTES)
    }
}

impl History {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            open: None,
            budget: budget_bytes,
            used: 0,
            applied: None,
        }
    }

    /// Starts or stops recording the Ops that change the Map for good: those of committed
    /// transactions, undo and redo, as the Recovery journal needs. Stopping drops any not yet
    /// taken.
    pub fn set_recording(&mut self, on: bool) {
        self.applied = on.then(Vec::new);
    }

    /// Takes the Ops recorded since the last call, in the order they were applied. Replaying
    /// them on the Map as it was then gives the Map as it is now. Rolled-back Ops are not
    /// included, and a transaction's Ops appear only once it commits.
    pub fn take_applied(&mut self) -> Vec<Op> {
        self.applied
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Opens a transaction. Commands executed until `commit` become one Undo step.
    pub fn begin(
        &mut self,
        label: impl Into<String>,
        selection: Selection,
    ) -> Result<(), HistoryError> {
        if self.open.is_some() {
            return Err(HistoryError::TransactionOpen);
        }
        self.open = Some(Transaction {
            label: label.into(),
            selection_before: selection,
            inverses: Vec::new(),
            applied: Vec::new(),
        });
        Ok(())
    }

    /// Applies a Command inside the open transaction. A failed Command changes nothing and
    /// leaves the transaction open.
    pub fn execute(&mut self, map: &mut Map, command: &Command) -> Result<(), ExecuteError> {
        let transaction = self.open.as_mut().ok_or(HistoryError::NoTransaction)?;
        let applied = self.applied.is_some().then_some(&mut transaction.applied);
        let inverses = map.execute_recording(command, applied)?;
        transaction.inverses.extend(inverses);
        Ok(())
    }

    /// Closes the transaction as one Undo step. A transaction that changed nothing leaves no
    /// step behind.
    pub fn commit(&mut self, selection: Selection) -> Result<(), HistoryError> {
        let transaction = self.open.take().ok_or(HistoryError::NoTransaction)?;
        if transaction.inverses.is_empty() {
            return Ok(());
        }
        if let Some(applied) = &mut self.applied {
            applied.extend(transaction.applied);
        }
        let step = UndoStep {
            bytes: step_bytes(&transaction.inverses),
            label: transaction.label,
            selection_before: transaction.selection_before,
            selection_after: selection,
            inverses: transaction.inverses,
        };
        self.used -= self.redo.drain(..).map(|s| s.bytes).sum::<usize>();
        self.used += step.bytes;
        self.undo.push_back(step);
        self.enforce_budget();
        Ok(())
    }

    /// Cancels the transaction, undoing everything it applied.
    pub fn rollback(&mut self, map: &mut Map) -> Result<(), HistoryError> {
        let transaction = self.open.take().ok_or(HistoryError::NoTransaction)?;
        map.revert(transaction.inverses);
        Ok(())
    }

    /// Undoes everything the open transaction applied but keeps it open. A gesture that
    /// re-issues its whole effect each frame, such as a drag, rewinds first, so the Undo step
    /// holds one Command's worth of inverses and nothing drifts from repeated small edits.
    pub fn rewind(&mut self, map: &mut Map) -> Result<(), HistoryError> {
        let transaction = self.open.as_mut().ok_or(HistoryError::NoTransaction)?;
        map.revert(std::mem::take(&mut transaction.inverses));
        // The journal never sees what was rewound, as it never sees a rollback.
        transaction.applied.clear();
        Ok(())
    }

    /// Executes one Command as its own Undo step. The Selection is unchanged by it.
    pub fn execute_step(
        &mut self,
        map: &mut Map,
        command: &Command,
        selection: Selection,
    ) -> Result<(), ExecuteError> {
        self.begin(command.label(), selection.clone())?;
        if let Err(err) = self.execute(map, command) {
            self.open = None;
            return Err(err);
        }
        self.commit(selection)?;
        Ok(())
    }

    /// Undoes the latest step and returns the Selection to restore, or `None` if there is
    /// nothing to undo.
    pub fn undo(&mut self, map: &mut Map) -> Result<Option<Selection>, HistoryError> {
        self.ensure_closed()?;
        let Some(mut step) = self.undo.pop_back() else {
            return Ok(None);
        };
        self.record_revert(&step);
        step.inverses = map.revert(std::mem::take(&mut step.inverses));
        let selection = step.selection_before.clone();
        self.redo.push(step);
        Ok(Some(selection))
    }

    /// Redoes the latest undone step and returns the Selection to restore.
    pub fn redo(&mut self, map: &mut Map) -> Result<Option<Selection>, HistoryError> {
        self.ensure_closed()?;
        let Some(mut step) = self.redo.pop() else {
            return Ok(None);
        };
        self.record_revert(&step);
        step.inverses = map.revert(std::mem::take(&mut step.inverses));
        let selection = step.selection_after.clone();
        self.undo.push_back(step);
        Ok(Some(selection))
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|step| step.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|step| step.label.as_str())
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Forgets all history, as when the Map closes.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open = None;
        self.used = 0;
    }

    /// Records the Ops that reverting `step` is about to apply: its inverses, newest first.
    fn record_revert(&mut self, step: &UndoStep) {
        if let Some(applied) = &mut self.applied {
            applied.extend(step.inverses.iter().rev().cloned());
        }
    }

    fn ensure_closed(&self) -> Result<(), HistoryError> {
        match self.open {
            Some(_) => Err(HistoryError::TransactionOpen),
            None => Ok(()),
        }
    }

    /// Drops the oldest steps until history fits the budget, always keeping the newest.
    fn enforce_budget(&mut self) {
        while self.used > self.budget && self.undo.len() > 1 {
            let oldest = self.undo.pop_front().expect("more than one step");
            self.used -= oldest.bytes;
        }
    }
}

#[derive(Clone, PartialEq, Debug, Error)]
pub enum ExecuteError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    History(#[from] HistoryError),
}

/// A rough count of the memory a step holds, for the history budget.
fn step_bytes(ops: &[Op]) -> usize {
    ops.iter().map(op_bytes).sum()
}

fn op_bytes(op: &Op) -> usize {
    std::mem::size_of::<Op>()
        + match op {
            Op::Insert { subtree, .. } => subtree_bytes(subtree),
            Op::SetProperty {
                property: Property::Name(name),
                ..
            } => name.len(),
            Op::SetProperty {
                property: Property::Label(label),
                ..
            } => std::mem::size_of::<Label>() + label.text.len(),
            Op::ReplaceGeometry { geometry, .. } => geometry_bytes(geometry),
            Op::SetBlob { data, .. } => data.as_ref().map_or(0, |data| data.len()),
            Op::Remove { .. }
            | Op::SetProperty { .. }
            | Op::Move { .. }
            | Op::SetDefaults { .. }
            | Op::SetEmbeddedAsset { .. }
            | Op::SetEmbeddedFont { .. } => 0,
        }
}

fn subtree_bytes(subtree: &Subtree) -> usize {
    let own = match &subtree.node {
        Node::Layer(layer) => layer.name.len(),
        Node::Object(object) => match &object.kind {
            ObjectKind::Label(label) => label.text.len(),
            kind => kind.geometry().map_or(0, geometry_bytes),
        },
    };
    std::mem::size_of::<Subtree>() + own + subtree.children.iter().map(subtree_bytes).sum::<usize>()
}

fn geometry_bytes(geometry: &MultiPolygon) -> usize {
    let points: usize = geometry
        .0
        .iter()
        .map(|polygon| polygon.exterior.len() + polygon.holes.iter().map(Vec::len).sum::<usize>())
        .sum();
    points * std::mem::size_of::<crate::geometry::Point>()
}
