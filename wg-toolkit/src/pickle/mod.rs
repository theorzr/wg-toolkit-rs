//! Python pickle codec, for the protocols 0 to 2 that Python 2, so the game, uses.
//!
//! Python 2 `str` are [`Value::Bytes`] and `unicode` are [`Value::String`]. Objects,
//! which the game pickles with their class and state, are [`Value::Object`].

mod de;
mod ser;
mod value;

pub use de::{from_reader, from_bytes, from_bytes_prefix, DeError};
pub use value::{Value, Global, Object};
pub use ser::{to_writer, to_bytes};


/// The opcodes of protocols 0 to 2.
mod op {
    pub const MARK: u8 = b'(';
    pub const STOP: u8 = b'.';
    pub const POP: u8 = b'0';
    pub const POP_MARK: u8 = b'1';
    pub const DUP: u8 = b'2';
    pub const FLOAT: u8 = b'F';
    pub const INT: u8 = b'I';
    pub const BININT: u8 = b'J';
    pub const BININT1: u8 = b'K';
    pub const LONG: u8 = b'L';
    pub const BININT2: u8 = b'M';
    pub const NONE: u8 = b'N';
    pub const REDUCE: u8 = b'R';
    pub const STRING: u8 = b'S';
    pub const BINSTRING: u8 = b'T';
    pub const SHORT_BINSTRING: u8 = b'U';
    pub const UNICODE: u8 = b'V';
    pub const BINUNICODE: u8 = b'X';
    pub const APPEND: u8 = b'a';
    pub const BUILD: u8 = b'b';
    pub const GLOBAL: u8 = b'c';
    pub const DICT: u8 = b'd';
    pub const EMPTY_DICT: u8 = b'}';
    pub const APPENDS: u8 = b'e';
    pub const GET: u8 = b'g';
    pub const BINGET: u8 = b'h';
    pub const LONG_BINGET: u8 = b'j';
    pub const LIST: u8 = b'l';
    pub const EMPTY_LIST: u8 = b']';
    pub const PUT: u8 = b'p';
    pub const BINPUT: u8 = b'q';
    pub const LONG_BINPUT: u8 = b'r';
    pub const SETITEM: u8 = b's';
    pub const TUPLE: u8 = b't';
    pub const EMPTY_TUPLE: u8 = b')';
    pub const SETITEMS: u8 = b'u';
    pub const BINFLOAT: u8 = b'G';
    pub const PROTO: u8 = 0x80;
    pub const NEWOBJ: u8 = 0x81;
    pub const TUPLE1: u8 = 0x85;
    pub const TUPLE2: u8 = 0x86;
    pub const TUPLE3: u8 = 0x87;
    pub const NEWTRUE: u8 = 0x88;
    pub const NEWFALSE: u8 = 0x89;
    pub const LONG1: u8 = 0x8A;
    pub const LONG4: u8 = 0x8B;
}


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn round_trip() {
        let value = Value::dict([
            ("none", Value::None),
            ("bool", Value::Bool(true)),
            ("ints", Value::List(vec![Value::I64(0), Value::I64(300), Value::I64(-5), Value::I64(1 << 40)])),
            ("float", Value::F64(0.5)),
            ("unicode", Value::String("é".to_string())),
            ("tuple", Value::Tuple(vec![Value::I64(1), Value::from("a")])),
            ("set", Value::Set([Value::I64(1), Value::I64(2)].into_iter().collect())),
            ("object", Value::object(Global::new("items", "ItemsPrices"), Value::dict([("_itemsPriceInfo", Value::dict([]))]))),
        ]);
        assert_eq!(from_bytes(to_bytes(&value)).unwrap(), value);
    }

    #[test]
    fn python2() {
        // pickle.dumps(({'a': [1, 2L, u'b']}, set([3]), 1.5), 2) with a shared list memoized.
        let data = b"\x80\x02}q\x00U\x01aq\x01]q\x02(K\x01\x8a\x01\x02X\x01\x00\x00\x00bq\x03es\
            c__builtin__\nset\nq\x04]q\x05K\x03a\x85q\x06Rq\x07G?\xf8\x00\x00\x00\x00\x00\x00\x87q\x08h\x02\x86.";
        let list = Value::List(vec![Value::I64(1), Value::I64(2), Value::String("b".to_string())]);
        let expected = Value::Tuple(vec![
            Value::Tuple(vec![
                Value::dict([("a", list.clone())]),
                Value::Set([Value::I64(3)].into_iter().collect()),
                Value::F64(1.5),
            ]),
            list,
        ]);
        assert_eq!(from_bytes(data).unwrap(), expected);
    }

}
