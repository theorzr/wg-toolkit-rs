//! This module provides a way of understanding the script definitions and the dynamic
//! types used by them.

mod value;
mod ty;

mod def;

mod parse;
mod load;

pub use def::{Script, Component, Entity, Interface, Method, Arg, Property, PropertyFlags, VariableHeaderSize};
pub use ty::{TySystem, Ty, TyKind, TyDict, TyDictProp, TySeq};
pub use value::{Value, StringValue, PythonValue};
pub use load::load;
