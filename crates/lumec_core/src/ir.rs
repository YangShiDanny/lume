use std::collections::BTreeSet;
use std::fmt;

pub type ValueId = usize;
pub type BlockId = usize;

/// Stack-style IR instructions. Produced values get monotonically increasing IDs
/// in function block/instruction order; `Phi` merges values from control-flow edges.
#[derive(Debug, Clone)]
pub enum IrOp {
    Const(i64),
    Param(usize),
    Add(ValueId, ValueId),
    Sub(ValueId, ValueId),
    Mul(ValueId, ValueId),
    Div(ValueId, ValueId),
    Eq(ValueId, ValueId),
    Neq(ValueId, ValueId),
    Lt(ValueId, ValueId),
    Gt(ValueId, ValueId),
    Le(ValueId, ValueId),
    Ge(ValueId, ValueId),
    Phi(Vec<(BlockId, ValueId)>),
    If(ValueId, BlockId, BlockId),
    Jump(BlockId),
    Return(ValueId),
    Call(String, Vec<ValueId>),
}

impl IrOp {
    pub(crate) fn defines_value(&self) -> bool {
        matches!(
            self,
            Self::Const(_)
                | Self::Param(_)
                | Self::Add(_, _)
                | Self::Sub(_, _)
                | Self::Mul(_, _)
                | Self::Div(_, _)
                | Self::Eq(_, _)
                | Self::Neq(_, _)
                | Self::Lt(_, _)
                | Self::Gt(_, _)
                | Self::Le(_, _)
                | Self::Ge(_, _)
                | Self::Phi(_)
                | Self::Call(_, _)
        )
    }

    fn is_terminator(&self) -> bool {
        matches!(self, Self::If(..) | Self::Jump(_) | Self::Return(_))
    }
}

#[derive(Debug, Clone)]
pub struct IrBlock {
    pub ops: Vec<IrOp>,
}

#[derive(Debug, Clone)]
pub struct IrFunc {
    pub name: String,
    pub n_params: usize,
    pub blocks: Vec<IrBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrValidationError(pub String);

impl fmt::Display for IrValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IrValidationError {}

impl IrFunc {
    /// Returns the number of value-producing instructions, and therefore the
    /// exclusive upper bound for valid `ValueId`s.
    pub fn value_count(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .filter(|op| op.defines_value())
            .count()
    }

    pub fn validate(&self) -> Result<(), IrValidationError> {
        if self.name.is_empty() {
            return Err(IrValidationError("function name must not be empty".into()));
        }
        if self.blocks.is_empty() {
            return Err(IrValidationError(format!(
                "function `{}` has no basic blocks",
                self.name
            )));
        }

        let value_count = self.value_count();
        let mut predecessors = vec![BTreeSet::new(); self.blocks.len()];
        let mut phi_prefix_ended = vec![false; self.blocks.len()];

        for (block_id, block) in self.blocks.iter().enumerate() {
            if block.ops.is_empty() {
                return Err(IrValidationError(format!(
                    "block {block_id} in `{}` is empty and has no terminator",
                    self.name
                )));
            }

            for (op_index, op) in block.ops.iter().enumerate() {
                if op.is_terminator() != (op_index + 1 == block.ops.len()) {
                    return Err(IrValidationError(format!(
                        "block {block_id} in `{}` must have exactly one final terminator",
                        self.name
                    )));
                }

                match op {
                    IrOp::Param(index) if *index >= self.n_params => {
                        return Err(IrValidationError(format!(
                            "parameter {index} is out of range in `{}`",
                            self.name
                        )));
                    }
                    IrOp::Phi(incoming) => {
                        if phi_prefix_ended[block_id] {
                            return Err(IrValidationError(format!(
                                "phi operations must be at the start of block {block_id} in `{}`",
                                self.name
                            )));
                        }
                        let incoming_blocks: BTreeSet<_> =
                            incoming.iter().map(|(pred, _)| *pred).collect();
                        if incoming_blocks.len() != incoming.len() {
                            return Err(IrValidationError(format!(
                                "phi in block {block_id} has duplicate predecessor blocks"
                            )));
                        }
                        if incoming.iter().any(|(_, value)| *value >= value_count) {
                            return Err(IrValidationError(format!(
                                "phi in block {block_id} references an invalid value"
                            )));
                        }
                    }
                    _ if !matches!(op, IrOp::Phi(_)) => phi_prefix_ended[block_id] = true,
                    _ => {}
                }

                match op {
                    IrOp::If(cond, then_block, else_block) => {
                        check_value(*cond, value_count, &self.name)?;
                        for target in [then_block, else_block] {
                            check_block(*target, self.blocks.len(), &self.name)?;
                        }
                        predecessors[*then_block].insert(block_id);
                        predecessors[*else_block].insert(block_id);
                    }
                    IrOp::Jump(target) => {
                        check_block(*target, self.blocks.len(), &self.name)?;
                        predecessors[*target].insert(block_id);
                    }
                    IrOp::Return(value) => check_value(*value, value_count, &self.name)?,
                    IrOp::Add(a, b)
                    | IrOp::Sub(a, b)
                    | IrOp::Mul(a, b)
                    | IrOp::Div(a, b)
                    | IrOp::Eq(a, b)
                    | IrOp::Neq(a, b)
                    | IrOp::Lt(a, b)
                    | IrOp::Gt(a, b)
                    | IrOp::Le(a, b)
                    | IrOp::Ge(a, b) => {
                        check_value(*a, value_count, &self.name)?;
                        check_value(*b, value_count, &self.name)?;
                    }
                    IrOp::Call(_, args) => {
                        for arg in args {
                            check_value(*arg, value_count, &self.name)?;
                        }
                    }
                    IrOp::Phi(incoming) => {
                        for (_, value) in incoming {
                            check_value(*value, value_count, &self.name)?;
                        }
                    }
                    IrOp::Const(_) | IrOp::Param(_) => {}
                }
            }
        }

        for (block_id, block) in self.blocks.iter().enumerate() {
            for op in &block.ops {
                if let IrOp::Phi(incoming) = op {
                    let incoming_blocks: BTreeSet<_> =
                        incoming.iter().map(|(pred, _)| *pred).collect();
                    if incoming_blocks != predecessors[block_id] {
                        return Err(IrValidationError(format!(
                            "phi in block {block_id} must have one incoming value for each predecessor"
                        )));
                    }
                }
            }
        }

        Ok(())
    }
}

fn check_value(
    value: ValueId,
    value_count: usize,
    function: &str,
) -> Result<(), IrValidationError> {
    if value >= value_count {
        return Err(IrValidationError(format!(
            "value {value} is out of range in `{function}`"
        )));
    }
    Ok(())
}

fn check_block(
    block: BlockId,
    block_count: usize,
    function: &str,
) -> Result<(), IrValidationError> {
    if block >= block_count {
        return Err(IrValidationError(format!(
            "block {block} is out of range in `{function}`"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blocks_without_terminators() {
        let function = IrFunc {
            name: "broken".into(),
            n_params: 0,
            blocks: vec![IrBlock { ops: vec![] }],
        };
        assert!(
            function
                .validate()
                .unwrap_err()
                .to_string()
                .contains("empty")
        );
    }

    #[test]
    fn rejects_phi_with_missing_predecessor() {
        let function = IrFunc {
            name: "bad_phi".into(),
            n_params: 0,
            blocks: vec![
                IrBlock {
                    ops: vec![IrOp::Const(1), IrOp::If(0, 1, 2)],
                },
                IrBlock {
                    ops: vec![IrOp::Const(2), IrOp::Jump(3)],
                },
                IrBlock {
                    ops: vec![IrOp::Const(3), IrOp::Jump(3)],
                },
                IrBlock {
                    ops: vec![IrOp::Phi(vec![(1, 1)]), IrOp::Return(3)],
                },
            ],
        };
        assert!(
            function
                .validate()
                .unwrap_err()
                .to_string()
                .contains("each predecessor")
        );
    }
}
