//! 强度削弱 + 常量折叠
//!
//! 在 checker 之后、codegen 之前运行。只做「对整数安全」的变换，
//! 不改变运行时行为。
//!
//! 关键假设：`x * 2^n` 里的 `x` 是整数。`x` 是浮点时，
//! `x * 8` 变成 `x << 3` 是错的（double 不能移位）。
//! 所以只对 `Expr::Int` 和 `Expr::Ident` 做——`Ident` 假设是整数。
//!
//! 移位和位与通过内部函数 `__shl` / `__bitand` 表达，
//! codegen 映射到 runtime 的 `lume_shl` / `lume_bitand`。

use crate::ast::*;

/// 优化级别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 只优化字面量（绝对安全）
    Safe,
    /// 也优化 `Ident`（假设是整数）
    Aggressive,
}

impl Default for Level {
    fn default() -> Self {
        Level::Aggressive
    }
}

pub fn optimize(program: &mut Program) {
    optimize_with(program, Level::default());
}

pub fn optimize_with(program: &mut Program, level: Level) {
    for item in &mut program.items {
        match item {
            Item::Fn(f) => optimize_fn(f, level),
            Item::Struct(s) => {
                for m in &mut s.methods {
                    optimize_fn(m, level);
                }
            }
            _ => {}
        }
    }
}

fn optimize_fn(f: &mut FnDecl, level: Level) {
    optimize_block(&mut f.body, level);
}

fn optimize_block(b: &mut Block, level: Level) {
    for stmt in &mut b.stmts {
        optimize_stmt(stmt, level);
    }
}

fn optimize_stmt(s: &mut Stmt, level: Level) {
    match s {
        Stmt::Let { init, .. } => optimize_expr(init, level),
        Stmt::Assign { target, value } => {
            optimize_expr(target, level);
            optimize_expr(value, level);
        }
        Stmt::Expr(e) => optimize_expr(e, level),
        Stmt::If { cond, then, els } => {
            optimize_expr(cond, level);
            optimize_block(then, level);
            if let Some(e) = els {
                optimize_stmt(e, level);
            }
        }
        Stmt::While { cond, body } => {
            optimize_expr(cond, level);
            optimize_block(body, level);
        }
        Stmt::For { iter, body, .. } | Stmt::ParFor { iter, body, .. } => {
            optimize_expr(iter, level);
            optimize_block(body, level);
        }
        Stmt::Return(Some(e)) => optimize_expr(e, level),
        Stmt::Match { value, arms } => {
            optimize_expr(value, level);
            for arm in arms {
                optimize_expr(&mut arm.body, level);
            }
        }
        _ => {}
    }
}

fn optimize_expr(e: &mut Expr, level: Level) {
    // 1. 先递归子表达式（自底向上）
    match e {
        Expr::Binary(l, _, r) => {
            optimize_expr(l, level);
            optimize_expr(r, level);
        }
        Expr::Unary(_, x) => optimize_expr(x, level),
        Expr::Call(c, args) => {
            optimize_expr(c, level);
            for a in args {
                optimize_expr(a, level);
            }
        }
        Expr::Field(o, _) => optimize_expr(o, level),
        Expr::Index(o, i) => {
            optimize_expr(o, level);
            optimize_expr(i, level);
        }
        Expr::If(c, t, el) => {
            optimize_expr(c, level);
            optimize_block(t, level);
            if let Some(b) = el {
                optimize_block(b, level);
            }
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                optimize_expr(item, level);
            }
        }
        Expr::StructLit(_, fields) => {
            for (_, v) in fields {
                optimize_expr(v, level);
            }
        }
        Expr::Range(a, b, _) => {
            optimize_expr(a, level);
            optimize_expr(b, level);
        }
        Expr::Repeat(v, _) => optimize_expr(v, level),
        _ => {}
    }

    // 2. 尝试重写当前节点（一次，不递归——规则是单向的）
    if let Some(new) = rewrite(e, level) {
        *e = new;
    }
}

fn rewrite(e: &Expr, level: Level) -> Option<Expr> {
    match e {
        Expr::Binary(l, op, r) => rewrite_binary(l, op, r, level),
        _ => None,
    }
}

fn rewrite_binary(l: &Expr, op: &BinOp, r: &Expr, level: Level) -> Option<Expr> {
    // ── 常量折叠 ──

    // 整数比较 → Bool
    if let (Expr::Int(a), Expr::Int(b)) = (l, r) {
        match op {
            BinOp::Eq => return Some(Expr::Bool(a == b)),
            BinOp::Ne => return Some(Expr::Bool(a != b)),
            BinOp::Lt => return Some(Expr::Bool(a < b)),
            BinOp::Gt => return Some(Expr::Bool(a > b)),
            BinOp::Le => return Some(Expr::Bool(a <= b)),
            BinOp::Ge => return Some(Expr::Bool(a >= b)),
            _ => {}
        }
        if let Some(v) = fold_int(*op, *a, *b) {
            return Some(Expr::Int(v));
        }
    }

    // 浮点比较 → Bool
    if let (Expr::Float(a), Expr::Float(b)) = (l, r) {
        match op {
            BinOp::Eq => return Some(Expr::Bool(a == b)),
            BinOp::Ne => return Some(Expr::Bool(a != b)),
            BinOp::Lt => return Some(Expr::Bool(a < b)),
            BinOp::Gt => return Some(Expr::Bool(a > b)),
            BinOp::Le => return Some(Expr::Bool(a <= b)),
            BinOp::Ge => return Some(Expr::Bool(a >= b)),
            _ => {}
        }
        if let Some(v) = fold_float(*op, *a, *b) {
            return Some(Expr::Float(v));
        }
    }

    // 布尔 and / or
    if let (Expr::Bool(a), Expr::Bool(b)) = (l, r) {
        match op {
            BinOp::And => return Some(Expr::Bool(*a && *b)),
            BinOp::Or => return Some(Expr::Bool(*a || *b)),
            _ => {}
        }
    }

    // ── 强度削弱：乘以 2^n → 左移 ──

    if let BinOp::Mul = op {
        // x * 2^n → x << n
        if let Expr::Int(n) = r {
            if *n > 1 && (*n as u64).is_power_of_two() {
                let shift = (*n as u64).trailing_zeros();
                if shift <= 62 && should_optimize_operand(l, level) {
                    return Some(make_shl(l.clone(), shift));
                }
            }
            // x * 3/5/7/9/15/17/31/33 → 移位 + 加减
            if should_optimize_operand(l, level) {
                if let Some(repl) = strength_reduce_mul(l.clone(), *n) {
                    return Some(repl);
                }
            }
        }
        // 2^n * x → x << n
        if let Expr::Int(n) = l {
            if *n > 1 && (*n as u64).is_power_of_two() {
                let shift = (*n as u64).trailing_zeros();
                if shift <= 62 && should_optimize_operand(r, level) {
                    return Some(make_shl(r.clone(), shift));
                }
            }
        }
    }

    // ── 取模 2^n → 位与（仅非负字面量）──
    //    变量可能是负数（-7 % 8 = -7，-7 & 7 = 1），不安全。
    //    所以只处理两边都是常量字面量的情况。

    if let BinOp::Mod = op {
        if let (Expr::Int(a), Expr::Int(b)) = (l, r) {
            if *a >= 0 && *b > 0 && (*b as u64).is_power_of_two() {
                let mask = (*b as u64) - 1;
                return Some(Expr::Int(*a & mask as i64));
            }
        }
    }

    // ── 单位元（只做对浮点也安全的）──
    //    x * 1 → x, 1 * x → x, x / 1 → x
    //    x + 0 / x - 0 / x * 0 对浮点不安全（-0.0、NaN），不做。

    match (l, op, r) {
        (_, BinOp::Mul, Expr::Int(1)) => return Some(l.clone()),
        (Expr::Int(1), BinOp::Mul, _) => return Some(r.clone()),
        (_, BinOp::Div, Expr::Int(1)) => return Some(l.clone()),
        _ => {}
    }

    None
}

/// 是否对这个操作数做强度削弱
fn should_optimize_operand(e: &Expr, level: Level) -> bool {
    match level {
        Level::Safe => matches!(e, Expr::Int(_)),
        // 只对字面量和标识符做。Call / Binary 可能是浮点，跳过。
        Level::Aggressive => matches!(e, Expr::Int(_) | Expr::Ident(_)),
    }
}

/// 构造 `__shl(x, n)`
fn make_shl(x: Expr, shift: u32) -> Expr {
    Expr::Call(
        Box::new(Expr::Ident("__shl".into())),
        vec![x, Expr::Int(shift as i64)],
    )
}

/// `x * 3` 这类非 2 的幂 → `x + (x << 1)` 等
fn strength_reduce_mul(x: Expr, n: i64) -> Option<Expr> {
    if n <= 1 { return None; }
    let n = n as u64;
    if n.is_power_of_two() {
        return None; // 已经在调用处处理
    }

    let x_box = Box::new(x.clone());
    let shl = |s: u32| -> Box<Expr> {
        Box::new(Expr::Call(
            Box::new(Expr::Ident("__shl".into())),
            vec![x.clone(), Expr::Int(s as i64)],
        ))
    };

    let repl = match n {
        3  => Expr::Binary(x_box, BinOp::Add, shl(1)),
        5  => Expr::Binary(x_box, BinOp::Add, shl(2)),
        7  => Expr::Binary(shl(3), BinOp::Sub, x_box),
        9  => Expr::Binary(x_box, BinOp::Add, shl(3)),
        15 => Expr::Binary(shl(4), BinOp::Sub, x_box),
        17 => Expr::Binary(x_box, BinOp::Add, shl(4)),
        31 => Expr::Binary(shl(5), BinOp::Sub, x_box),
        33 => Expr::Binary(x_box, BinOp::Add, shl(5)),
        _ => return None,
    };
    Some(repl)
}

fn fold_int(op: BinOp, a: i64, b: i64) -> Option<i64> {
    use std::num::Wrapping;
    let (wa, wb) = (Wrapping(a), Wrapping(b));
    let r = match op {
        BinOp::Add => (wa + wb).0,
        BinOp::Sub => (wa - wb).0,
        BinOp::Mul => (wa * wb).0,
        BinOp::Div => {
            if b == 0 { return None; }
            if a == i64::MIN && b == -1 { return None; }
            a / b
        }
        BinOp::Mod => {
            if b == 0 { return None; }
            if a == i64::MIN && b == -1 { return Some(0); }
            a % b
        }
        _ => return None,
    };
    Some(r)
}

fn fold_float(op: BinOp, a: f64, b: f64) -> Option<f64> {
    let r = match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => a / b,
        _ => return None,
    };
    if r.is_finite() { Some(r) } else { None }
}