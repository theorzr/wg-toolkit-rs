//! Deserialization module for Packed XML.

use std::io::{self, Read, Seek, Cursor};

use smallvec::SmallVec;
use thiserror::Error;
use base64::Engine;

use crate::util::io::WgReadExt;

use super::{DataType, Element, Value, Vector, MAGIC};


/// Read a packed XML data from an readable and seek-able object.
/// 
/// *The content will be read starting from the initial position of the reader.*
pub fn from_reader<R: Read + Seek>(mut reader: R) -> Result<Box<Element>, DeError> {

    // Validate file's magic
    if !reader.check_exact(MAGIC)? {
        return Err(DeError::InvalidMagic);
    }

    // Unknown byte
    reader.skip::<1>()?;

    // Parsing
    let dict = read_dictionary(&mut reader)?;
    let mut element = Box::new(Element::new());
    read_element(&mut reader, &mut *element, &dict[..])?;
    Ok(element)

}


/// Read a packed XML from raw bytes.
#[inline]
pub fn from_bytes<B: AsRef<[u8]>>(data: B) -> Result<Box<Element>, DeError> {
    let data = data.as_ref();
    from_reader(Cursor::new(data))
}


/// Internal function to read dictionary.
fn read_dictionary<R: Read + Seek>(reader: &mut R) -> Result<Vec<String>, DeError> {
    let mut dict = Vec::new();
    loop {
        let string = reader.read_cstring_variable()?;
        if string.is_empty() {
            return Ok(dict)
        }
        dict.push(string);
    }
}


/// Internal function to read a data descriptor.
fn read_data_descriptor<R: Read>(reader: &mut R) -> Result<DataDescriptor, DeError> {
    let data_descriptor = reader.read_u32()?;
    let raw_data_type = data_descriptor >> 28;
    Ok(DataDescriptor {
        ty: DataType::from_raw(raw_data_type)
            .ok_or(DeError::InvalidDataType(raw_data_type))?,
        end_offset: data_descriptor & 0x00FFFFFFF,
    })
}


/// Internal function to read a child descriptor (data + name).
fn read_child_descriptor<R: Read>(reader: &mut R) -> Result<ChildDescriptor, DeError> {
    Ok(ChildDescriptor {
        name_index: reader.read_u16()? as usize,
        data: read_data_descriptor(&mut *reader)?,
    })
}


/// Internal function that reads the current's element descriptor
/// and its children.
fn read_element<R: Read>(reader: &mut R, element: &mut Element, dict: &[String]) -> Result<(), DeError> {
    
    let children_count = reader.read_u16()? as usize;
    let self_descriptor = read_data_descriptor(&mut *reader)?;
    let mut children_descriptors = SmallVec::<[ChildDescriptor; 16]>::new();
    
    for _ in 0..children_count {
        children_descriptors.push(read_child_descriptor(&mut *reader)?);
    }

    read_data(&mut *reader, &mut element.value, &self_descriptor, dict, 0)?;
    let mut offset = self_descriptor.end_offset;

    for child in children_descriptors {
        let mut value = Value::Boolean(false);
        read_data(&mut *reader, &mut value, &child.data, dict, offset)?;
        offset = child.data.end_offset;
        element.add_children(&dict[child.name_index], value);
    }

    Ok(())

}


/// Internal function to read a value.
fn read_data<R: Read>(reader: &mut R, value: &mut Value, desc: &DataDescriptor, dict: &[String], offset: u32) -> Result<(), DeError> {
    let len = (desc.end_offset - offset) as usize;
    *value = match desc.ty {
        DataType::Element => {
            let mut element = Box::new(Element::new());
            read_element(reader, &mut *element, dict)?;
            Value::Element(element)
        },
        DataType::String => Value::String(read_string(reader, len)?),
        DataType::Integer => Value::Integer(read_integer(reader, len)?),
        DataType::Boolean => Value::Boolean(read_bool(reader, len)?),
        DataType::CompressedString => Value::String(read_compressed_string(reader, len)?),
        DataType::Vector => Value::Vector(Vector::from_smallvec(read_vector(reader, len)?)),
    };
    Ok(())
}


/// Internal function to read a string of specific length.
fn read_string<R: Read>(reader: &mut R, len: usize) -> Result<String, DeError> {
    if len == 0 {
        Ok("".to_string())
    } else {
        reader.read_string(len as usize).map_err(Into::into)
    }
}


/// Internal function that reads a compressed string.
fn read_compressed_string<R: Read>(reader: &mut R, len: usize) -> Result<String, DeError> {
    let data = reader.read_blob(len)?;
    Ok(base64::prelude::BASE64_STANDARD.encode(&data[..]))
}


/// Internal function to read a data from its descriptor and a reader.
fn read_integer<R: Read>(reader: &mut R, len: usize) -> Result<i64, DeError> {
    match len {
        0 => Ok(0),
        1 => Ok(reader.read_i8()? as i64),
        2 => Ok(reader.read_i16()? as i64),
        4 => Ok(reader.read_i32()? as i64),
        8 => Ok(reader.read_i64()?),
        _ => Err(DeError::InvalidIntegerLen(len))
    }
}


/// Internal function to read a boolean data.
fn read_bool<R: Read>(reader: &mut R, len: usize) -> Result<bool, DeError> {
    match len {
        0 => Ok(false),
        1 => Ok(reader.read_u8()? == 1),
        _ => Err(DeError::InvalidBoolLen(len))
    }
}


/// Internal function to read a 
fn read_vector<R: Read>(reader: &mut R, len: usize) -> Result<SmallVec<[f32; 3]>, DeError> {
    
    if len % 4 != 0 {
        return Err(DeError::InvalidVectorLen(len))
    }

    let n = len / 4;
    let mut res = SmallVec::new();
    for _ in 0..n {
        res.push(reader.read_f32()?);
    }

    Ok(res)

}


/// Internal data descriptor.
struct DataDescriptor {
    /// Type of data.
    ty: DataType,
    /// Offset of the end of the data, this can be used to
    /// compute data length if start address is known.
    end_offset: u32,
}


/// Internal descriptor for children elements of an element.
struct ChildDescriptor {
    /// Data descriptor for this child.
    data: DataDescriptor,
    /// Name index in the dictionary.
    name_index: usize
}


/// Deserialization error that can happen while deserializing
#[derive(Debug, Error)]
pub enum DeError {
    /// Invalid magic signature for the file.
    #[error("invalid magic")]
    InvalidMagic,
    /// Invalid data type while parsing.
    #[error("invalid data type id {0}")]
    InvalidDataType(u32),
    /// Invalid data size for a number.
    #[error("invalid data length of {0} bytes for a number")]
    InvalidIntegerLen(usize),
    /// Invalid data size for a boolean.
    #[error("invalid data length of {0} bytes for a boolean")]
    InvalidBoolLen(usize),
    /// Invalid vector length, not a multiple a 4 bytes (f32).
    #[error("invalid data length of {0} bytes for a vector")]
    InvalidVectorLen(usize),
    /// IO error while unpacking.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}
