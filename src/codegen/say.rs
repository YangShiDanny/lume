// src/codegen/say.rs
// say 的插值在解析期就已经变成 AST（Expr::FmtStr），这里只负责：
// 1. 按 AST 推断每个表达式的 printf 格式串
// 2. 生成对应的 C 表达式（含必要的强制转换）
use super::Codegen;
use crate::ast::*;

impl Codegen {
    pub(crate) fn gen_say(&mut self, args: &[Expr]) -> String {
        self.gen_say_with(args, None)
    }

    // fields = Some(...) 表示当前在 struct 方法里，裸字段要映射成 _self->field
    pub(crate) fn gen_say_with(
        &mut self,
        args: &[Expr],
        fields: Option<&[String]>,
    ) -> String {
        let mut format_str = String::new();
        let mut format_args: Vec<String> = Vec::new();

        for arg in args {
            match arg {
                Expr::Str(s) => {
                    format_str.push_str(&escape_fmt(s));
                }
                Expr::FmtStr(parts) => {
                    for part in parts {
                        match part {
                            StrPart::Text(t) => format_str.push_str(&escape_fmt(t)),
                            StrPart::Expr(e) => {
                                let fmt = self.format_for_expr(e);
                                format_str.push_str(fmt);
                                format_args.push(self.say_arg(e, fmt, fields));
                            }
                        }
                    }
                }
                _ => {
                    let fmt = self.format_for_expr(arg);
                    format_str.push_str(fmt);
                    format_args.push(self.say_arg(arg, fmt, fields));
                }
            }
        }

        format_str.push_str("\\n");

        if format_args.is_empty() {
            format!("printf(\"{}\")", format_str)
        } else {
            format!("printf(\"{}\", {})", format_str, format_args.join(", "))
        }
    }

    // 生成 printf 的实参，并按格式串加上显式转换，避免 varargs 类型不匹配
    fn say_arg(&mut self, e: &Expr, fmt: &str, fields: Option<&[String]>) -> String {
        let code = match fields {
            Some(f) => self.gen_expr_with_self(e, f),
            None => self.gen_expr(e),
        };
        if code.starts_with("printf(") || code.contains("/* ") {
            return code;
        }
        match fmt {
            "%s" => code,
            "%lld" => format!("(long long)({})", code),
            "%llu" => format!("(unsigned long long)({})", code),
            "%f" => format!("(double)({})", code),
            "%d" => format!("(int)({})", code),
            "%u" | "%c" | "0x%06X" => format!("(unsigned int)({})", code),
            _ => code,
        }
    }

    // ── 格式串推断（走 AST，不再对源码文本做启发式匹配）──

    fn format_for_expr(&self, expr: &Expr) -> &'static str {
        match expr {
            Expr::Str(_) | Expr::FmtStr(_) | Expr::Nothing => "%s",
            Expr::Int(_) => "%lld",
            Expr::Float(_) => "%f",
            Expr::Bool(_) => "%d",
            Expr::Rune(_) => "%c",
            Expr::Color(_) => "0x%06X",
            Expr::Call(callee, _) => {
                // 内置函数按返回类型
                if let Expr::Ident(name) = callee.as_ref() {
                    if builtin_c_name(name).is_some() {
                        return builtin_ret_fmt(name);
                    }
                }
                Self::fmt_for_c_ty(&self.infer_type(expr))
            }
            _ => Self::fmt_for_c_ty(&self.infer_type(expr)),
        }
    }

    fn fmt_for_c_ty(ty: &str) -> &'static str {
        match ty {
            "const char*" | "char*" => "%s",
            "double" | "float" => "%f",
            "bool" => "%d",
            "uint8_t" | "uint16_t" | "uint32_t" => "%u",
            "uint64_t" => "%llu",
            "int8_t" | "int16_t" | "int32_t" => "%d",
            _ => "%lld",
        }
    }
}

// C 字符串字面量转义（% 也要转成 %%，因为它进的是 printf 格式串）
fn escape_fmt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out
}

// ── 内置函数映射 ──

fn builtin_c_name(name: &str) -> Option<&'static str> {
    Some(match name {
        // str
        "str_len" => "lume_str_len",
        "str_upper" => "lume_str_upper",
        "str_lower" => "lume_str_lower",
        "str_trim" => "lume_str_trim",
        "str_to_int" => "lume_str_to_int",
        "str_to_float" => "lume_str_to_float",
        // int
        "int_abs" => "lume_int_abs",
        "int_min" => "lume_int_min",
        "int_max" => "lume_int_max",
        // float
        "float_sqrt" | "sqrt" => "lume_float_sqrt",
        "float_abs" => "lume_float_abs",
        "float_pow" | "pow" => "lume_float_pow",
        "float_sin" | "sin" => "lume_float_sin",
        "float_cos" | "cos" => "lume_float_cos",
        "float_tan" | "tan" => "lume_float_tan",
        "float_floor" | "floor" => "lume_float_floor",
        "float_ceil" | "ceil" => "lume_float_ceil",
        "float_round" | "round" => "lume_float_round",
        _ => return None,
    })
}

fn builtin_ret_fmt(name: &str) -> &'static str {
    match name {
        "str_len" | "str_to_int" | "int_abs" | "int_min" | "int_max" => "%lld",
        "str_upper" | "str_lower" | "str_trim" => "%s",
        "str_to_float" | "float_sqrt" | "sqrt" | "float_abs"
        | "float_pow" | "pow" | "float_sin" | "sin" | "float_cos" | "cos"
        | "float_tan" | "tan" | "float_floor" | "floor" | "float_ceil" | "ceil"
        | "float_round" | "round" => "%f",
        _ => "%lld",
    }
}
