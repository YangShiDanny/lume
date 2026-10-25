//! 运算符重载：源码里的 `a + b` 怎么落到结构体方法上。
//!
//! 一份解析，两个用户：
//! - **checker** 只看「有没有这个方法」「结果类型是什么」；
//! - **codegen** 还要知道「叫什么、参数要不要换位、结果要不要取反」。
//!
//! 两边必须查同一份表。分开写就会出现「检查通过但生成不出来」
//! （checker 认了 op_ne，codegen 只找 op_eq）这种最难查的错。
//!
//! # 方法名
//!
//! ```text
//! a + b   →  op_add      a - b  →  op_sub     a * b  →  op_mul
//! a / b   →  op_div      a % b  →  op_mod     -a     →  op_neg
//! a == b  →  op_eq       a != b →  op_ne
//! a <  b  →  op_lt       a >  b →  op_gt
//! a <= b  →  op_le       a >= b →  op_ge
//! "{a}"   →  op_str
//! ```
//!
//! 统一加 `op_` 前缀，是为了和普通方法（`add` / `sub` 这类自然名字，
//! 语义往往是「就地累加」）明确分开 —— `a.add(b)` 和 `a + b` 是两件事。
//!
//! # 只写一半比较运算符也能用
//!
//! 比较最容易缺：写了 `op_lt` 和 `op_eq` 之后，`>` / `<=` / `>=` / `!=`
//! 都能推出来，但用户得写四个几乎一样的函数。所以缺的时候自动组合：
//!
//! ```text
//! a >  b  →  b <  a          （op_gt 缺失时用 op_lt 换位）
//! a <= b  →  !(b < a)        （op_le 缺失时）
//! a >= b  →  !(a < b)        （op_ge 缺失时）
//! a != b  →  !(a == b)       （op_ne 缺失时）
//! ```
//!
//! 精确名字优先：写了 `op_ne` 就用它，不套 `!op_eq`。

use crate::ast::{BinOp, UnOp};

/// 插值 / `say` 里的「转成字符串」方法名。
pub const STR_METHOD: &str = "op_str";

/// 一次重载调用的形态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpForm {
    /// `S_op_x(a, b)`
    Direct,
    /// `!S_op_x(a, b)`
    Not,
    /// `S_op_x(b, a)`
    Swapped,
    /// `!S_op_x(b, a)`
    NotSwapped,
}

#[derive(Clone, Copy, Debug)]
pub struct OpCall {
    pub method: &'static str,
    pub form: OpForm,
}

impl OpCall {
    /// 生成的调用里，self 是右操作数而不是左操作数
    pub fn swaps(self) -> bool {
        matches!(self.form, OpForm::Swapped | OpForm::NotSwapped)
    }

    /// 结果要取反（`!=` / `<=` / `>=` 的组合形态）
    pub fn negates(self) -> bool {
        matches!(self.form, OpForm::Not | OpForm::NotSwapped)
    }
}

/// 比较类运算符：结果恒为 bool，对应方法也必须返回 bool。
pub fn is_comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge
    )
}

/// 这个运算符能不能被重载。`and` / `or` / `|>` 不走重载。
pub fn is_overloadable(op: BinOp) -> bool {
    !matches!(op, BinOp::And | BinOp::Or | BinOp::Pipe)
}

/// 解析二元运算符。`has` 回答「这个结构体有没有名为 X 的方法」。
///
/// 返回 None 表示这个运算符没有可用的重载（要么不可重载，要么一个候选
/// 方法都没定义）。调用方据此决定是报错还是落回内建语义。
pub fn resolve_binop(op: BinOp, has: impl Fn(&str) -> bool) -> Option<OpCall> {
    let pick = |method: &'static str, form: OpForm| {
        if has(method) {
            Some(OpCall { method, form })
        } else {
            None
        }
    };
    match op {
        BinOp::Add => pick("op_add", OpForm::Direct),
        BinOp::Sub => pick("op_sub", OpForm::Direct),
        BinOp::Mul => pick("op_mul", OpForm::Direct),
        BinOp::Div => pick("op_div", OpForm::Direct),
        BinOp::Mod => pick("op_mod", OpForm::Direct),
        BinOp::Eq => pick("op_eq", OpForm::Direct),
        BinOp::Ne => pick("op_ne", OpForm::Direct)
            .or_else(|| pick("op_eq", OpForm::Not)),
        BinOp::Lt => pick("op_lt", OpForm::Direct),
        BinOp::Gt => pick("op_gt", OpForm::Direct)
            .or_else(|| pick("op_lt", OpForm::Swapped)),
        BinOp::Le => pick("op_le", OpForm::Direct)
            .or_else(|| pick("op_lt", OpForm::NotSwapped)),
        BinOp::Ge => pick("op_ge", OpForm::Direct)
            .or_else(|| pick("op_lt", OpForm::Not)),
        BinOp::And | BinOp::Or | BinOp::Pipe => None,
    }
}

/// 一元运算符。只有取负能重载（`not` 对结构体没有意义）。
pub fn resolve_unop(op: UnOp, has: impl Fn(&str) -> bool) -> Option<OpCall> {
    match op {
        UnOp::Neg if has("op_neg") => Some(OpCall {
            method: "op_neg",
            form: OpForm::Direct,
        }),
        _ => None,
    }
}

/// 运算符的源码写法，报错信息里用。
pub fn symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Pipe => "|>",
    }
}

/// 「这个运算符至少需要哪个方法」—— 用于「没定义」的报错提示。
///
/// 和 `resolve_binop` 的候选表同源：`>` 的主候选是 `op_gt`，但报错建议
/// 里给 `op_lt` 更省事（写了它 `>` 也能用），所以这里返回推荐写的那个。
pub fn primary_method(op: BinOp) -> Option<&'static str> {
    match op {
        BinOp::Add => Some("op_add"),
        BinOp::Sub => Some("op_sub"),
        BinOp::Mul => Some("op_mul"),
        BinOp::Div => Some("op_div"),
        BinOp::Mod => Some("op_mod"),
        BinOp::Eq | BinOp::Ne => Some("op_eq"),
        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => Some("op_lt"),
        BinOp::And | BinOp::Or | BinOp::Pipe => None,
    }
}

/// 所有会被 codegen 当成运算符的方法名。写普通方法时撞上这些名字
/// 不会报错，但语义会意外地「生效」，提示里要说得清楚。
pub fn method_names() -> &'static [&'static str] {
    &[
        "op_add", "op_sub", "op_mul", "op_div", "op_mod", "op_neg",
        "op_eq", "op_ne", "op_lt", "op_gt", "op_le", "op_ge", "op_str",
    ]
}

/// 名字是不是保留的运算符方法名
pub fn is_op_method(name: &str) -> bool {
    method_names().contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |m: &str| names.contains(&m)
    }

    #[test]
    fn exact_names_win() {
        let c = resolve_binop(BinOp::Ne, only(&["op_eq", "op_ne"])).unwrap();
        assert_eq!(c.method, "op_ne");
        assert!(!c.negates());
    }

    #[test]
    fn ne_falls_back_to_not_eq() {
        let c = resolve_binop(BinOp::Ne, only(&["op_eq"])).unwrap();
        assert_eq!(c.method, "op_eq");
        assert!(c.negates());
    }

    #[test]
    fn gt_swaps_lt() {
        let c = resolve_binop(BinOp::Gt, only(&["op_lt"])).unwrap();
        assert!(c.swaps());
        assert!(!c.negates());
    }

    #[test]
    fn le_negates_and_swaps() {
        let c = resolve_binop(BinOp::Le, only(&["op_lt"])).unwrap();
        assert!(c.swaps());
        assert!(c.negates());
    }

    #[test]
    fn ge_only_negates() {
        let c = resolve_binop(BinOp::Ge, only(&["op_lt"])).unwrap();
        assert!(!c.swaps());
        assert!(c.negates());
    }

    #[test]
    fn arithmetic_needs_exact_method() {
        assert!(resolve_binop(BinOp::Add, only(&["op_sub"])).is_none());
        assert!(resolve_binop(BinOp::Add, only(&["op_add"])).is_some());
    }

    #[test]
    fn logic_operators_are_not_overloadable() {
        assert!(resolve_binop(BinOp::And, only(&["op_add", "op_lt"])).is_none());
        assert!(resolve_binop(BinOp::Pipe, only(&["op_add"])).is_none());
    }

    #[test]
    fn neg_needs_op_neg() {
        assert!(resolve_unop(UnOp::Neg, only(&["op_neg"])).is_some());
        assert!(resolve_unop(UnOp::Neg, only(&["op_sub"])).is_none());
        assert!(resolve_unop(UnOp::Not, only(&["op_neg"])).is_none());
    }
}
