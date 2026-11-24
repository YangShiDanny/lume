//! Parser for the textual IR emitted by `lib/syntax/lower.lume`.
//!
//! The format is intentionally small and line oriented so the Lume-written
//! backend can produce it with string concatenation:
//!
//! ```text
//! func main 0
//! block 0
//! const 0 42
//! ret 0
//! end
//! ```

use crate::ir::{BlockId, IrBlock, IrFunc, IrOp, ValueId};

#[derive(Debug, Clone)]
pub struct IrTextError {
    line: usize,
    message: String,
}

impl IrTextError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for IrTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for IrTextError {}

struct FunctionParser {
    name: String,
    n_params: usize,
    blocks: Vec<IrBlock>,
    value_count: usize,
    started: bool,
}

impl FunctionParser {
    fn new() -> Self {
        Self {
            name: String::new(),
            n_params: 0,
            blocks: Vec::new(),
            value_count: 0,
            started: false,
        }
    }

    fn define(&mut self, id: ValueId, op: IrOp, line: usize) -> Result<(), IrTextError> {
        if id != self.value_count {
            return Err(IrTextError::new(
                line,
                format!("expected next value {id} to be {}", self.value_count),
            ));
        }
        let block = self
            .blocks
            .last_mut()
            .ok_or_else(|| IrTextError::new(line, "value declared outside a block"))?;
        block.ops.push(op);
        self.value_count += 1;
        Ok(())
    }

    fn emit(&mut self, op: IrOp, line: usize) -> Result<(), IrTextError> {
        let block = self
            .blocks
            .last_mut()
            .ok_or_else(|| IrTextError::new(line, "instruction declared outside a block"))?;
        block.ops.push(op);
        Ok(())
    }

    fn block(&mut self, id: BlockId, line: usize) -> Result<(), IrTextError> {
        if id != self.blocks.len() {
            return Err(IrTextError::new(
                line,
                format!("expected next block {id} to be {}", self.blocks.len()),
            ));
        }
        self.blocks.push(IrBlock { ops: Vec::new() });
        Ok(())
    }

    fn finish(self, line: usize) -> Result<IrFunc, IrTextError> {
        if !self.started {
            return Err(IrTextError::new(line, "function has no `func` header"));
        }
        let function = IrFunc {
            name: self.name,
            n_params: self.n_params,
            blocks: self.blocks,
        };
        function
            .validate()
            .map_err(|error| IrTextError::new(line, error.to_string()))?;
        Ok(function)
    }
}

fn integer(token: &str, line: usize, what: &str) -> Result<i64, IrTextError> {
    token
        .parse()
        .map_err(|_| IrTextError::new(line, format!("invalid {what} `{token}`")))
}

fn value(token: &str, line: usize) -> Result<ValueId, IrTextError> {
    let parsed = integer(token, line, "value id")?;
    usize::try_from(parsed)
        .map_err(|_| IrTextError::new(line, format!("negative value id `{token}`")))
}

fn block(token: &str, line: usize) -> Result<BlockId, IrTextError> {
    let parsed = integer(token, line, "block id")?;
    usize::try_from(parsed)
        .map_err(|_| IrTextError::new(line, format!("negative block id `{token}`")))
}

fn phi_incoming(token: &str, line: usize) -> Result<(BlockId, ValueId), IrTextError> {
    let (predecessor, incoming_value) = token
        .split_once(':')
        .ok_or_else(|| IrTextError::new(line, format!("invalid phi incoming `{token}`")))?;
    Ok((block(predecessor, line)?, value(incoming_value, line)?))
}

pub fn parse_program(text: &str) -> Result<Vec<IrFunc>, IrTextError> {
    let mut functions = Vec::new();
    let mut current: Option<FunctionParser> = Some(FunctionParser::new());
    for (index, raw) in text.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        let op = tokens[0];
        let parser = current
            .as_mut()
            .ok_or_else(|| IrTextError::new(line_number, "instruction outside a function"))?;
        match op {
            "func" => {
                if parser.started {
                    return Err(IrTextError::new(line_number, "nested `func` header"));
                }
                if tokens.len() != 3 {
                    return Err(IrTextError::new(
                        line_number,
                        "expected `func <name> <nparams>`",
                    ));
                }
                parser.name = tokens[1].to_string();
                parser.n_params = value(tokens[2], line_number)?;
                parser.started = true;
            }
            "block" => {
                if tokens.len() != 2 {
                    return Err(IrTextError::new(line_number, "expected `block <id>`"));
                }
                parser.block(block(tokens[1], line_number)?, line_number)?;
            }
            "const" => {
                if tokens.len() != 3 {
                    return Err(IrTextError::new(line_number, "expected `const <id> <int>`"));
                }
                let id = value(tokens[1], line_number)?;
                let constant = integer(tokens[2], line_number, "constant")?;
                parser.define(id, IrOp::Const(constant), line_number)?;
            }
            "param" => {
                if tokens.len() != 3 {
                    return Err(IrTextError::new(
                        line_number,
                        "expected `param <id> <index>`",
                    ));
                }
                let id = value(tokens[1], line_number)?;
                let param = value(tokens[2], line_number)?;
                parser.define(id, IrOp::Param(param), line_number)?;
            }
            "add" | "sub" | "mul" | "div" | "eq" | "neq" | "lt" | "gt" | "le" | "ge" => {
                if tokens.len() != 4 {
                    return Err(IrTextError::new(
                        line_number,
                        format!("expected `{op} <id> <lhs> <rhs>`"),
                    ));
                }
                let id = value(tokens[1], line_number)?;
                let lhs = value(tokens[2], line_number)?;
                let rhs = value(tokens[3], line_number)?;
                let operation = match op {
                    "add" => IrOp::Add(lhs, rhs),
                    "sub" => IrOp::Sub(lhs, rhs),
                    "mul" => IrOp::Mul(lhs, rhs),
                    "div" => IrOp::Div(lhs, rhs),
                    "eq" => IrOp::Eq(lhs, rhs),
                    "neq" => IrOp::Neq(lhs, rhs),
                    "lt" => IrOp::Lt(lhs, rhs),
                    "gt" => IrOp::Gt(lhs, rhs),
                    "le" => IrOp::Le(lhs, rhs),
                    "ge" => IrOp::Ge(lhs, rhs),
                    _ => unreachable!(),
                };
                parser.define(id, operation, line_number)?;
            }
            "cast8" | "cast32" => {
                if tokens.len() != 3 {
                    return Err(IrTextError::new(
                        line_number,
                        format!("expected `{op} <id> <source>`"),
                    ));
                }
                let id = value(tokens[1], line_number)?;
                let source = value(tokens[2], line_number)?;
                let width = if op == "cast8" { 8 } else { 32 };
                parser.define(id, IrOp::CastUInt(source, width), line_number)?;
            }
            "call" => {
                if tokens.len() < 3 {
                    return Err(IrTextError::new(
                        line_number,
                        "expected `call <id> <name> [args...]`",
                    ));
                }
                let id = value(tokens[1], line_number)?;
                let arguments = tokens[3..]
                    .iter()
                    .map(|token| value(token, line_number))
                    .collect::<Result<Vec<_>, _>>()?;
                parser.define(
                    id,
                    IrOp::Call(tokens[2].to_string(), arguments),
                    line_number,
                )?;
            }
            "phi" => {
                if tokens.len() < 3 {
                    return Err(IrTextError::new(
                        line_number,
                        "expected `phi <id> <block>:<value> ...`",
                    ));
                }
                let id = value(tokens[1], line_number)?;
                let incoming = tokens[2..]
                    .iter()
                    .map(|token| phi_incoming(token, line_number))
                    .collect::<Result<Vec<_>, _>>()?;
                parser.define(id, IrOp::Phi(incoming), line_number)?;
            }
            "if" => {
                if tokens.len() != 4 {
                    return Err(IrTextError::new(
                        line_number,
                        "expected `if <condition> <then> <else>`",
                    ));
                }
                let condition = value(tokens[1], line_number)?;
                let then_block = block(tokens[2], line_number)?;
                let else_block = block(tokens[3], line_number)?;
                parser.emit(IrOp::If(condition, then_block, else_block), line_number)?;
            }
            "jump" => {
                if tokens.len() != 2 {
                    return Err(IrTextError::new(line_number, "expected `jump <block>`"));
                }
                let target = block(tokens[1], line_number)?;
                parser.emit(IrOp::Jump(target), line_number)?;
            }
            "ret" => {
                if tokens.len() != 2 {
                    return Err(IrTextError::new(line_number, "expected `ret <value>`"));
                }
                let result = value(tokens[1], line_number)?;
                parser.emit(IrOp::Return(result), line_number)?;
            }
            "end" => {
                if tokens.len() != 1 {
                    return Err(IrTextError::new(line_number, "expected `end`"));
                }
                let finished = current.take().unwrap().finish(line_number)?;
                functions.push(finished);
                current = Some(FunctionParser::new());
            }
            other => {
                return Err(IrTextError::new(
                    line_number,
                    format!("unknown IR instruction `{other}`"),
                ));
            }
        }
    }
    if let Some(parser) = current.filter(|parser| parser.started) {
        return Err(IrTextError::new(
            text.lines().count(),
            format!("function `{}` is missing `end`", parser.name),
        ));
    }
    Ok(functions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates_textual_ir() {
        let source = "\
func add 2
block 0
param 0 0
param 1 1
add 2 0 1
ret 2
end
func main 0
block 0
const 0 19
const 1 23
call 2 add 0 1
ret 2
end
";
        let functions = parse_program(source).unwrap();
        assert_eq!(functions.len(), 2);
        assert_eq!(functions[0].name, "add");
        assert_eq!(functions[1].name, "main");
    }

    #[test]
    fn rejects_non_sequential_value_ids() {
        let source = "func main 0\nblock 0\nconst 3 1\nret 3\nend\n";
        assert!(parse_program(source).is_err());
    }
}
