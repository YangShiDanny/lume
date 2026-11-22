use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Any,
    Int64,
    Float64,
    Bool,
    String,
    Array,
    Function,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct TypeId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeObj {
    pub id: TypeId,
    pub name: String,
    pub kind: TypeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeError(pub String);

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TypeError {}

#[derive(Debug)]
pub struct TypeRegistry {
    by_name: HashMap<String, TypeObj>,
    next_id: u64,
}

impl Default for TypeRegistry {
    fn default() -> Self {
        let mut registry = Self {
            by_name: HashMap::new(),
            next_id: 0,
        };
        for (name, kind) in [
            ("Any", TypeKind::Any),
            ("Int64", TypeKind::Int64),
            ("Float64", TypeKind::Float64),
            ("Bool", TypeKind::Bool),
            ("String", TypeKind::String),
            ("Array", TypeKind::Array),
            ("Function", TypeKind::Function),
        ] {
            registry
                .register(name, kind)
                .expect("builtin type names are unique");
        }
        registry
    }
}

impl TypeRegistry {
    pub fn register(
        &mut self,
        name: impl Into<String>,
        kind: TypeKind,
    ) -> Result<TypeId, TypeError> {
        let name = name.into();
        if name.is_empty() {
            return Err(TypeError("type name must not be empty".into()));
        }
        if self.by_name.contains_key(&name) {
            return Err(TypeError(format!("type `{name}` is already registered")));
        }
        let id = TypeId(self.next_id);
        self.next_id += 1;
        self.by_name
            .insert(name.clone(), TypeObj { id, name, kind });
        Ok(id)
    }

    pub fn get(&self, name: &str) -> Option<&TypeObj> {
        self.by_name.get(name)
    }

    pub fn check(&self, expected: TypeId, actual: TypeId) -> Result<(), TypeError> {
        let expected_type = self
            .by_name
            .values()
            .find(|ty| ty.id == expected)
            .ok_or_else(|| TypeError(format!("unknown expected type id {}", expected.0)))?;
        let actual_type = self
            .by_name
            .values()
            .find(|ty| ty.id == actual)
            .ok_or_else(|| TypeError(format!("unknown actual type id {}", actual.0)))?;
        if expected == actual || expected_type.kind == TypeKind::Any {
            return Ok(());
        }
        Err(TypeError(format!(
            "type mismatch: expected {}, got {}",
            expected_type.name, actual_type.name
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_types_and_lightweight_compatibility_check_work() {
        let mut types = TypeRegistry::default();
        let any = types.get("Any").unwrap().id;
        let int = types.get("Int64").unwrap().id;
        assert!(types.check(any, int).is_ok());
        assert!(types.check(int, any).is_err());
        assert!(types.register("Int64", TypeKind::Int64).is_err());
    }
}
