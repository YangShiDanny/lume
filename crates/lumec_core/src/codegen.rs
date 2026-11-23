use crate::ir::{IrBlock, IrFunc, IrOp, ValueId};
use crate::parser::{Expr, FuncDef, Stmt};
use std::collections::HashMap;

pub fn compile_program(functions: &[FuncDef]) -> Result<Vec<IrFunc>, String> {
    functions.iter().map(compile_function).collect()
}

fn compile_function(function: &FuncDef) -> Result<IrFunc, String> {
    let mut compiler = FunctionCompiler {
        function,
        ops: Vec::new(),
        locals: HashMap::new(),
        next_value: 0,
    };

    for (index, param) in function.params.iter().enumerate() {
        let value = compiler.define(IrOp::Param(index));
        compiler.locals.insert(param.name.clone(), value);
    }

    let mut returned = false;
    for statement in &function.body {
        if returned {
            break;
        }
        returned = compiler.compile_statement(statement)?;
    }
    if !returned {
        let zero = compiler.define(IrOp::Const(0));
        compiler.ops.push(IrOp::Return(zero));
    }

    let compiled = IrFunc {
        name: function.name.clone(),
        n_params: function.params.len(),
        blocks: vec![IrBlock { ops: compiler.ops }],
    };
    compiled
        .validate()
        .map_err(|error| format!("{}: {error}", function.name))?;
    Ok(compiled)
}

struct FunctionCompiler<'a> {
    function: &'a FuncDef,
    ops: Vec<IrOp>,
    locals: HashMap<String, ValueId>,
    next_value: ValueId,
}

impl FunctionCompiler<'_> {
    fn define(&mut self, op: IrOp) -> ValueId {
        let value = self.next_value;
        self.next_value += 1;
        self.ops.push(op);
        value
    }

    fn compile_statement(&mut self, statement: &Stmt) -> Result<bool, String> {
        match statement {
            Stmt::VarDecl { name, init, .. } => {
                let value = self.compile_expr(init)?;
                self.locals.insert(name.clone(), value);
            }
            Stmt::Assign { name, expr } | Stmt::Walrus { name, expr } => {
                if !self.locals.contains_key(name) {
                    return Err(self.error(format!("assignment to undeclared variable `{name}`")));
                }
                let value = self.compile_expr(expr)?;
                self.locals.insert(name.clone(), value);
            }
            Stmt::Ret(expr) => {
                let value = match expr {
                    Some(expr) => self.compile_expr(expr)?,
                    None => self.define(IrOp::Const(0)),
                };
                self.ops.push(IrOp::Return(value));
                return Ok(true);
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr)?;
            }
            Stmt::IndexAssign { .. }
            | Stmt::FieldAssign { .. }
            | Stmt::If { .. }
            | Stmt::While { .. } => {
                return Err(
                    self.error("LLVM lowering currently supports straight-line functions only")
                );
            }
        }
        Ok(false)
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<ValueId, String> {
        match expr {
            Expr::Int(value) => Ok(self.define(IrOp::Const(*value))),
            Expr::Bool(value) => Ok(self.define(IrOp::Const(i64::from(*value)))),
            Expr::Ident(name) => self
                .locals
                .get(name)
                .copied()
                .ok_or_else(|| self.error(format!("use of undeclared variable `{name}`"))),
            Expr::Binary(operator, lhs, rhs) => {
                let lhs = self.compile_expr(lhs)?;
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
                Ok(self.define(op))
            }
            Expr::Call(name, args) => {
                let args = args
                    .iter()
                    .map(|arg| self.compile_expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(self.define(IrOp::Call(name.clone(), args)))
            }
            Expr::Unary(operator, inner) if operator == "-" => {
                let zero = self.define(IrOp::Const(0));
                let value = self.compile_expr(inner)?;
                Ok(self.define(IrOp::Sub(zero, value)))
            }
            _ => Err(self.error("LLVM lowering does not support this expression")),
        }
    }

    fn error(&self, message: impl Into<String>) -> String {
        format!("{}: {}", self.function.name, message.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LumeCore, parser::parse_source};

    #[test]
    fn lowers_integer_lume_functions_to_llvm_jit() {
        let functions = parse_source(
            r#"
fn add(a:int, b:int) -> int
ret a + b
fn main() -> int
ret add(19, 23)
"#,
        )
        .unwrap();
        let ir = compile_program(&functions).unwrap();
        let mut core = LumeCore::new();
        for function in ir {
            core.define_function(function).unwrap();
        }
        assert_eq!(core.call("main", &[]).unwrap(), 42);
    }

    #[test]
    fn rejects_constructs_not_yet_supported_by_llvm_lowering() {
        let functions = parse_source(
            r#"
fn main() -> int
    if true
        ret 1
    end
ret 0
"#,
        )
        .unwrap();
        assert!(
            compile_program(&functions)
                .unwrap_err()
                .contains("straight-line functions only")
        );
    }
}
