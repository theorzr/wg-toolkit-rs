//! This module provides extension traits for [`Read`] and [`Write`] for
//! supporting common formats used within the BigWorld engine.

use std::io::{self, Read, Write, Cursor};

use byteorder::{ReadBytesExt, WriteBytesExt, LE};
use glam::{Vec2, Vec3, Vec4};


/// An extension to the [`Read`] trait specifically used to decode WG formats
/// and used for network protocol.
pub trait WgReadExt: Read {

    #[inline]
    fn read_uint(&mut self, nbytes: usize) -> io::Result<u64> {
        ReadBytesExt::read_uint::<LE>(self, nbytes)
    }

    #[inline]
    fn read_int(&mut self, nbytes: usize) -> io::Result<i64> {
        ReadBytesExt::read_int::<LE>(self, nbytes)
    }

    /// Reads an unsigned 8 bit integer from the underlying reader.
    #[inline]
    fn read_u8(&mut self) -> io::Result<u8> {
        ReadBytesExt::read_u8(self)
    }

    /// Reads a signed 8 bit integer from the underlying reader.
    #[inline]
    fn read_i8(&mut self) -> io::Result<i8> {
        ReadBytesExt::read_i8(self)
    }

    /// Skip the given number of u8 integers.
    #[inline]
    fn skip<const N: usize>(&mut self) -> io::Result<()> {
        let mut buf = [0; N];
        self.read_exact(&mut buf)?;
        Ok(())
    }

    /// Reads an unsigned 16 bit integer from the underlying reader.
    #[inline]
    fn read_u16(&mut self) -> io::Result<u16> {
        ReadBytesExt::read_u16::<LE>(self)
    }

    /// Reads a signed 16 bit integer from the underlying reader.
    #[inline]
    fn read_i16(&mut self) -> io::Result<i16> {
        ReadBytesExt::read_i16::<LE>(self)
    }

    /// Reads an unsigned 24 bit integer from the underlying reader.
    #[inline]
    fn read_u24(&mut self) -> io::Result<u32> {
        ReadBytesExt::read_u24::<LE>(self)
    }

    /// Reads a signed 24 bit integer from the underlying reader.
    #[inline]
    fn read_i24(&mut self) -> io::Result<i32> {
        ReadBytesExt::read_i24::<LE>(self)
    }

    /// Reads an unsigned 32 bit integer from the underlying reader.
    #[inline]
    fn read_u32(&mut self) -> io::Result<u32> {
        ReadBytesExt::read_u32::<LE>(self)
    }

    /// Reads a signed 32 bit integer from the underlying reader.
    #[inline]
    fn read_i32(&mut self) -> io::Result<i32> {
        ReadBytesExt::read_i32::<LE>(self)
    }

    /// Read a packed unsigned 24 bit integer from the underlying reader.
    #[inline]
    fn read_packed_u24(&mut self) -> io::Result<u32> {
        match self.read_u8()? {
            255 => self.read_u24(),
            n => Ok(n as u32)
        }
    }

    /// Reads an unsigned 64 bit integer from the underlying reader.
    #[inline]
    fn read_u64(&mut self) -> io::Result<u64> {
        ReadBytesExt::read_u64::<LE>(self)
    }

    /// Reads a signed 64 bit integer from the underlying reader.
    #[inline]
    fn read_i64(&mut self) -> io::Result<i64> {
        ReadBytesExt::read_i64::<LE>(self)
    }

    /// Reads a IEEE754 single-precision (4 bytes) floating point number 
    /// from the underlying reader.
    #[inline]
    fn read_f32(&mut self) -> io::Result<f32> {
        ReadBytesExt::read_f32::<LE>(self)
    }

    /// Reads a IEEE754 double-precision (8 bytes) floating point number 
    /// from the underlying reader.
    #[inline]
    fn read_f64(&mut self) -> io::Result<f64> {
        ReadBytesExt::read_f64::<LE>(self)
    }

    /// Read a single boolean from the underlying reader.
    #[inline]
    fn read_bool(&mut self) -> io::Result<bool> {
        Ok(self.read_u8()? != 0)
    }

    /// Check that the next `N` bytes are the exact same as the given array.
    #[inline]
    fn check_exact<const N: usize>(&mut self, bytes: &[u8; N]) -> io::Result<bool> {
        let mut buf = [0; N];
        self.read_exact(&mut buf[..])?;
        Ok(&buf == bytes)
    }

    #[inline]
    fn read_blob_to_end(&mut self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Read a blob of the given length.
    fn read_blob(&mut self, len: usize) -> io::Result<Vec<u8>> {
        // TODO: Maybe use a better uninit approach in the future.
        let mut buf = vec![0; len];
        self.read_exact(&mut buf[..])?;
        Ok(buf)
    }

    /// Read a blob of a length that is specified with a packed u32 before the 
    /// actual vector.
    fn read_blob_variable(&mut self) -> io::Result<Vec<u8>> {
        let len = self.read_packed_u24()? as usize;
        let mut buf = vec![0; len];
        self.read_exact(&mut buf[..])?;
        Ok(buf)
    }

    /// Read an UTF-8 string of the given length.
    fn read_string(&mut self, len: usize) -> io::Result<String> {
        String::from_utf8(self.read_blob(len)?)
            .map_err(|_| io::ErrorKind::InvalidData.into())
    }

    fn read_string_lossy(&mut self, len: usize) -> io::Result<String> {
        Ok(String::from_utf8_lossy(&self.read_blob(len)?).into_owned())
    }

    // /// Read an UTF-8 string into the given buffer, returning an error if the data is not
    // /// valid UTF-8, and the given buffer is zero-ed out.
    // fn read_string_into(&mut self, dst: &mut str) -> io::Result<()> {
        
    //     let bytes = unsafe { dst.as_bytes_mut() };
    //     self.read_blob_into(bytes)?;

    //     // Here we just run UTF-8 validation, and zero out if it fails.
    //     std::str::from_utf8(bytes).map(|_| ()).map_err(|_| {
    //         bytes.fill(0);
    //         io::ErrorKind::InvalidData.into()
    //     })

    // }

    /// Read an UTF-8 string of a length that is specified with a packed u32
    /// before the actual vector.
    fn read_string_variable(&mut self) -> io::Result<String> {
        let blob = self.read_blob_variable()?;
        match String::from_utf8(blob) {
            Ok(s) => Ok(s),
            Err(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid utf8 string"))
        }
    }

    fn read_string_variable_lossy(&mut self) -> io::Result<String> {
        let blob = self.read_blob_variable()?;
        Ok(String::from_utf8_lossy(&blob).into_owned())
    }

    /// Read a null-terminated string of a fixed length, trailing zeros
    /// are ignored and if no zero is encountered, an invalid data error
    /// is returned.
    fn read_cstring(&mut self, len: usize) -> io::Result<String> {
        let mut buf = self.read_blob(len)?;
        let pos = buf.iter().position(|&o| o == 0)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
        buf.truncate(pos); // Truncate trailing zeros.
        String::from_utf8(buf).map_err(|_| io::ErrorKind::InvalidData.into())
    }

    /// Read a null-terminated string of unknown length.
    fn read_cstring_variable(&mut self) -> io::Result<String> {
        // The implementation is intentionally naive because it could be
        // speed up if the underlying read is buffered.
        let mut buf = Vec::new();
        loop {
            let b = self.read_u8()?;
            if b == 0 {
                break
            }
            buf.push(b);
        }
        String::from_utf8(buf).map_err(|_| io::ErrorKind::InvalidData.into())
    }

    #[inline]
    fn read_vec2(&mut self) -> io::Result<Vec2> {
        Ok(Vec2::new(
            self.read_f32()?,
            self.read_f32()?,
        ))
    }

    #[inline]
    fn read_vec3(&mut self) -> io::Result<Vec3> {
        Ok(Vec3::new(
            self.read_f32()?,
            self.read_f32()?,
            self.read_f32()?,
        ))
    }

    #[inline]
    fn read_vec4(&mut self) -> io::Result<Vec4> {
        Ok(Vec4::new(
            self.read_f32()?,
            self.read_f32()?,
            self.read_f32()?,
            self.read_f32()?,
        ))
    }

    /// Read a Python Pickle of the given `serde::Deserialize` type, this also
    /// reads the length of the pickle's data in the packed header.
    fn read_python_pickle(&mut self) -> io::Result<serde_pickle::Value> {
        let length = self.read_packed_u24()?;
        serde_pickle::value_from_reader(self.take(length as _), serde_pickle_de_options())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("invalid python pickle: {e}")))
    }

    /// Read the size header for a single structure. To read the header of
    /// a vector, see `read_vector_head`.
    fn read_single_head(&mut self) -> io::Result<usize> {
        Ok(self.read_u32()? as usize)
    }

    /// Read header for vector of structure, returns `(size, count)` with the
    /// number of structure of the given size, total size is `size * count`.
    fn read_vector_head(&mut self) -> io::Result<(usize, usize)> {
        let sec_size = self.read_u32()? as usize;
        let sec_count = self.read_u32()? as usize;
        Ok((sec_size, sec_count))
    }

    /// Read a full vector of structure, use a function to convert each structures'
    /// bytes to an object, returns a vector with all vector's objects.
    fn read_vector<F, T>(&mut self, mut func: F) -> io::Result<Vec<T>>
    where
        F: FnMut(&mut Cursor<&Vec<u8>>) -> io::Result<T>
    {

        let (sec_size, sec_count) = self.read_vector_head()?;

        let mut buf = Vec::with_capacity(sec_size);
        buf.resize(sec_size, 0);

        let mut data = Vec::with_capacity(sec_count);
        for _ in 0..sec_count {
            self.read_exact(&mut buf[..])?;
            data.push((func)(&mut Cursor::new(&buf))?);
        }

        Ok(data)

    }

}


/// An extension to the [`Write`] trait specifically used to decode WG formats
/// and used for network protocol.
pub trait WgWriteExt: Write {

    #[inline]
    fn write_uint(&mut self, n: u64, nbytes: usize) -> io::Result<()> {
        WriteBytesExt::write_uint::<LE>(self, n, nbytes)
    }

    #[inline]
    fn write_int(&mut self, n: i64, nbytes: usize) -> io::Result<()> {
        WriteBytesExt::write_int::<LE>(self, n, nbytes)
    }

    /// Writes an unsigned 8 bit integer to the underlying writer.
    #[inline]
    fn write_u8(&mut self, n: u8) -> io::Result<()> {
        WriteBytesExt::write_u8(self, n)
    }

    /// Writes a signed 8 bit integer to the underlying writer.
    #[inline]
    fn write_i8(&mut self, n: i8) -> io::Result<()> {
        WriteBytesExt::write_i8(self, n)
    }

    /// Writes an unsigned 16 bit integer to the underlying writer.
    #[inline]
    fn write_u16(&mut self, n: u16) -> io::Result<()> {
        WriteBytesExt::write_u16::<LE>(self, n)
    }

    /// Writes a signed 16 bit integer to the underlying writer.
    #[inline]
    fn write_i16(&mut self, n: i16) -> io::Result<()> {
        WriteBytesExt::write_i16::<LE>(self, n)
    }

    /// Writes an unsigned 24 bit integer to the underlying writer.
    #[inline]
    fn write_u24(&mut self, n: u32) -> io::Result<()> {
        WriteBytesExt::write_u24::<LE>(self, n)
    }

    /// Writes a signed 24 bit integer to the underlying writer.
    #[inline]
    fn write_i24(&mut self, n: i32) -> io::Result<()> {
        WriteBytesExt::write_i24::<LE>(self, n)
    }

    /// Writes an unsigned 32 bit integer to the underlying writer.
    #[inline]
    fn write_u32(&mut self, n: u32) -> io::Result<()> {
        WriteBytesExt::write_u32::<LE>(self, n)
    }

    /// Writes a signed 32 bit integer to the underlying writer.
    #[inline]
    fn write_i32(&mut self, n: i32) -> io::Result<()> {
        WriteBytesExt::write_i32::<LE>(self, n)
    }

    /// Writes a packed unsigned 24 bit integer to the underlying writer.
    fn write_packed_u24(&mut self, n: u32) -> io::Result<()> {
        if n >= 255 {
            self.write_u8(255)?;
            self.write_u24(n)
        } else {
            self.write_u8(n as u8)
        }
    }

    /// Writes an unsigned 64 bit integer to the underlying writer.
    #[inline]
    fn write_u64(&mut self, n: u64) -> io::Result<()> {
        WriteBytesExt::write_u64::<LE>(self, n)
    }

    /// Writes a signed 64 bit integer to the underlying writer.
    #[inline]
    fn write_i64(&mut self, n: i64) -> io::Result<()> {
        WriteBytesExt::write_i64::<LE>(self, n)
    }

    /// Writes a IEEE754 single-precision (4 bytes) floating point number 
    /// to the underlying writer.
    #[inline]
    fn write_f32(&mut self, n: f32) -> io::Result<()> {
        WriteBytesExt::write_f32::<LE>(self, n)
    }

    /// Writes a IEEE754 double-precision (8 bytes) floating point number 
    /// to the underlying writer.
    #[inline]
    fn write_f64(&mut self, n: f64) -> io::Result<()> {
        WriteBytesExt::write_f64::<LE>(self, n)
    }

    /// Write a single boolean to the underlying writer.
    #[inline]
    fn write_bool(&mut self, b: bool) -> io::Result<()> {
        self.write_u8(b as _)
    }

    #[inline]
    fn write_blob(&mut self, data: &[u8]) -> io::Result<()> {
        self.write_all(data)
    }

    /// Write a blob with its packed length before the actual data.
    fn write_blob_variable(&mut self, data: &[u8]) -> io::Result<()> {
        self.write_packed_u24(data.len() as u32)?;
        self.write_blob(data)
    }

    /// Writes a string to the underlying writer. Note that the length of
    /// the string is not written.
    #[inline]
    fn write_string<S: AsRef<str>>(&mut self, s: S) -> io::Result<()> {
        self.write_blob(s.as_ref().as_bytes())
    }

    /// Write a string with its packed length before.
    #[inline]
    fn write_string_variable(&mut self, s: &str) -> io::Result<()> {
        self.write_blob_variable(s.as_bytes())
    }

    /// Writes a null-terminated string to the underlying writer.
    #[inline]
    fn write_cstring<S: AsRef<str>>(&mut self, s: S) -> io::Result<()> {
        self.write_string(s)?;
        self.write_u8(0)
    }

    /// Write the size header for a single structure. To write the header of
    /// a vector, see `write_vector_head`.
    fn write_single_head(&mut self, n: usize) -> io::Result<()> {
        self.write_u32(n as u32)
    }

    fn write_vec2(&mut self, vec: Vec2) -> io::Result<()> {
        self.write_f32(vec.x)?;
        self.write_f32(vec.y)?;
        Ok(())
    }

    fn write_vec3(&mut self, vec: Vec3) -> io::Result<()> {
        self.write_f32(vec.x)?;
        self.write_f32(vec.y)?;
        self.write_f32(vec.z)?;
        Ok(())
    }

    fn write_vec4(&mut self, vec: Vec4) -> io::Result<()> {
        self.write_f32(vec.x)?;
        self.write_f32(vec.y)?;
        self.write_f32(vec.z)?;
        self.write_f32(vec.w)?;
        Ok(())
    }

    /// Write a Python Pickle from the given `serde::Serialize` value, the pickle's
    /// data is prefixed with the variable length of the data (like a variable blob
    /// or string).
    fn write_python_pickle(&mut self, value: &serde_pickle::Value) -> io::Result<()> {
        self.write_blob_variable(&serde_pickle::value_to_vec(value, serde_pickle_ser_options()).unwrap())
    }

    /// Write header for vector of structure.
    fn write_vector_head(&mut self, size: usize, count: usize) -> io::Result<()> {
        self.write_u32(size as u32)?;
        self.write_u32(count as u32)
    }

    /// Write a vector of structure. Items give in the iterator are converted 
    /// through the given function. This function take the 
    fn write_vector<T, I, F>(&mut self, vec: &[T], size: usize, mut func: F) -> io::Result<()>
    where
        F: FnMut(&T, &mut Self),
    {
        self.write_vector_head(size, vec.len())?;
        for elt in vec {
            (func)(elt, self);
        }
        Ok(())
    }

}

impl<R: Read + ?Sized> WgReadExt for R {}
impl<W: Write + ?Sized> WgWriteExt for W {}


/// A wrapper for a [`Read`] or [`Write`] implementor that will increment
/// an internal counter when a byte is either read or written.
pub struct IoCounter<I> {
    inner: I,
    count: usize,
}

impl<I> IoCounter<I> {

    #[inline]
    pub fn new(inner: I) -> Self {
        Self {
            inner,
            count: 0,
        }
    }

    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn into_inner(self) -> I {
        self.inner
    }

}

impl<R: Read> Read for IoCounter<R> {

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = self.inner.read(buf)?;
        self.count += len;
        Ok(len)
    }

    // TODO: Support vectored read later...

}

impl<W: Write> Write for IoCounter<W> {

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = self.inner.write(buf)?;
        self.count += len;
        Ok(len)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.inner.write_all(buf)?;
        self.count += buf.len();
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

}


#[derive(Debug)]
pub struct SliceCursor<'a>(&'a [u8]);

impl<'a> SliceCursor<'a> {

    pub fn new(slice: &'a [u8]) -> Self {
        Self(slice)
    }

    pub fn inner(&self) -> &'a [u8] {
        self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn pop_front(&mut self, len: usize) -> Option<&'a [u8]> {
        let (ret, rest) = self.0.split_at_checked(len)?;
        self.0 = rest;
        Some(ret)
    }

    // pub fn pop_front_read(&mut self, len: usize) -> Option<impl Read + 'a> {
    //     self.pop_front(len).map(Cursor::new)
    // }

    pub fn pop_back(&mut self, len: usize) -> Option<&'a [u8]> {
        if len  > self.0.len() { return None; }
        let (rest, ret) = self.0.split_at(self.0.len() - len);
        self.0 = rest;
        Some(ret)
    }

    // pub fn pop_back_read(&mut self, len: usize) -> Option<impl Read + 'a> {
    //     self.pop_back(len).map(Cursor::new)
    // }

}


#[inline]
pub fn serde_pickle_de_options() -> serde_pickle::DeOptions {
    serde_pickle::DeOptions::new().decode_strings_relaxed().replace_recursive_structures()
}

#[inline]
pub fn serde_pickle_ser_options() -> serde_pickle::SerOptions {
    serde_pickle::SerOptions::new().proto_v2()
}
