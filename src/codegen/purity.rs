// src/codegen/purity.rs
//
// 纯函数分析：找出"只读参数、只返回值"的函数，给它们打上
// __attribute__((const))（宏 LUME_PURE）。
//
// 这个属性 clang 不敢自己猜 —— 它不知道一个 C 函数会不会读全局变量。
// 但 Lume 有完整 AST，可以证明。打上之后，重复调用会被 CSE 掉，
// 循环里的常量调用会被外提。
//
// 判据是保守的：任何一点不确定就放弃，宁可少标也绝不标错
// （标错了会导致错误的 CSE，属于静默改结果，比不优化严重得多）。

use super::{Codegen, is_array_ty};
use crate::ast::*;
use std::collections::HashSet;

// 数学库函数：无副作用（配合 -fno-math-errno，连 errno 都不写）
const BUILTIN_PURE: &[&str] = &[
    "sqrt", "pow", "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "abs", "fabs", "floor", "ceil", "round", "trunc",
    "log", "log2", "log10", "exp", "exp2", "cbrt", "fmod", "hypot",
    "min", "max", "fmin", "fmax",
];

impl Codegen {
    // 收集整个程序里所有可证明为 const 的顶层函数
    pub(crate) fn collect_const_fns(&mut self, program: &Program) {
        let mut cands: Vec<FnDecl> = Vec::new();
        for item in &program.items {
            if let Item::Fn(f) = item {
                // 必须有返回值；@cache/@precompute 的包装函数读写全局缓存表，不算纯；
                // 返回数组的函数要 malloc，同样不可能是 const
                if f.ret.is_some()
                    && !is_array_ty(&f.ret)
                    && !f.decorators.iter()
                        .any(|d| d.name == "cache" || d.name == "precompute")
                {
                    cands.push(f.clone());
                }
            }
        }

        // 不动点：函数调用别的 const 函数，自己也是 const
        let mut set: HashSet<String> = HashSet::new();
        loop {
            let mut changed = false;
            for f in &cands {
                if set.contains(&f.name) {
                    continue;
                }
                if self.is_const_fn(f, &set) {
                    set.insert(f.name.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        self.const_fns = set;
    }

    fn is_const_fn(&self, f: &FnDecl, set: &HashSet<String>) -> bool {
        let mut locals: HashSet<String> =
            f.params.iter().map(|p| p.name.clone()).collect();
        self.block_const(&f.body, &mut locals, set)
    }

    fn block_const(
        &self,
        b: &Block,
        locals: &mut HashSet<String>,
        set: &HashSet<String>,
    ) -> bool {
        for s in &b.stmts {
            if !self.stmt_const(s, locals, set) {
                return false;
            }
        }
        true
    }

    fn stmt_const(
        &self,
        s: &Stmt,
        locals: &mut HashSet<String>,
        set: &HashSet<String>,
    ) -> bool {
        match s {
            Stmt::Let { name, init, .. } => {
                let ok = self.expr_const(init, locals, set);
                locals.insert(name.clone());
                ok
            }

            // 只允许改自己的局部变量；写数组/写字段/写全局一律放弃
            Stmt::Assign { target, value } => {
                match target {
                    Expr::Ident(n) if locals.contains(n) => {
                        self.expr_const(value, locals, set)
                    }
                    _ => false,
                }
            }

            // 表达式语句只能是调用（别的都没有意义）
            Stmt::Expr(e) => match e {
                Expr::Call(_, _) => self.expr_const(e, locals, set),
                _ => false,
            },

            Stmt::Return(Some(e)) => self.expr_const(e, locals, set),
            Stmt::Return(None) => false,

            Stmt::If { cond, then, els } => {
                if !self.expr_const(cond, locals, set) {
                    return false;
                }
                let mut l2 = locals.clone();
                if !self.block_const(then, &mut l2, set) {
                    return false;
                }
                match els {
                    Some(e) => {
                        let mut l3 = locals.clone();
                        // parser 把 `else { }` 包成 Expr::If(Bool(true), block)
                        match e.as_ref() {
                            Stmt::Expr(Expr::If(c, block, _))
                                if matches!(c.as_ref(), Expr::Bool(true)) =>
                            {
                                self.block_const(block, &mut l3, set)
                            }
                            other => self.stmt_const(other, &mut l3, set),
                        }
                    }
                    None => true,
                }
            }

            Stmt::While { cond, body } => {
                if !self.expr_const(cond, locals, set) {
                    return false;
                }
                let mut l2 = locals.clone();
                self.block_const(body, &mut l2, set)
            }

            Stmt::For { var, iter, body, .. } => {
                if !self.expr_const(iter, locals, set) {
                    return false;
                }
                let mut l2 = locals.clone();
                l2.insert(var.clone());
                self.block_const(body, &mut l2, set)
            }

            // par for 要读写线程共享状态，say 会写 stdout，其余一律保守放弃
            _ => false,
        }
    }

    fn expr_const(
        &self,
        e: &Expr,
        locals: &HashSet<String>,
        set: &HashSet<String>,
    ) -> bool {
        match e {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_)
            | Expr::Rune(_) | Expr::Color(_) => true,

            // 字符串字面量指向静态存储，只读，安全
            Expr::Str(_) => true,

            Expr::Ident(n) => locals.contains(n) || set.contains(n),

            Expr::Binary(l, op, r) => {
                if matches!(op, BinOp::Pipe) {
                    return false;
                }
                self.expr_const(l, locals, set)
                    && self.expr_const(r, locals, set)
            }

            Expr::Unary(_, e) => self.expr_const(e, locals, set),

            // 索引 = 通过指针读内存。GCC 明确规定：
            // "a function that has pointer arguments and examines the data
            //  pointed to must not be declared const"。
            // 数组参数退化成指针，读 a[i] 就是解引用，不能标 const。
            Expr::Index(_, _) => false,

            Expr::Range(a, b, _) => {
                self.expr_const(a, locals, set)
                    && self.expr_const(b, locals, set)
            }

            Expr::Call(callee, args) => {
                if !args.iter().all(|a| self.expr_const(a, locals, set)) {
                    return false;
                }
                match callee.as_ref() {
                    Expr::Ident(n) => {
                        BUILTIN_PURE.contains(&n.as_str())
                            || set.contains(n)
                    }
                    _ => false,
                }
            }

            // 字段访问、方法调用、结构体字面量、插值……
            // 都可能摸到参数以外的内存，保守放弃
            _ => false,
        }
    }
}
