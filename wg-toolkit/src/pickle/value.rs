//! Pickled values.

use std::collections::{BTreeMap, BTreeSet};
use std::cmp::Ordering;
use std::fmt;


/// A pickled value. Every value can be a dict key or set item: they are totally
/// ordered, floats by [`f64::total_cmp`].
#[derive(Debug, Clone, Default)]
pub enum Value {
    #[default]
    None,
    Bool(bool),
    I64(i64),
    F64(f64),
    /// A Python 2 `str`.
    Bytes(Vec<u8>),
    /// A Python 2 `unicode`.
    String(String),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Set(BTreeSet<Value>),
    FrozenSet(BTreeSet<Value>),
    Dict(BTreeMap<Value, Value>),
    /// A class or function, by name.
    Global(Global),
    Object(Box<Object>),
}

/// A class or function, named by its module and its name in it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Global {
    pub module: String,
    pub name: String,
}

/// An object, created by calling its class (or a function) with arguments, then given
/// its state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Object {
    pub class: Global,
    pub args: Vec<Value>,
    /// Created with `class.__new__(class, *args)`, as protocol 2 does for new-style
    /// classes, rather than `class(*args)`.
    pub new: bool,
    /// The state given to `__setstate__`, or merged into the object's `__dict__`.
    pub state: Option<Value>,
}

impl Global {

    pub fn new(module: impl Into<String>, name: impl Into<String>) -> Self {
        Self { module: module.into(), name: name.into() }
    }

}

impl Value {

    /// A dict of the given entries, keyed by their names as Python 2 `str`.
    pub fn dict<'a>(entries: impl IntoIterator<Item = (&'a str, Value)>) -> Self {
        Value::Dict(entries.into_iter().map(|(key, value)| (Value::from(key), value)).collect())
    }

    /// An object of the given class, created as protocol 2 does, without arguments,
    /// and with the given state.
    pub fn object(class: Global, state: Value) -> Self {
        Value::Object(Box::new(Object { class, args: Vec::new(), new: true, state: Some(state) }))
    }

    fn rank(&self) -> u8 {
        match self {
            Value::None => 0,
            Value::Bool(_) => 1,
            Value::I64(_) => 2,
            Value::F64(_) => 3,
            Value::Bytes(_) => 4,
            Value::String(_) => 5,
            Value::List(_) => 6,
            Value::Tuple(_) => 7,
            Value::Set(_) => 8,
            Value::FrozenSet(_) => 9,
            Value::Dict(_) => 10,
            Value::Global(_) => 11,
            Value::Object(_) => 12,
        }
    }

}

/// A Python 2 `str`, as the game's dict keys are.
impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::Bytes(value.as_bytes().to_vec())
    }
}

impl Ord for Value {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Value::None, Value::None) => Ordering::Equal,
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::I64(a), Value::I64(b)) => a.cmp(b),
            (Value::F64(a), Value::F64(b)) => a.total_cmp(b),
            (Value::Bytes(a), Value::Bytes(b)) => a.cmp(b),
            (Value::String(a), Value::String(b)) => a.cmp(b),
            (Value::List(a), Value::List(b)) | (Value::Tuple(a), Value::Tuple(b)) => a.cmp(b),
            (Value::Set(a), Value::Set(b)) | (Value::FrozenSet(a), Value::FrozenSet(b)) => a.cmp(b),
            (Value::Dict(a), Value::Dict(b)) => a.cmp(b),
            (Value::Global(a), Value::Global(b)) => a.cmp(b),
            (Value::Object(a), Value::Object(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Value {}

/// Python's `repr` of the value.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn seq<'a>(f: &mut fmt::Formatter<'_>, items: impl IntoIterator<Item = &'a Value>) -> fmt::Result {
            for (i, item) in items.into_iter().enumerate() {
                if i != 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{item}")?;
            }
            Ok(())
        }
        match self {
            Value::None => f.write_str("None"),
            Value::Bool(true) => f.write_str("True"),
            Value::Bool(false) => f.write_str("False"),
            Value::I64(value) => write!(f, "{value}"),
            Value::F64(value) => write!(f, "{value:?}"),
            Value::Bytes(value) => write!(f, "'{}'", value.escape_ascii()),
            Value::String(value) => write!(f, "u'{}'", value.escape_debug()),
            Value::List(items) => {
                f.write_str("[")?;
                seq(f, items)?;
                f.write_str("]")
            }
            Value::Tuple(items) => {
                f.write_str("(")?;
                seq(f, items)?;
                f.write_str(if items.len() == 1 { ",)" } else { ")" })
            }
            Value::Set(items) | Value::FrozenSet(items) => {
                f.write_str(if matches!(self, Value::Set(_)) { "set([" } else { "frozenset([" })?;
                seq(f, items)?;
                f.write_str("])")
            }
            Value::Dict(entries) => {
                f.write_str("{")?;
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i != 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{key}: {value}")?;
                }
                f.write_str("}")
            }
            Value::Global(global) => write!(f, "{}.{}", global.module, global.name),
            Value::Object(object) => {
                write!(f, "{}.{}(", object.class.module, object.class.name)?;
                seq(f, &object.args)?;
                f.write_str(")")?;
                if let Some(state) = &object.state {
                    write!(f, " with {state}")?;
                }
                Ok(())
            }
        }
    }
}
