// src/builtins.rs
//! 内置函数的唯一真相来源。
//!
//! 以前加一个内置函数要同时改四处：checker 的白名单（`is_builtin`）、
//! checker 的返回类型（`builtin_return_type`）、codegen 的名字映射
//! （`builtin_map`）、say 的 C 名映射（`builtin_c_name`）。漏一处的后果
//! 各不相同，而且都不会报错：
//!
//! - 漏 checker 白名单 → 报「未定义的函数」（还算好）
//! - 漏 checker 返回类型 → 返回 `Ty::Unknown`，类型检查静默放行
//! - 漏 codegen 名字映射 → 生成 `str_upper(x)` 这种不存在的 C 调用，
//!   要到 clang 那一步才炸，且报错指向生成的 C 不是源文件
//! - 漏 say 的映射 → 插值里的 `%s` / `%lld` 选错，varargs 直接 UB
//!
//! 所以收敛成一张表。加新函数只改这里 + runtime 里的 C 实现。

use crate::checker::Ty;

/// 返回类型的分类。一份定义同时给出 C 类型、checker 类型和 printf 格式。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ret {
    Int,
    Float,
    Bool,
    Str,
    Void,
    /// `const char**` + 一个 int64_t* 的伴随长度出参（只有 str_split）
    StrArray,
    /// `str?`。失败返回 None，不靠 errno 也不靠空串猜。
    OptStr,
}

impl Ret {
    /// 生成的 C 代码里这个返回值的类型
    pub fn c_type(self) -> &'static str {
        match self {
            Ret::Int => "int64_t",
            Ret::Float => "double",
            Ret::Bool => "bool",
            Ret::Str => "const char*",
            Ret::Void => "void",
            Ret::StrArray => "const char**",
            // 注意：这不是 `str?` 在 C 里的真实类型。可空值的 C 类型是
            // `lume_opt_N`（或字符串专用的 `LumeOptStr`），tag 由 codegen
            // 的 `opt_tag()` 惰性分配，静态表里给不出来。这里返回元素类型
            // 只是为了让别名一致性测试有个可比的值 —— register_builtin_sigs
            // 对 OptStr 走的是 `opt_tag("const char*")` 那条路。
            Ret::OptStr => "const char*",
        }
    }

    /// 进 printf 格式串。bool 必须是 %d —— varargs 里 bool 提升成 int，
    /// 用 %lld 会读 8 字节，是 UB。
    pub fn c_fmt(self) -> &'static str {
        match self {
            Ret::Int => "%lld",
            Ret::Float => "%f",
            Ret::Bool => "%d",
            Ret::Str => "%s",
            Ret::Void => "",
            Ret::StrArray => "%p",
            // 可空值不能直接进 varargs（那是结构体，%s 会读到垃圾）。
            // checker 在插值和 say 的参数位置都会拦下来，这里给 %s 只是
            // 让「万一漏了」时的输出至少不是随机的指针宽度错位。
            Ret::OptStr => "%s",
        }
    }

    /// checker 眼里的类型
    pub fn ty(self) -> Ty {
        match self {
            Ret::Int => Ty::Int,
            Ret::Float => Ty::Float,
            Ret::Bool => Ty::Bool,
            Ret::Str => Ty::Str,
            Ret::Void => Ty::Void,
            Ret::StrArray => Ty::Array(Box::new(Ty::Str)),
            Ret::OptStr => Ty::Optional(Box::new(Ty::Str)),
        }
    }
}

pub struct Builtin {
    /// Lume 源码里写的名字
    pub name: &'static str,
    /// 生成的 C 函数名
    pub c_name: &'static str,
    pub ret: Ret,
    /// true = 编译器内部用（optimize 生成的中间调用），
    /// 用户源码里写这个名字应当报「未定义的函数」
    pub hidden: bool,
}

const fn bi(name: &'static str, c_name: &'static str, ret: Ret) -> Builtin {
    Builtin { name, c_name, ret, hidden: false }
}

const fn internal(name: &'static str, c_name: &'static str, ret: Ret) -> Builtin {
    Builtin { name, c_name, ret, hidden: true }
}

pub const TABLE: &[Builtin] = &[
    // ── std.str：查询 ──
    bi("str_len", "lume_str_len", Ret::Int),
    bi("str_to_int", "lume_str_to_int", Ret::Int),
    bi("str_to_float", "lume_str_to_float", Ret::Float),
    bi("str_contains", "lume_str_contains", Ret::Bool),
    bi("str_starts_with", "lume_str_starts_with", Ret::Bool),
    bi("str_ends_with", "lume_str_ends_with", Ret::Bool),
    bi("str_index_of", "lume_str_index_of", Ret::Int),
    // ── std.str：派生（结果进 arena，随便存多久）──
    bi("str_upper", "lume_str_upper", Ret::Str),
    bi("str_lower", "lume_str_lower", Ret::Str),
    bi("str_trim", "lume_str_trim", Ret::Str),
    bi("str_replace", "lume_str_replace", Ret::Str),
    bi("str_slice", "lume_str_slice", Ret::Str),
    bi("str_repeat", "lume_str_repeat", Ret::Str),
    bi("str_split", "lume_str_split", Ret::StrArray),
    // 释放整个 arena。所有 str 变成悬垂指针，只在「一批临时字符串
    // 已经用完」时调（比如解析完一个文件）。
    bi("str_arena_reset", "lume_str_arena_reset", Ret::Void),
    // 当前 arena 已用字节数。测 `scope` 有没有真的回收、以及盯住
    // 长期运行的循环有没有无界增长（自举时很有用）。
    bi("str_arena_bytes", "lume_str_arena_bytes", Ret::Int),
    // ── int ──
    bi("int_abs", "lume_int_abs", Ret::Int),
    bi("int_min", "lume_int_min", Ret::Int),
    bi("int_max", "lume_int_max", Ret::Int),
    bi("int_to_str", "lume_int_str", Ret::Str),
    // ── float ──
    bi("float_abs", "lume_float_abs", Ret::Float),
    bi("float_sqrt", "lume_float_sqrt", Ret::Float),
    bi("sqrt", "lume_float_sqrt", Ret::Float),
    bi("float_pow", "lume_float_pow", Ret::Float),
    bi("pow", "lume_float_pow", Ret::Float),
    bi("float_sin", "lume_float_sin", Ret::Float),
    bi("sin", "lume_float_sin", Ret::Float),
    bi("float_cos", "lume_float_cos", Ret::Float),
    bi("cos", "lume_float_cos", Ret::Float),
    bi("float_tan", "lume_float_tan", Ret::Float),
    bi("tan", "lume_float_tan", Ret::Float),
    bi("float_floor", "lume_float_floor", Ret::Float),
    bi("floor", "lume_float_floor", Ret::Float),
    bi("float_ceil", "lume_float_ceil", Ret::Float),
    bi("ceil", "lume_float_ceil", Ret::Float),
    bi("float_round", "lume_float_round", Ret::Float),
    bi("round", "lume_float_round", Ret::Float),
    bi("float_to_str", "lume_float_str", Ret::Str),
    // ── 内部：optimize 生成，用户写不出来 ──
    internal("__shl", "lume_shl", Ret::Int),
    internal("__bitand", "lume_bitand", Ret::Int),
    // ── std.os：文件 ──
    //
    // read_file 失败返回 None，而不是空串 —— 空文件是合法内容，
    // 用 "" 表示失败就再也分不清「读到了空文件」和「打不开」。
    // 这是自举的第一步：编译器要把整个源文件吃进来。
    //
    // 读到的内容从 arena 切（和 str_upper / 拼接同类），生命周期跟
    // arena 走，不用也不该手动 free。
    bi("read_file", "lume_file_read", Ret::OptStr),
    bi("write_file", "lume_file_write", Ret::Bool),
    bi("append_file", "lume_file_append", Ret::Bool),
    bi("file_exists", "lume_file_exists", Ret::Bool),
    bi("remove_file", "lume_file_remove", Ret::Bool),
    // ── std.os：进程 ──
    bi("arg_count", "lume_arg_count", Ret::Int),
    bi("arg", "lume_arg", Ret::Str),
    bi("exit", "lume_exit", Ret::Void),
];

/// 按 Lume 名字查表（含内部函数）。
pub fn lookup(name: &str) -> Option<&'static Builtin> {
    TABLE.iter().find(|b| b.name == name)
}

/// 用户能不能在源码里写这个名字。checker 的白名单用这个，
/// 所以 `__shl` 在用户代码里会正常报「未定义的函数」。
pub fn is_user_visible(name: &str) -> bool {
    TABLE.iter().any(|b| b.name == name && !b.hidden)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表里不能有重名 —— 重名会让 lookup 静默返回第一条，
    /// 于是 alias 悄悄失效。
    #[test]
    fn no_duplicate_names() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                assert_ne!(a.name, b.name, "内置函数重名：{}", a.name);
            }
        }
    }

    /// 同名的 C 目标必须返回类型一致（`sqrt` / `float_sqrt` 这类别名）。
    #[test]
    fn aliases_agree_on_return_type() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                if a.c_name == b.c_name {
                    assert_eq!(
                        a.ret.c_type(),
                        b.ret.c_type(),
                        "{} 和 {} 指向同一个 C 函数但返回类型不同",
                        a.name,
                        b.name
                    );
                }
            }
        }
    }

    #[test]
    fn internal_fns_are_hidden_from_users() {
        assert!(!is_user_visible("__shl"));
        assert!(!is_user_visible("__bitand"));
        assert!(is_user_visible("str_len"));
        assert!(is_user_visible("int_to_str"));
    }
}
