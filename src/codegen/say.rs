// src/codegen/say.rs
// say 的插值在解析期就已经变成 AST（Expr::FmtStr），这里只负责：
// 1. 按 AST 推断每个表达式的 printf 格式串
// 2. 生成对应的 C 表达式（含必要的强制转换）
use super::Codegen;
use crate::ast::*;

impl Codegen {
    pub(crate) fn gen_say(&mut self, args: &[Expr]) -> String {
        self.gen_output_with(args, None, true, false)
    }

    pub(crate) fn gen_write(&mut self, args: &[Expr]) -> String {
        self.gen_output_with(args, None, false, false)
    }

    /// `eprint` / `ewrite`：同样的格式串逻辑，出口换成 stderr（同步写）。
    pub(crate) fn gen_eprint(&mut self, args: &[Expr]) -> String {
        self.gen_output_with(args, None, true, true)
    }

    pub(crate) fn gen_ewrite(&mut self, args: &[Expr]) -> String {
        self.gen_output_with(args, None, false, true)
    }

    // fields = Some(...) 表示当前在 struct 方法里，裸字段要映射成 _self->field
    pub(crate) fn gen_say_with(
        &mut self,
        args: &[Expr],
        fields: Option<&[String]>,
    ) -> String {
        self.gen_output_with(args, fields, true, false)
    }

    pub(crate) fn gen_write_with(
        &mut self,
        args: &[Expr],
        fields: Option<&[String]>,
    ) -> String {
        self.gen_output_with(args, fields, false, false)
    }

    pub(crate) fn gen_eprint_with(
        &mut self,
        args: &[Expr],
        fields: Option<&[String]>,
    ) -> String {
        self.gen_output_with(args, fields, true, true)
    }

    pub(crate) fn gen_ewrite_with(
        &mut self,
        args: &[Expr],
        fields: Option<&[String]>,
    ) -> String {
        self.gen_output_with(args, fields, false, true)
    }

    fn gen_output_with(
        &mut self,
        args: &[Expr],
        fields: Option<&[String]>,
        newline: bool,
        to_err: bool,
    ) -> String {
        let mut format_str = String::new();
        let mut format_args: Vec<String> = Vec::new();
        let mut literal_text = String::new();
        let mut literal_only = true;

        for arg in args {
            match arg {
                Expr::Str(s) => {
                    format_str.push_str(&escape_fmt(s));
                    literal_text.push_str(s);
                }
                Expr::FmtStr(parts) => {
                    for part in parts {
                        match part {
                            StrPart::Text(t) => {
                                format_str.push_str(&escape_fmt(t));
                                literal_text.push_str(t);
                            }
                            StrPart::Expr(e) => {
                                literal_only = false;
                                let fmt = self.format_for_expr(e);
                                format_str.push_str(fmt);
                                format_args.push(self.say_arg(e, fmt, fields));
                            }
                        }
                    }
                }
                _ => {
                    literal_only = false;
                    let fmt = self.format_for_expr(arg);
                    format_str.push_str(fmt);
                    format_args.push(self.say_arg(arg, fmt, fields));
                }
            }
        }

        if newline {
            format_str.push_str("\\n");
            literal_text.push('\n');
        }

        // stdout 走异步日志队列，stderr 同步直写（错误信息不能等）
        let (literal_fn, printf_fn) = if to_err {
            ("lume_eprint_literal", "lume_eprintf")
        } else {
            ("lume_log_write_literal", "lume_logf")
        };

        if literal_only {
            return format!(
                "{}(\"{}\", {}U)",
                literal_fn,
                escape_c_string(&literal_text),
                literal_text.len()
            );
        }

        if format_args.is_empty() {
            format!("{}(\"{}\")", printf_fn, format_str)
        } else {
            format!("{}(\"{}\", {})", printf_fn, format_str, format_args.join(", "))
        }
    }

    // 生成 printf 的实参，并按格式串加上显式转换，避免 varargs 类型不匹配
    fn say_arg(&mut self, e: &Expr, fmt: &str, fields: Option<&[String]>) -> String {
        let code = match fields {
            Some(f) => self.gen_expr_with_self(e, f),
            None => self.gen_expr(e),
        };
        // 带 op_str 的结构体：按字符串打印，格式串由 format_for_expr 给 %s。
        // 这里必须在类型分支之前拦 —— 结构体进 varargs 读的是第一个字段。
        if let Some(sname) = self.op_str_struct(e) {
            return self.gen_op_str_call(e, &code, &sname);
        }
        // i128/u128 没有 printf 格式符，转成字符串再 %s
        let cty = self.infer_type(e);
        if cty == "__int128" {
            return format!("lume_i128_str({})", code);
        }
        if cty == "unsigned __int128" {
            return format!("lume_u128_str({})", code);
        }
        if code.starts_with("lume_logf(")
            || code.starts_with("lume_eprintf(")
            || code.contains("/* ")
        {
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
        // 带 op_str 的结构体先判 —— 否则会落到 fmt_for_c_ty 的 %lld 兜底，
        // 把结构体当整数塞进 varargs。
        if self.op_str_struct(expr).is_some() {
            return "%s";
        }
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

fn escape_c_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

// ── 内置函数：查 crate::builtins 那张唯一的表 ──

fn builtin_c_name(name: &str) -> Option<&'static str> {
    crate::builtins::lookup(name).map(|b| b.c_name)
}

fn builtin_ret_fmt(name: &str) -> &'static str {
    match crate::builtins::lookup(name) {
        Some(b) => b.ret.c_fmt(),
        None => "%lld",
    }
}
