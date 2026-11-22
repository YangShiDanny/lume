use crate::value::Value;
use std::collections::HashMap;

/// 类型ID，对应 LumeC 的类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u64);

/// 方法签名：多个参数的类型id序列
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signature(pub Vec<TypeId>);

/// 一个方法：名字 + 类型签名 -> 编译后的函数入口
#[derive(Debug)]
pub struct MethodTable {
    // (方法名, 签名) => 函数指针
    table: HashMap<(String, Signature), unsafe extern "C" fn() -> Value>,
}

impl MethodTable {
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
        }
    }

    /// 注册一个方法
    pub fn insert(&mut self, name: &str, sig: Signature, f: unsafe extern "C" fn() -> Value) {
        self.table.insert((name.to_string(), sig), f);
    }

    /// 根据名字 + 参数的值，动态构造签名并查找（多重派发核心）
    pub fn lookup(&self, name: &str, args: &[Value]) -> Option<unsafe extern "C" fn() -> Value> {
        let mut tys = Vec::new();
        for arg in args {
            let tid = if arg.is_int() {
                TypeId(1)
            } else if arg.is_bool() {
                TypeId(2)
            } else if arg.is_float() {
                TypeId(3)
            } else if arg.is_ptr() {
                TypeId(4)
            } else {
                panic!("unknown value type");
            };
            tys.push(tid);
        }
        let sig = Signature(tys);
        self.table.get(&(name.to_string(), sig)).copied()
    }
}
