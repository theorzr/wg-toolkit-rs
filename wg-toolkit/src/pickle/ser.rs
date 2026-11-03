//! Pickle encoding, with protocol 2 as Python 2 writes it, without the memo.

use std::io::{self, Write};

use super::{op, Global, Value};


/// How many items Python appends or sets at once.
const BATCH: usize = 1000;

/// Write a value with protocol 2.
pub fn to_writer<W: Write>(mut write: W, value: &Value) -> io::Result<()> {
    write.write_all(&[op::PROTO, 2])?;
    encode(value, &mut write)?;
    write.write_all(&[op::STOP])
}

/// Write a value with protocol 2 to bytes.
pub fn to_bytes(value: &Value) -> Vec<u8> {
    let mut data = Vec::new();
    to_writer(&mut data, value).unwrap();
    data
}

fn encode(value: &Value, write: &mut impl Write) -> io::Result<()> {
    match value {
        Value::None => write.write_all(&[op::NONE]),
        Value::Bool(true) => write.write_all(&[op::NEWTRUE]),
        Value::Bool(false) => write.write_all(&[op::NEWFALSE]),
        &Value::I64(value) => {
            if let Ok(value) = u8::try_from(value) {
                write.write_all(&[op::BININT1, value])
            } else if let Ok(value) = u16::try_from(value) {
                write.write_all(&[op::BININT2])?;
                write.write_all(&value.to_le_bytes())
            } else if let Ok(value) = i32::try_from(value) {
                write.write_all(&[op::BININT])?;
                write.write_all(&value.to_le_bytes())
            } else {
                write!(write, "{}{value}\n", op::INT as char)
            }
        }
        Value::F64(value) => {
            write.write_all(&[op::BINFLOAT])?;
            write.write_all(&value.to_be_bytes())
        }
        Value::Bytes(value) => {
            if let Ok(len) = u8::try_from(value.len()) {
                write.write_all(&[op::SHORT_BINSTRING, len])?;
            } else {
                write.write_all(&[op::BINSTRING])?;
                write.write_all(&(value.len() as u32).to_le_bytes())?;
            }
            write.write_all(value)
        }
        Value::String(value) => {
            write.write_all(&[op::BINUNICODE])?;
            write.write_all(&(value.len() as u32).to_le_bytes())?;
            write.write_all(value.as_bytes())
        }
        Value::List(items) => {
            write.write_all(&[op::EMPTY_LIST])?;
            encode_appends(items.iter(), items.len(), write)
        }
        Value::Tuple(items) => encode_tuple(items, write),
        Value::Dict(entries) => {
            write.write_all(&[op::EMPTY_DICT])?;
            let entries: Vec<_> = entries.iter().collect();
            for batch in entries.chunks(BATCH) {
                if let [(key, value)] = batch {
                    encode(key, write)?;
                    encode(value, write)?;
                    write.write_all(&[op::SETITEM])?;
                } else {
                    write.write_all(&[op::MARK])?;
                    for (key, value) in batch {
                        encode(key, write)?;
                        encode(value, write)?;
                    }
                    write.write_all(&[op::SETITEMS])?;
                }
            }
            Ok(())
        }
        // Python 2 has no set opcodes and pickles `set([items])`.
        Value::Set(items) | Value::FrozenSet(items) => {
            let name = if matches!(value, Value::Set(_)) { "set" } else { "frozenset" };
            encode_global(&Global::new("__builtin__", name), write)?;
            write.write_all(&[op::EMPTY_LIST])?;
            encode_appends(items.iter(), items.len(), write)?;
            write.write_all(&[op::TUPLE1, op::REDUCE])
        }
        Value::Global(global) => encode_global(global, write),
        Value::Object(object) => {
            encode_global(&object.class, write)?;
            encode_tuple(&object.args, write)?;
            write.write_all(&[if object.new { op::NEWOBJ } else { op::REDUCE }])?;
            if let Some(state) = &object.state {
                encode(state, write)?;
                write.write_all(&[op::BUILD])?;
            }
            Ok(())
        }
    }
}

fn encode_appends<'a>(items: impl Iterator<Item = &'a Value>, len: usize, write: &mut impl Write) -> io::Result<()> {
    if len == 1 {
        for item in items {
            encode(item, write)?;
        }
        return write.write_all(&[op::APPEND]);
    }
    for (i, item) in items.enumerate() {
        if i % BATCH == 0 {
            if i != 0 {
                write.write_all(&[op::APPENDS])?;
            }
            write.write_all(&[op::MARK])?;
        }
        encode(item, write)?;
    }
    if len != 0 {
        write.write_all(&[op::APPENDS])?;
    }
    Ok(())
}

fn encode_tuple(items: &[Value], write: &mut impl Write) -> io::Result<()> {
    if items.is_empty() {
        return write.write_all(&[op::EMPTY_TUPLE]);
    }
    if items.len() > 3 {
        write.write_all(&[op::MARK])?;
    }
    for item in items {
        encode(item, write)?;
    }
    write.write_all(&[match items.len() {
        1 => op::TUPLE1,
        2 => op::TUPLE2,
        3 => op::TUPLE3,
        _ => op::TUPLE,
    }])
}

fn encode_global(global: &Global, write: &mut impl Write) -> io::Result<()> {
    write.write_all(&[op::GLOBAL])?;
    write!(write, "{}\n{}\n", global.module, global.name)
}
