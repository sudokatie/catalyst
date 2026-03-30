//! Target definitions and attribute value types

use std::collections::HashMap;
use std::path::PathBuf;

use crate::Label;

/// A buildable target defined in a BUILD file.
#[derive(Debug, Clone)]
pub struct Target {
    /// Unique label for this target
    pub label: Label,
    /// Rule type (e.g., "rust_binary", "genrule")
    pub rule_type: String,
    /// Source files
    pub srcs: Vec<PathBuf>,
    /// Dependencies on other targets
    pub deps: Vec<Label>,
    /// Output files (derived from rule)
    pub outs: Vec<PathBuf>,
    /// Additional attributes
    pub attrs: HashMap<String, Value>,
}

impl Target {
    /// Create a new target with the given label and rule type
    pub fn new(label: Label, rule_type: impl Into<String>) -> Self {
        Self {
            label,
            rule_type: rule_type.into(),
            srcs: Vec::new(),
            deps: Vec::new(),
            outs: Vec::new(),
            attrs: HashMap::new(),
        }
    }

    /// Add a source file
    pub fn add_src(&mut self, path: PathBuf) {
        self.srcs.push(path);
    }

    /// Add a dependency
    pub fn add_dep(&mut self, label: Label) {
        self.deps.push(label);
    }

    /// Add an output file
    pub fn add_out(&mut self, path: PathBuf) {
        self.outs.push(path);
    }

    /// Set an attribute value
    pub fn set_attr(&mut self, key: impl Into<String>, value: Value) {
        self.attrs.insert(key.into(), value);
    }

    /// Get an attribute value
    pub fn get_attr(&self, key: &str) -> Option<&Value> {
        self.attrs.get(key)
    }
}

/// Attribute value types for BUILD files.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// String value
    String(String),
    /// Integer value
    Int(i64),
    /// Boolean value
    Bool(bool),
    /// List of values
    List(Vec<Value>),
    /// Dictionary of string keys to values
    Dict(HashMap<String, Value>),
}

impl Value {
    /// Try to get as string
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as integer
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Try to get as boolean
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get as list
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    /// Try to get as dict
    pub fn as_dict(&self) -> Option<&HashMap<String, Value>> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_creation() {
        let label = Label::new("pkg", "mylib");
        let target = Target::new(label.clone(), "rust_library");

        assert_eq!(target.label, label);
        assert_eq!(target.rule_type, "rust_library");
        assert!(target.srcs.is_empty());
        assert!(target.deps.is_empty());
    }

    #[test]
    fn target_add_sources() {
        let label = Label::new("pkg", "mylib");
        let mut target = Target::new(label, "rust_library");

        target.add_src(PathBuf::from("src/lib.rs"));
        target.add_src(PathBuf::from("src/util.rs"));

        assert_eq!(target.srcs.len(), 2);
        assert_eq!(target.srcs[0], PathBuf::from("src/lib.rs"));
    }

    #[test]
    fn target_add_deps() {
        let label = Label::new("pkg", "mylib");
        let mut target = Target::new(label, "rust_library");

        target.add_dep(Label::new("other", "dep1"));
        target.add_dep(Label::new("other", "dep2"));

        assert_eq!(target.deps.len(), 2);
    }

    #[test]
    fn target_attributes() {
        let label = Label::new("pkg", "mylib");
        let mut target = Target::new(label, "rust_library");

        target.set_attr("visibility", Value::String("public".to_string()));
        target.set_attr("opt_level", Value::Int(3));

        assert_eq!(
            target.get_attr("visibility"),
            Some(&Value::String("public".to_string()))
        );
        assert_eq!(target.get_attr("opt_level"), Some(&Value::Int(3)));
        assert_eq!(target.get_attr("missing"), None);
    }

    #[test]
    fn value_conversions() {
        let s: Value = "hello".into();
        assert_eq!(s.as_str(), Some("hello"));

        let i: Value = 42i64.into();
        assert_eq!(i.as_int(), Some(42));

        let b: Value = true.into();
        assert_eq!(b.as_bool(), Some(true));

        let l: Value = vec!["a", "b"].into();
        assert!(l.as_list().is_some());
    }

    #[test]
    fn value_type_checks() {
        let s = Value::String("test".to_string());
        assert!(s.as_str().is_some());
        assert!(s.as_int().is_none());
        assert!(s.as_bool().is_none());
        assert!(s.as_list().is_none());
    }
}
