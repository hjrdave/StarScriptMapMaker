use crate::map::Map;
use crate::op::Op;

use super::{Command, CommandError};

impl Map {
    /// Applies a Command all or nothing and returns the inverses of its Ops, in the order they
    /// were applied.
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
            match self.apply(op) {
                Ok(inverse) => inverses.push(inverse),
                Err(err) => {
                    self.revert(inverses);
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
    /// redo them, in the same form.
    pub fn revert(&mut self, inverses: Vec<Op>) -> Vec<Op> {
        inverses
            .into_iter()
            .rev()
            .map(|inverse| self.apply(inverse).expect("an inverse always applies"))
            .collect()
    }
}
