use crate::ir::{BlockId, IrBlock, IrFunc, IrOp, ValueId};
use crate::parser::{Expr, FuncDef, Stmt};
use std::collections::{BTreeSet, HashMap};

pub fn compile_program(functions: &[FuncDef]) -> Result<Vec<IrFunc>, String> {
    functions.iter().map(compile_function).collect()
}

fn compile_function(function: &FuncDef) -> Result<IrFunc, String> {
    let mut compiler = FunctionCompiler {
        function,
        blocks: vec![IrBlock { ops: Vec::new() }],
        current: Some(0),
        locals: HashMap::new(),
    };

    for (index, param) in function.params.iter().enumerate() {
        let value = compiler.define(IrOp::Param(index))?;
        compiler.locals.insert(param.name.clone(), value);
    }

    compiler.compile_block(&function.body)?;
    if compiler.current.is_some() {
        let zero = compiler.define(IrOp::Const(0))?;
        compiler.emit(IrOp::Return(zero))?;
    }

    let compiled = IrFunc {
        name: function.name.clone(),
        n_params: function.params.len(),
        blocks: compiler.blocks,
    };
    compiled
        .validate()
        .map_err(|error| format!("{}: {error}", function.name))?;
    Ok(compiled)
}

struct FunctionCompiler<'a> {
    function: &'a FuncDef,
    blocks: Vec<IrBlock>,
    current: Option<BlockId>,
    locals: HashMap<String, ValueId>,
}

impl FunctionCompiler<'_> {
    fn new_block(&mut self) -> BlockId {
        let block = self.blocks.len();
        self.blocks.push(IrBlock { ops: Vec::new() });
        block
    }

    fn value_count(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .filter(|op| defines_value(op))
            .count()
    }

    fn define(&mut self, op: IrOp) -> Result<ValueId, String> {
        let block = self
            .current
            .ok_or_else(|| self.error("cannot emit a value after a terminator"))?;
        let value = self.value_count();
        self.blocks[block].ops.push(op);
        Ok(value)
    }

    fn emit(&mut self, op: IrOp) -> Result<(), String> {
        let block = self
            .current
            .ok_or_else(|| self.error("cannot emit after a terminator"))?;
        self.blocks[block].ops.push(op);
        Ok(())
    }

    fn terminate(&mut self, op: IrOp) -> Result<BlockId, String> {
        let block = self
            .current
            .ok_or_else(|| self.error("block already terminated"))?;
        self.blocks[block].ops.push(op);
        self.current = None;
        Ok(block)
    }

    fn compile_block(&mut self, statements: &[Stmt]) -> Result<Option<ValueId>, String> {
        let mut last_value = None;
        for statement in statements {
            if self.current.is_none() {
                break;
            }
            match statement {
                Stmt::VarDecl { name, init, .. } => {
                    let value = self.compile_expr(init)?;
                    self.locals.insert(name.clone(), value);
                }
                Stmt::Assign { name, expr } | Stmt::Walrus { name, expr } => {
                    if !self.locals.contains_key(name) {
                        return Err(
                            self.error(format!("assignment to undeclared variable `{name}`"))
                        );
                    }
                    let value = self.compile_expr(expr)?;
                    self.locals.insert(name.clone(), value);
                    if matches!(statement, Stmt::Walrus { .. }) {
                        last_value = Some(value);
                    }
                }
                Stmt::Ret(expr) => {
                    let value = match expr {
                        Some(expr) => self.compile_expr(expr)?,
                        None => self.define(IrOp::Const(0))?,
                    };
                    self.terminate(IrOp::Return(value))?;
                }
                Stmt::Expr(expr) => last_value = Some(self.compile_expr(expr)?),
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                } => {
                    self.compile_if(cond, then_body, else_body, false)?;
                    last_value = None;
                }
                Stmt::While { cond, body } => {
                    self.compile_while(cond, body)?;
                    last_value = None;
                }
                Stmt::IndexAssign { .. } | Stmt::FieldAssign { .. } => {
                    return Err(
                        self.error("LLVM lowering does not yet support aggregate assignment")
                    );
                }
            }
        }
        Ok(last_value)
    }

    fn compile_if(
        &mut self,
        condition: &Expr,
        then_body: &[Stmt],
        else_body: &[Stmt],
        produce_value: bool,
    ) -> Result<Option<ValueId>, String> {
        let condition = self.compile_expr(condition)?;
        let branch_block = self
            .current
            .expect("condition evaluation leaves a live block");
        let then_block = self.new_block();
        self.terminate(IrOp::If(condition, then_block, usize::MAX))?;

        let original_locals = self.locals.clone();
        self.current = Some(then_block);
        self.locals = original_locals.clone();
        let mut then_value = self.compile_block(then_body)?;
        if produce_value && self.current.is_some() && then_value.is_none() {
            then_value = Some(self.define(IrOp::Const(0))?);
        }
        let then_end = self.current.map(|block| {
            self.blocks[block].ops.push(IrOp::Jump(usize::MAX));
            self.current = None;
            block
        });
        let then_locals = self.locals.clone();

        let else_block = self.new_block();
        let branch = self.blocks[branch_block]
            .ops
            .last_mut()
            .expect("if source block ends with a branch");
        *branch = IrOp::If(condition, then_block, else_block);
        self.current = Some(else_block);
        self.locals = original_locals.clone();
        let mut else_value = self.compile_block(else_body)?;
        if produce_value && self.current.is_some() && else_value.is_none() {
            else_value = Some(self.define(IrOp::Const(0))?);
        }
        let else_end = self.current.map(|block| {
            self.blocks[block].ops.push(IrOp::Jump(usize::MAX));
            self.current = None;
            block
        });
        let else_locals = self.locals.clone();

        let live_ends: Vec<_> = [then_end, else_end].into_iter().flatten().collect();
        if live_ends.is_empty() {
            self.locals = original_locals;
            return Ok(None);
        }

        let merge_block = self.new_block();
        for end in &live_ends {
            let terminator = self.blocks[*end]
                .ops
                .last_mut()
                .expect("live branch ends with a placeholder jump");
            *terminator = IrOp::Jump(merge_block);
        }
        let branch = self.blocks[branch_block]
            .ops
            .last_mut()
            .expect("if source block ends with a branch");
        *branch = IrOp::If(condition, then_block, else_block);

        self.current = Some(merge_block);
        let live_locals = match (then_end, else_end) {
            (Some(_), Some(_)) => merge_local_maps(
                &original_locals,
                &then_locals,
                &else_locals,
                then_end.unwrap(),
                else_end.unwrap(),
                self,
            )?,
            (Some(_), None) => then_locals,
            (None, Some(_)) => else_locals,
            (None, None) => unreachable!(),
        };
        self.locals = live_locals;

        if produce_value {
            let incoming = [
                then_end.and_then(|block| then_value.map(|value| (block, value))),
                else_end.and_then(|block| else_value.map(|value| (block, value))),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            if incoming.is_empty() {
                return Ok(Some(self.define(IrOp::Const(0))?));
            }
            if incoming.len() == 1 {
                return Ok(Some(incoming[0].1));
            }
            return Ok(Some(self.define(IrOp::Phi(incoming))?));
        }
        Ok(None)
    }

    fn compile_while(&mut self, condition: &Expr, body: &[Stmt]) -> Result<ValueId, String> {
        let preheader = self.current.expect("while begins in a live block");
        let pre_loop_locals = self.locals.clone();
        let initial_result = self.define(IrOp::Const(0))?;
        let header = self.new_block();
        self.terminate(IrOp::Jump(header))?;

        self.current = Some(header);
        let mut loop_values = HashMap::new();
        let mut phi_positions = HashMap::new();
        let mut names: Vec<_> = pre_loop_locals.keys().cloned().collect();
        names.sort();
        for name in names {
            let incoming = pre_loop_locals[&name];
            let value = self.define(IrOp::Phi(vec![(preheader, incoming)]))?;
            let position = self.blocks[header].ops.len() - 1;
            phi_positions.insert(name.clone(), position);
            loop_values.insert(name, value);
        }
        let result = self.define(IrOp::Phi(vec![(preheader, initial_result)]))?;
        let result_phi_position = self.blocks[header].ops.len() - 1;
        self.locals = loop_values.clone();
        let condition_value = self.compile_expr(condition)?;
        let body_block = self.new_block();
        self.terminate(IrOp::If(condition_value, body_block, usize::MAX))?;

        self.current = Some(body_block);
        self.locals = loop_values.clone();
        let body_value = self.compile_block(body)?;
        if let Some(backedge) = self.current {
            self.terminate(IrOp::Jump(header))?;
            for (name, position) in phi_positions {
                let value = self
                    .locals
                    .get(&name)
                    .copied()
                    .unwrap_or(loop_values[&name]);
                let IrOp::Phi(incoming) = &mut self.blocks[header].ops[position] else {
                    unreachable!();
                };
                incoming.push((backedge, value));
            }
            let result_value = body_value.unwrap_or(result);
            let IrOp::Phi(incoming) = &mut self.blocks[header].ops[result_phi_position] else {
                unreachable!();
            };
            incoming.push((backedge, result_value));
        }

        let exit_block = self.new_block();
        let branch = self.blocks[header]
            .ops
            .last_mut()
            .expect("loop header ends with a branch");
        *branch = IrOp::If(condition_value, body_block, exit_block);
        self.current = Some(exit_block);
        self.locals = loop_values;
        Ok(result)
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<ValueId, String> {
        match expr {
            Expr::Int(value) => self.define(IrOp::Const(*value)),
            Expr::Bool(value) => self.define(IrOp::Const(i64::from(*value))),
            Expr::Ident(name) => self
                .locals
                .get(name)
                .copied()
                .ok_or_else(|| self.error(format!("use of undeclared variable `{name}`"))),
            Expr::Binary(operator, lhs, rhs) => {
                let lhs = self.compile_expr(lhs)?;
                if operator == "and" || operator == "or" {
                    return self.compile_short_circuit(operator, lhs, rhs);
                }
                let rhs = self.compile_expr(rhs)?;
                let op = match operator.as_str() {
                    "+" => IrOp::Add(lhs, rhs),
                    "-" => IrOp::Sub(lhs, rhs),
                    "*" => IrOp::Mul(lhs, rhs),
                    "/" => IrOp::Div(lhs, rhs),
                    "==" => IrOp::Eq(lhs, rhs),
                    "!=" => IrOp::Neq(lhs, rhs),
                    "<" => IrOp::Lt(lhs, rhs),
                    ">" => IrOp::Gt(lhs, rhs),
                    "<=" => IrOp::Le(lhs, rhs),
                    ">=" => IrOp::Ge(lhs, rhs),
                    _ => {
                        return Err(
                            self.error(format!("LLVM lowering does not support `{operator}`"))
                        );
                    }
                };
                self.define(op)
            }
            Expr::Call(name, args) => {
                let args = args
                    .iter()
                    .map(|arg| self.compile_expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                self.define(IrOp::Call(name.clone(), args))
            }
            Expr::Unary(operator, inner) if operator == "-" => {
                let zero = self.define(IrOp::Const(0))?;
                let value = self.compile_expr(inner)?;
                self.define(IrOp::Sub(zero, value))
            }
            Expr::Unary(operator, inner) if operator == "not" => {
                let value = self.compile_expr(inner)?;
                let zero = self.define(IrOp::Const(0))?;
                self.define(IrOp::Eq(value, zero))
            }
            Expr::If(condition, then_body, else_body) => self
                .compile_if(condition, then_body, else_body, true)?
                .ok_or_else(|| self.error("if expression has no reachable value")),
            Expr::While(condition, body) => self.compile_while(condition, body),
            _ => Err(self.error("LLVM lowering does not support this expression")),
        }
    }

    fn compile_short_circuit(
        &mut self,
        operator: &str,
        lhs: ValueId,
        rhs: &Expr,
    ) -> Result<ValueId, String> {
        let branch_block = self
            .current
            .expect("logical expression begins in a live block");
        let rhs_block = self.new_block();
        let is_and = operator == "and";
        self.terminate(if is_and {
            IrOp::If(lhs, rhs_block, usize::MAX)
        } else {
            IrOp::If(lhs, usize::MAX, rhs_block)
        })?;

        self.current = Some(rhs_block);
        let rhs_value = self.compile_expr(rhs)?;
        let zero = self.define(IrOp::Const(0))?;
        let rhs_bool = self.define(IrOp::Neq(rhs_value, zero))?;
        let rhs_end = self.terminate(IrOp::Jump(usize::MAX))?;

        let short_block = self.new_block();
        self.current = Some(short_block);
        let short_value = self.define(IrOp::Const(i64::from(!is_and)))?;
        let short_end = self.terminate(IrOp::Jump(usize::MAX))?;
        let merge_block = self.new_block();

        let branch = self.blocks[branch_block]
            .ops
            .last_mut()
            .expect("logical expression source ends with a branch");
        *branch = if is_and {
            IrOp::If(lhs, rhs_block, short_block)
        } else {
            IrOp::If(lhs, short_block, rhs_block)
        };
        for end in [rhs_end, short_end] {
            let terminator = self.blocks[end]
                .ops
                .last_mut()
                .expect("logical path ends with a jump");
            *terminator = IrOp::Jump(merge_block);
        }

        self.current = Some(merge_block);
        self.define(IrOp::Phi(vec![
            (rhs_end, rhs_bool),
            (short_end, short_value),
        ]))
    }

    fn error(&self, message: impl Into<String>) -> String {
        format!("{}: {}", self.function.name, message.into())
    }
}

fn merge_local_maps(
    original: &HashMap<String, ValueId>,
    then_locals: &HashMap<String, ValueId>,
    else_locals: &HashMap<String, ValueId>,
    then_block: BlockId,
    else_block: BlockId,
    compiler: &mut FunctionCompiler<'_>,
) -> Result<HashMap<String, ValueId>, String> {
    let mut names = BTreeSet::new();
    names.extend(original.keys().cloned());
    names.extend(then_locals.keys().cloned());
    names.extend(else_locals.keys().cloned());

    let mut merged = HashMap::new();
    for name in names {
        let original_value = original.get(&name).copied();
        let then_value = then_locals.get(&name).copied().or(original_value);
        let else_value = else_locals.get(&name).copied().or(original_value);
        match (then_value, else_value) {
            (Some(left), Some(right)) if left == right => {
                merged.insert(name, left);
            }
            (Some(left), Some(right)) => {
                let phi =
                    compiler.define(IrOp::Phi(vec![(then_block, left), (else_block, right)]))?;
                merged.insert(name, phi);
            }
            _ => {
                return Err(compiler.error(format!(
                    "variable `{name}` is not initialized on every branch"
                )));
            }
        }
    }
    Ok(merged)
}

fn defines_value(op: &IrOp) -> bool {
    matches!(
        op,
        IrOp::Const(_)
            | IrOp::Param(_)
            | IrOp::Add(_, _)
            | IrOp::Sub(_, _)
            | IrOp::Mul(_, _)
            | IrOp::Div(_, _)
            | IrOp::Eq(_, _)
            | IrOp::Neq(_, _)
            | IrOp::Lt(_, _)
            | IrOp::Gt(_, _)
            | IrOp::Le(_, _)
            | IrOp::Ge(_, _)
            | IrOp::Phi(_)
            | IrOp::Call(_, _)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LumeCore, parser::parse_source};

    fn jit_result(source: &str, function: &str, args: &[i64]) -> i64 {
        let functions = parse_source(source).unwrap();
        let ir = compile_program(&functions).unwrap();
        let mut core = LumeCore::new();
        for function in ir {
            core.define_function(function).unwrap();
        }
        core.call(function, args).unwrap()
    }

    #[test]
    fn lowers_integer_functions_and_calls_to_llvm_jit() {
        assert_eq!(
            jit_result(
                "fn add(a:int, b:int) -> int\nret a + b\nfn main() -> int\nret add(19, 23)\n",
                "main",
                &[]
            ),
            42
        );
    }

    #[test]
    fn lowers_if_expression_with_phi_merge() {
        assert_eq!(
            jit_result(
                "fn choose(value:int) -> int\n    result:int = if value > 0\n        42\n    else\n        7\n    end\nret result\n",
                "choose",
                &[1]
            ),
            42
        );
        assert_eq!(
            jit_result(
                "fn choose(value:int) -> int\n    result:int = if value > 0\n        42\n    else\n        7\n    end\nret result\n",
                "choose",
                &[-1]
            ),
            7
        );
    }

    #[test]
    fn lowers_mutable_if_and_while_control_flow() {
        assert_eq!(
            jit_result(
                "fn total(n:int) -> int\n    sum:int = 0\n    i:int = 0\n    while i < n\n        if i < 3\n            sum := sum + i\n        else\n            sum := sum + 2\n        end\n        i := i + 1\n    end\nret sum\n",
                "total",
                &[5]
            ),
            7
        );
    }

    #[test]
    fn lowers_while_expression_and_sum_loop() {
        assert_eq!(
            jit_result(
                "fn count() -> int\n    i:int = 0\n    result:int = while i < 3\n        i := i + 1\n        i\n    end\nret result\n",
                "count",
                &[]
            ),
            3
        );
        assert_eq!(
            jit_result(
                "fn sum_to(n:int) -> int\n    sum:int = 0\n    i:int = 1\n    while i <= n\n        sum := sum + i\n        i := i + 1\n    end\nret sum\n",
                "sum_to",
                &[10]
            ),
            55
        );
    }

    #[test]
    fn lowers_short_circuit_boolean_expressions() {
        assert_eq!(
            jit_result("fn main() -> int\nret 0 and 1\n", "main", &[]),
            0
        );
        assert_eq!(
            jit_result("fn main() -> int\nret 1 and 2\n", "main", &[]),
            1
        );
        assert_eq!(jit_result("fn main() -> int\nret 1 or 0\n", "main", &[]), 1);
        assert_eq!(jit_result("fn main() -> int\nret not 1\n", "main", &[]), 0);
    }

    #[test]
    fn return_from_each_if_branch_does_not_create_invalid_merge() {
        assert_eq!(
            jit_result(
                "fn absolute(value:int) -> int\n    if value < 0\n        ret 0 - value\n    else\n        ret value\n    end\n",
                "absolute",
                &[-7]
            ),
            7
        );
    }
}
