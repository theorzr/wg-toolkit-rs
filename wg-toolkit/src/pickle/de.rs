//! Pickle decoding. Containers are built in an arena first, because the memo shares
//! them, possibly before they are filled: values are only made once the pickle ends.

use std::collections::HashMap;
use std::io::{self, Read};

use thiserror::Error;

use super::{op, Global, Object, Value};


/// Deserialization error that can happen while deserializing.
#[derive(Debug, Error)]
pub enum DeError {
    /// An opcode of protocols above 2, or one we don't support (persistent ids,
    /// extensions, old-style instances).
    #[error("unsupported opcode 0x{0:02X}")]
    UnsupportedOpcode(u8),
    #[error("unsupported protocol {0}")]
    UnsupportedProtocol(u8),
    /// An opcode needing more values or a mark than the stack has.
    #[error("stack underflow")]
    StackUnderflow,
    #[error("missing memo {0}")]
    MissingMemo(u32),
    /// A text literal, of the text protocol, that doesn't parse.
    #[error("invalid literal")]
    InvalidLiteral,
    #[error("integer too large")]
    IntegerTooLarge,
    /// An opcode applied to a value of the wrong kind, appending to a dict say.
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("trailing data")]
    TrailingData,
    /// IO error while unpickling.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// Decode a pickle from a reader, reading up to its end, no further.
pub fn from_reader<R: Read>(read: R) -> Result<Value, DeError> {
    Decoder { read, nodes: Vec::new(), stack: Vec::new(), marks: Vec::new(), memo: HashMap::new() }.decode()
}

/// Decode a pickle that is the whole data.
pub fn from_bytes<B: AsRef<[u8]>>(data: B) -> Result<Value, DeError> {
    let data = data.as_ref();
    let (value, len) = from_bytes_prefix(data)?;
    if len != data.len() {
        return Err(DeError::TrailingData);
    }
    Ok(value)
}

/// Decode a pickle at the start of the data, returning it with its length.
pub fn from_bytes_prefix(data: &[u8]) -> Result<(Value, usize), DeError> {
    let mut read = data;
    let value = from_reader(&mut read)?;
    Ok((value, data.len() - read.len()))
}

type NodeId = usize;

/// A value in the arena, its contents being other nodes.
#[derive(Debug)]
enum Node {
    Value(Value),
    List(Vec<NodeId>),
    Tuple(Vec<NodeId>),
    Dict(Vec<(NodeId, NodeId)>),
    Set(NodeId, bool),
    Object { class: Global, args: NodeId, new: bool, state: Option<NodeId> },
}

struct Decoder<R> {
    read: R,
    nodes: Vec<Node>,
    stack: Vec<NodeId>,
    marks: Vec<usize>,
    memo: HashMap<u32, NodeId>,
}

impl<R: Read> Decoder<R> {

    fn decode(mut self) -> Result<Value, DeError> {
        loop {
            let code = self.u8()?;
            match code {
                op::STOP => break,
                op::PROTO => {
                    let proto = self.u8()?;
                    if proto > 2 {
                        return Err(DeError::UnsupportedProtocol(proto));
                    }
                }
                op::MARK => self.marks.push(self.stack.len()),
                op::POP => { self.pop()?; }
                op::POP_MARK => { self.pop_mark()?; }
                op::DUP => {
                    let top = *self.stack.last().ok_or(DeError::StackUnderflow)?;
                    self.stack.push(top);
                }
                op::NONE => self.push(Value::None),
                op::NEWTRUE => self.push(Value::Bool(true)),
                op::NEWFALSE => self.push(Value::Bool(false)),
                op::INT => {
                    let line = self.line()?;
                    let value = match &line[..] {
                        b"00" => Value::Bool(false),
                        b"01" => Value::Bool(true),
                        _ => Value::I64(parse(&line)?),
                    };
                    self.push(value);
                }
                op::BININT => {
                    let value = i32::from_le_bytes(self.array()?);
                    self.push(Value::I64(value.into()));
                }
                op::BININT1 => {
                    let value = self.u8()?;
                    self.push(Value::I64(value.into()));
                }
                op::BININT2 => {
                    let value = u16::from_le_bytes(self.array()?);
                    self.push(Value::I64(value.into()));
                }
                op::LONG => {
                    let line = self.line()?;
                    let value = parse(line.strip_suffix(b"L").unwrap_or(&line))?;
                    self.push(Value::I64(value));
                }
                op::LONG1 => {
                    let len = self.u8()?;
                    let value = self.long(len.into())?;
                    self.push(value);
                }
                op::LONG4 => {
                    let len = u32::from_le_bytes(self.array()?);
                    let value = self.long(len as usize)?;
                    self.push(value);
                }
                op::FLOAT => {
                    let line = self.line()?;
                    self.push(Value::F64(parse(&line)?));
                }
                op::BINFLOAT => {
                    let value = f64::from_be_bytes(self.array()?);
                    self.push(Value::F64(value));
                }
                op::STRING => {
                    let line = self.line()?;
                    let value = unescape_string(&line)?;
                    self.push(Value::Bytes(value));
                }
                op::BINSTRING => {
                    let len = i32::from_le_bytes(self.array()?);
                    let len = usize::try_from(len).map_err(|_| DeError::Invalid("length"))?;
                    let value = self.bytes(len)?;
                    self.push(Value::Bytes(value));
                }
                op::SHORT_BINSTRING => {
                    let len = self.u8()?;
                    let value = self.bytes(len.into())?;
                    self.push(Value::Bytes(value));
                }
                op::UNICODE => {
                    let line = self.line()?;
                    let value = unescape_unicode(&line)?;
                    self.push(Value::String(value));
                }
                op::BINUNICODE => {
                    let len = u32::from_le_bytes(self.array()?);
                    let value = self.bytes(len as usize)?;
                    self.push(Value::String(String::from_utf8_lossy(&value).into_owned()));
                }
                op::EMPTY_LIST => self.push_node(Node::List(Vec::new())),
                op::EMPTY_TUPLE => self.push_node(Node::Tuple(Vec::new())),
                op::EMPTY_DICT => self.push_node(Node::Dict(Vec::new())),
                op::LIST => {
                    let items = self.pop_mark()?;
                    self.push_node(Node::List(items));
                }
                op::TUPLE => {
                    let items = self.pop_mark()?;
                    self.push_node(Node::Tuple(items));
                }
                op::TUPLE1 | op::TUPLE2 | op::TUPLE3 => {
                    let len = usize::from(code - op::TUPLE1 + 1);
                    let start = self.stack.len().checked_sub(len).ok_or(DeError::StackUnderflow)?;
                    let items = self.stack.split_off(start);
                    self.push_node(Node::Tuple(items));
                }
                op::DICT => {
                    let items = self.pop_mark()?;
                    let entries = pairs(items)?;
                    self.push_node(Node::Dict(entries));
                }
                op::APPEND => {
                    let item = self.pop()?;
                    self.list(self.top()?)?.push(item);
                }
                op::APPENDS => {
                    let items = self.pop_mark()?;
                    self.list(self.top()?)?.extend(items);
                }
                op::SETITEM => {
                    let value = self.pop()?;
                    let key = self.pop()?;
                    self.dict(self.top()?)?.push((key, value));
                }
                op::SETITEMS => {
                    let items = self.pop_mark()?;
                    let entries = pairs(items)?;
                    self.dict(self.top()?)?.extend(entries);
                }
                op::GET => {
                    let index = parse(&self.line()?)?;
                    self.get(index)?;
                }
                op::BINGET => {
                    let index = self.u8()?;
                    self.get(index.into())?;
                }
                op::LONG_BINGET => {
                    let index = u32::from_le_bytes(self.array()?);
                    self.get(index)?;
                }
                op::PUT => {
                    let index = parse(&self.line()?)?;
                    self.memo.insert(index, self.top()?);
                }
                op::BINPUT => {
                    let index = self.u8()?;
                    self.memo.insert(index.into(), self.top()?);
                }
                op::LONG_BINPUT => {
                    let index = u32::from_le_bytes(self.array()?);
                    self.memo.insert(index, self.top()?);
                }
                op::GLOBAL => {
                    let module = String::from_utf8_lossy(&self.line()?).into_owned();
                    let name = String::from_utf8_lossy(&self.line()?).into_owned();
                    self.push(Value::Global(Global { module, name }));
                }
                op::REDUCE | op::NEWOBJ => {
                    let args = self.pop()?;
                    let class = match self.pop().map(|id| &self.nodes[id])? {
                        Node::Value(Value::Global(global)) => global.clone(),
                        _ => return Err(DeError::Invalid("class")),
                    };
                    let Node::Tuple(items) = &self.nodes[args] else { return Err(DeError::Invalid("arguments")) };
                    // Python 2 pickles sets as `set([items])`.
                    let set = code == op::REDUCE && class.module == "__builtin__" && items.len() <= 1;
                    let node = match &class.name[..] {
                        "set" | "frozenset" if set => Node::Set(items.first().copied().unwrap_or(args), class.name == "frozenset"),
                        _ => Node::Object { class, args, new: code == op::NEWOBJ, state: None },
                    };
                    self.push_node(node);
                }
                op::BUILD => {
                    let state = self.pop()?;
                    let top = self.top()?;
                    let Node::Object { state: object_state, .. } = &mut self.nodes[top] else {
                        return Err(DeError::Invalid("object"));
                    };
                    *object_state = Some(state);
                }
                _ => return Err(DeError::UnsupportedOpcode(code)),
            }
        }
        let root = self.pop()?;
        let mut builder = Builder { refs: vec![0; self.nodes.len()], built: Vec::new(), nodes: self.nodes };
        builder.count(root);
        builder.built.resize_with(builder.nodes.len(), || None);
        Ok(builder.build(root))
    }

    fn push(&mut self, value: Value) {
        self.push_node(Node::Value(value));
    }

    fn push_node(&mut self, node: Node) {
        self.stack.push(self.nodes.len());
        self.nodes.push(node);
    }

    fn pop(&mut self) -> Result<NodeId, DeError> {
        self.stack.pop().ok_or(DeError::StackUnderflow)
    }

    fn top(&self) -> Result<NodeId, DeError> {
        self.stack.last().copied().ok_or(DeError::StackUnderflow)
    }

    fn pop_mark(&mut self) -> Result<Vec<NodeId>, DeError> {
        let mark = self.marks.pop().ok_or(DeError::StackUnderflow)?;
        if mark > self.stack.len() {
            return Err(DeError::StackUnderflow);
        }
        Ok(self.stack.split_off(mark))
    }

    fn get(&mut self, index: u32) -> Result<(), DeError> {
        let id = *self.memo.get(&index).ok_or(DeError::MissingMemo(index))?;
        self.stack.push(id);
        Ok(())
    }

    fn list(&mut self, id: NodeId) -> Result<&mut Vec<NodeId>, DeError> {
        match &mut self.nodes[id] {
            Node::List(items) => Ok(items),
            _ => Err(DeError::Invalid("list")),
        }
    }

    fn dict(&mut self, id: NodeId) -> Result<&mut Vec<(NodeId, NodeId)>, DeError> {
        match &mut self.nodes[id] {
            Node::Dict(entries) => Ok(entries),
            _ => Err(DeError::Invalid("dict")),
        }
    }

    fn u8(&mut self) -> Result<u8, DeError> {
        Ok(self.array::<1>()?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DeError> {
        let mut buf = [0; N];
        self.read.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn bytes(&mut self, len: usize) -> Result<Vec<u8>, DeError> {
        let mut buf = Vec::new();
        (&mut self.read).take(len as u64).read_to_end(&mut buf)?;
        if buf.len() != len {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        Ok(buf)
    }

    fn line(&mut self) -> Result<Vec<u8>, DeError> {
        let mut line = Vec::new();
        loop {
            match self.u8()? {
                b'\n' => break,
                byte => line.push(byte),
            }
        }
        // Text protocol pickles written on Windows.
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Ok(line)
    }

    /// A little-endian two's complement integer.
    fn long(&mut self, len: usize) -> Result<Value, DeError> {
        let bytes = self.bytes(len)?;
        let negative = bytes.last().is_some_and(|&b| b & 0x80 != 0);
        let fill = if negative { 0xFF } else { 0 };
        if len > 8 && bytes[8..].iter().any(|&b| b != fill) {
            return Err(DeError::IntegerTooLarge);
        }
        let mut buf = [fill; 8];
        let n = len.min(8);
        buf[..n].copy_from_slice(&bytes[..n]);
        let value = i64::from_le_bytes(buf);
        if len > 8 && (value < 0) != negative {
            return Err(DeError::IntegerTooLarge);
        }
        Ok(Value::I64(value))
    }

}

/// Make the values once the arena is complete. A node referenced once is moved into
/// its value, a shared one is built once and cloned.
struct Builder {
    nodes: Vec<Node>,
    refs: Vec<u32>,
    built: Vec<Option<Value>>,
}

impl Builder {

    fn count(&mut self, id: NodeId) {
        self.refs[id] += 1;
        if self.refs[id] > 1 {
            return;
        }
        let children: Vec<NodeId> = match &self.nodes[id] {
            Node::Value(_) => return,
            Node::List(items) | Node::Tuple(items) => items.clone(),
            Node::Dict(entries) => entries.iter().flat_map(|&(k, v)| [k, v]).collect(),
            &Node::Set(items, _) => vec![items],
            &Node::Object { args, state, .. } => [Some(args), state].into_iter().flatten().collect(),
        };
        for child in children {
            self.count(child);
        }
    }

    fn build(&mut self, id: NodeId) -> Value {
        if let Some(value) = &self.built[id] {
            return value.clone();
        }
        // Taken out while building: a structure containing itself gets `None` there.
        let value = match std::mem::replace(&mut self.nodes[id], Node::Value(Value::None)) {
            Node::Value(value) => value,
            Node::List(items) => Value::List(items.into_iter().map(|id| self.build(id)).collect()),
            Node::Tuple(items) => Value::Tuple(items.into_iter().map(|id| self.build(id)).collect()),
            Node::Dict(entries) => Value::Dict(entries.into_iter().map(|(k, v)| (self.build(k), self.build(v))).collect()),
            Node::Set(items, frozen) => {
                let items = match self.build(items) {
                    Value::List(items) | Value::Tuple(items) => items.into_iter().collect(),
                    _ => Default::default(),
                };
                if frozen { Value::FrozenSet(items) } else { Value::Set(items) }
            }
            Node::Object { class, args, new, state } => {
                let Value::Tuple(args) = self.build(args) else { return Value::None };
                let state = state.map(|id| self.build(id));
                Value::Object(Box::new(Object { class, args, new, state }))
            }
        };
        if self.refs[id] > 1 {
            self.built[id] = Some(value.clone());
        }
        value
    }

}

fn pairs(items: Vec<NodeId>) -> Result<Vec<(NodeId, NodeId)>, DeError> {
    if items.len() % 2 != 0 {
        return Err(DeError::Invalid("dict items"));
    }
    Ok(items.chunks_exact(2).map(|pair| (pair[0], pair[1])).collect())
}

fn parse<T: std::str::FromStr>(line: &[u8]) -> Result<T, DeError> {
    std::str::from_utf8(line).ok().and_then(|s| s.trim().parse().ok()).ok_or(DeError::InvalidLiteral)
}

/// A `str` literal as `repr` writes it, quoted and escaped.
fn unescape_string(line: &[u8]) -> Result<Vec<u8>, DeError> {
    let inner = match line {
        [q, inner @ .., e] if q == e && (*q == b'\'' || *q == b'"') => inner,
        _ => return Err(DeError::InvalidLiteral),
    };
    let mut out = Vec::with_capacity(inner.len());
    let mut iter = inner.iter().copied().peekable();
    while let Some(byte) = iter.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let escaped = iter.next().ok_or(DeError::InvalidLiteral)?;
        out.push(match escaped {
            b'n' => b'\n',
            b't' => b'\t',
            b'r' => b'\r',
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0C,
            b'v' => 0x0B,
            b'x' => {
                let hex = [iter.next().ok_or(DeError::InvalidLiteral)?, iter.next().ok_or(DeError::InvalidLiteral)?];
                u8::from_str_radix(std::str::from_utf8(&hex).map_err(|_| DeError::InvalidLiteral)?, 16)
                    .map_err(|_| DeError::InvalidLiteral)?
            }
            b'0'..=b'7' => {
                let mut value = u32::from(escaped - b'0');
                for _ in 0..2 {
                    match iter.peek() {
                        Some(&d @ b'0'..=b'7') => { value = value * 8 + u32::from(d - b'0'); iter.next(); }
                        _ => break,
                    }
                }
                value as u8
            }
            other => other,
        });
    }
    Ok(out)
}

/// A `unicode` literal as `raw-unicode-escape` writes it: `\uXXXX` and `\UXXXXXXXX`
/// escapes, other bytes being Latin-1 characters.
fn unescape_unicode(line: &[u8]) -> Result<String, DeError> {
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < line.len() {
        let len = match &line[i..] {
            [b'\\', b'u', ..] => 4,
            [b'\\', b'U', ..] => 8,
            _ => {
                out.push(char::from(line[i]));
                i += 1;
                continue;
            }
        };
        let hex = line.get(i + 2..i + 2 + len).ok_or(DeError::InvalidLiteral)?;
        let code = u32::from_str_radix(std::str::from_utf8(hex).map_err(|_| DeError::InvalidLiteral)?, 16)
            .map_err(|_| DeError::InvalidLiteral)?;
        out.push(char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER));
        i += 2 + len;
    }
    Ok(out)
}
