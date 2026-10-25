// src/codegen/ops.rs
//! 运算符重载的 C 代码生成。
//!
//! 解析（哪个方法、要不要换位/取反）全在 `crate::ops`，这里只负责把它
//! 变成 C 调用。三处独立的二元生成器（`gen_expr` / `gen_expr_with_self` /
//! `parallel::gen_par_expr`）都调同一个入口 —— 以前字符串特化就是只改了
//! 两处，`par for` 里生成 C 指针加法的事故，见 `gen_str_binop` 的注释。
//!
//! # 为什么要「左值化」
//!
//! 方法在 C 里是 `S_op_add(S* _self, S o)` —— 第一个参数是**指针**。
//! `a + b` 里 a 是变量，`&a` 没问题；但 `(a + b) + c` 的左边是函数返回值，
//! `&S_op_add(...)` 在 C 里不合法（取临时值地址）。所以左操作数不是左值时，
//! 先落进一个临时变量再取址：
//!
//! ```c
//! ({ Big _op_self3 = Big_op_add(&a, b); Big_op_add(&_op_self3, c); })
//! ```
//!
//! 用的是 GNU 语句表达式，clang 和 tcc 都支持（`!` / `?` 早就在用）。

use super::Codegen;
use crate::ast::*;
use crate::ops::{self, OpCall};

impl Codegen {
    /// 二元运算符重载。`Some(..)` 表示已经处理完，`None` 交回内建规则。
    ///
    /// `ls` / `rs` 是调用方已经生成好的操作数 C 代码。
    pub(crate) fn gen_op_overload(
        &mut self,
        l: &Expr,
        op: BinOp,
        r: &Expr,
        ls: &str,
        rs: &str,
    ) -> Option<String> {
        let cty = self.infer_type(l);
        if !self.structs.contains_key(&cty) {
            return None;
        }
        let call = ops::resolve_binop(op, |m| {
            self.methods
                .contains_key(&(cty.clone(), m.to_string()))
        })?;
        // Swapped 形态（`a > b` 借用 `b < a`）时，self 是右操作数。
        let (self_expr, self_code, other_code): (&Expr, &str, &str) = if call.swaps() {
            (r, rs, ls)
        } else {
            (l, ls, rs)
        };
        Some(self.gen_op_invoke(&cty, call, self_expr, self_code, other_code))
    }

    /// 一元运算符重载（只有取负）。
    pub(crate) fn gen_unop_overload(
        &mut self,
        op: UnOp,
        e: &Expr,
        es: &str,
    ) -> Option<String> {
        let cty = self.infer_type(e);
        if !self.structs.contains_key(&cty) {
            return None;
        }
        let call = ops::resolve_unop(op, |m| {
            self.methods
                .contains_key(&(cty.clone(), m.to_string()))
        })?;
        Some(self.gen_op_invoke(&cty, call, e, es, ""))
    }

    /// 表达式是「带 op_str 的结构体」时，返回结构体 C 名。
    ///
    /// 插值 / `say` 靠它把 `%lld` 换成 `%s`，并把实参换成 `S_op_str(&x)`。
    pub(crate) fn op_str_struct(&self, e: &Expr) -> Option<String> {
        let cty = self.infer_type(e);
        if !self.structs.contains_key(&cty) {
            return None;
        }
        let ret = self
            .methods
            .get(&(cty.clone(), ops::STR_METHOD.to_string()))?;
        if ret != "const char*" {
            return None;
        }
        Some(cty)
    }

    /// 生成 `S_op_str(&x)`。`code` 是调用方已经生成好的 x。
    pub(crate) fn gen_op_str_call(&mut self, e: &Expr, code: &str, sname: &str) -> String {
        let call = OpCall {
            method: ops::STR_METHOD,
            form: ops::OpForm::Direct,
        };
        self.gen_op_invoke(sname, call, e, code, "")
    }

    /// 组装 `S_op_x(&self, other)`，必要时先落临时变量。
    fn gen_op_invoke(
        &mut self,
        sname: &str,
        call: OpCall,
        self_expr: &Expr,
        self_code: &str,
        other_code: &str,
    ) -> String {
        let fname = format!("{}_{}", sname, call.method);
        let body = if Self::is_addressable(self_expr) {
            // 左操作数是左值：直接取址
            if other_code.is_empty() {
                format!("{}(&{})", fname, self_code)
            } else {
                format!("{}(&{}, {})", fname, self_code, other_code)
            }
        } else {
            // 左操作数没有地址（`(a + b) + c`、`f().x + y`）。语句表达式
            // 给个块作用域，临时变量不会和内层的撞名。
            let id = self.tmp_counter;
            self.tmp_counter += 1;
            let tmp = format!("_op_self{}", id);
            if other_code.is_empty() {
                format!(
                    "({{ {} {} = {}; {}(&{}); }})",
                    sname, tmp, self_code, fname, tmp
                )
            } else {
                format!(
                    "({{ {} {} = {}; {}(&{}, {}); }})",
                    sname, tmp, self_code, fname, tmp, other_code
                )
            }
        };
        if call.negates() {
            format!("!({})", body)
        } else {
            body
        }
    }

    /// 能不能在 C 里对这个表达式取地址（`&e` 合法）。
    ///
    /// 只看语法形状：变量、成员访问、下标都是左值。复合字面量/函数调用不是。
    /// 递归检查基表达式 —— `f().x` 的 `.x` 看着像成员访问，但基没有地址，
    /// `&f().x` 同样不合法。
    pub(crate) fn is_addressable(e: &Expr) -> bool {
        match e {
            // `self` 生成 `(*_self)`，`&(*_self)` 合法
            Expr::Ident(_) => true,
            Expr::Field(base, _) => Self::is_addressable(base),
            Expr::Index(base, _) => Self::is_addressable(base),
            _ => false,
        }
    }
}
