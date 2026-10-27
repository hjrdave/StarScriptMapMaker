#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]
use crate::map::Map;
use crate::op::Op;

use super::{Command, CommandError};

impl Map {
    /// Applies a Command all or nothing and returns the inverses of its Ops, in the order they
    /// were applied. Outside the `test-support` feature, edits go through `History` (ADR 0004).
    #[cfg(any(test, feature = "test-support"))]
    pub fn execute(&mut self, command: &Command) -> Result<Vec<Op>, CommandError> {
        self.execute_recording(command, None)
    }

    /// Like `execute`, and also appends the Ops it applied to `applied`. A failed Command
    /// leaves `applied` as it was.
    pub(crate) fn execute_recording(
        &mut self,
        command: &Command,
        mut applied: Option<&mut Vec<Op>>,
    ) -> Result<Vec<Op>, CommandError> {
        let ops = command.compile(self)?;
        let recorded = applied.as_ref().map_or(0, |applied| applied.len());
        let mut inverses = Vec::with_capacity(ops.len());
        for op in ops {
            if let Some(applied) = applied.as_deref_mut() {
                applied.push(op.clone());
            }
            match self.apply_op(op) {
                Ok(inverse) => inverses.push(inverse),
                Err(err) => {
                    self.revert_ops(inverses);
                    if let Some(applied) = applied {
                        applied.truncate(recorded);
                    }
                    return Err(err.into());
                }
            }
        }
        Ok(inverses)
    }

    /// Undoes Ops from their inverses (as `execute` returns them) and returns the Ops that
    /// redo them, in the same form. Outside the `test-support` feature, `History` undoes.
    #[cfg(any(test, feature = "test-support"))]
    pub fn revert(&mut self, inverses: Vec<Op>) -> Vec<Op> {
        self.revert_ops(inverses)
    }

    /// The reverter behind [`Map::revert`], for `core` itself.
    pub(crate) fn revert_ops(&mut self, inverses: Vec<Op>) -> Vec<Op> {
        inverses
            .into_iter()
            .rev()
            .map(|inverse| self.apply_op(inverse).expect("an inverse always applies"))
            .collect()
    }
}
