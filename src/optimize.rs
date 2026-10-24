//! 强度削弱 + 常量折叠
//!
//! 在 checker 之后、codegen 之前运行。只做「对整数安全」的变换，
//! 不改变运行时行为。
//!
//! 只做**有类型把握**的变换：字面量与字面量。
//!
//! 曾经这里对 `Expr::Ident` 也做 `x * 2^n → x << n`，但 AST 里没有类型信息，
//! 而 `x` 完全可能是 `float`：`x * 8` 生成 `lume_shl(x, 3)` 会把 double
//! 隐式截成 int64（2.5 * 8 得 16）。这类强度削弱 clang 自己会做，且知道类型，
//! 所以变量参与的部分一律交回给 clang。
//!
//! 移位和位与通过内部函数 `__shl` / `__bitand` 表达，
//! codegen 映射到 runtime 的 `lume_shl` / `lume_bitand`。

use crate::ast::*;

/// 优化级别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 只做绝对安全的字面量变换
    Safe,
    /// 允许将来加入需要类型证明的变换。
    /// 现在与 Safe 等价：变量参与的强度削弱已移除（AST 没有类型，
    /// 无法区分 `int * 8` 和 `float * 8`）。
    Aggressive,
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

fn rewrite_binary(l: &Expr, op: &BinOp, r: &Expr, _level: Level) -> Option<Expr> {
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
    //
    // 只在**两边都是整数字面量**时成立；`x * 2^n` 里的 x 在 AST 层
    // 没有类型（可能是 float），这里不碰，交给知道类型的 clang。
    // 字面量乘法已经被上面的 fold_int 折叠，所以这一节没有实际分支。

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