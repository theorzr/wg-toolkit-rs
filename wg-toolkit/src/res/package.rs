//! Package file codec.
//! 
//! Packages are ZIP files with constrained flags and properties,
//! for example no encryption and no compression is needed.
//! 
//! Following official specification: 
//! https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT

use std::io::{self, Seek, Read, SeekFrom, BufReader};
use std::sync::Arc;
use std::fmt;

use crate::util::io::WgReadExt;


/// Signature for the Local File Header structure.
#[allow(unused)]
const LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x04034b50;

/// Signature for the Central Directory Header structure.
const CENTRAL_DIRECTORY_HEADER_SIGNATURE: u32 = 0x02014b50;

/// Signature for the end of central directory.
const END_OF_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x06054b50;


/// A package-specialized ZIP reader that is optimized for reading all file names as fast
/// as possible. This reader only accesses file immutably. This reader ignores folders.
/// 
/// If the underlying stream (such as file) is modified while this reader is create,
/// subsequent file reads are really likely to error (will never panic!).
pub struct PackageReader<R: Read + Seek> {
    /// Underlying reader. Not buffered because once the header has been parsed, the data
    /// reading will be spread way over the default 8 KB block of the buffered reader,
    /// so this is useless.
    inner: R,
    /// This string buffer holds all file names, so only one allocation is needed for all
    /// names. We use an immutable ref counted buffer because we don't alter it afterward,
    /// and it might be shared between multiple readers.
    name_buffer: Arc<str>,
    /// All informations about each file available to the reader. Behind ref counted for
    /// the same reason as [`Self::name_buffer`].
    file_infos: Arc<[PackageFileInternalInfo]>,
}

/// Internal metadata about a file.
#[derive(Debug)]
struct PackageFileInternalInfo {
    /// Offset of the package name into the global name buffer.
    name_offset: u32,
    /// Length of the file name in the global name buffer.
    name_len: u16,
    /// Offset within the file of the local header of this file.
    header_offset: u32,
    /// Expected uncompressed size for this file, packages should not compress files
    /// so the compressed size should be equal, but this will be checked later if the
    /// file is actually opened.
    size: u32,
}

impl<R: Read + Seek> PackageReader<R> {

    /// Create a package reader with the underlying read+seek implementor.
    pub fn new(mut reader: R) -> io::Result<Self> {
        
        const HEADER_MIN_SIZE: u64 = 22;
        const HEADER_MAX_SIZE: u64 = 22 + u16::MAX as u64;

        // Here we try to find the position of the End of Central Directory.
        let file_length = reader.seek(SeekFrom::End(0))?;
        let mut eocd_pos = file_length.checked_sub(HEADER_MIN_SIZE)
            .ok_or(io::Error::new(io::ErrorKind::InvalidData, "cannot find end of central directory, file too short"))?;
        let eocd_pos_bound = file_length.saturating_sub(HEADER_MAX_SIZE);

        // A successful return from this loop means we found the EoCD position.
        loop {

            reader.seek(SeekFrom::Start(eocd_pos))?;
            if reader.read_u32()? == END_OF_CENTRAL_DIRECTORY_SIGNATURE {
                break;
            }

            if eocd_pos == eocd_pos_bound {
                // If we didn't find signature on the lower bound.
                return Err(io::Error::new(io::ErrorKind::InvalidData, "cannot find end of central directory, reached lower bound"));
            }

            eocd_pos = eocd_pos.checked_sub(1)
                .ok_or(io::Error::new(io::ErrorKind::InvalidData, "cannot find end of central directory, underflow position"))?;

        }

        // Here we finish parsing the EoCD (we are placed just after the directory signature).
        let disk_number = reader.read_u16()?;
        let disk_with_central_directory = reader.read_u16()?;

        if disk_number != disk_with_central_directory {
            // Multi-disk ZIP files are not valid packages.
            return Err(io::Error::new(io::ErrorKind::InvalidData, "multi-disk is not supported for packages zip"));
        }

        let number_of_files_on_this_disk = reader.read_u16()?;
        let number_of_files = reader.read_u16()?;

        if number_of_files_on_this_disk != number_of_files {
            // Same as above, no multi-disk, so the number of files must be coherent.
            return Err(io::Error::new(io::ErrorKind::InvalidData, "multi-disk is not supported for packages zip"));
        }

        let _central_directory_size = reader.read_u32()?;
        let central_directory_offset = reader.read_u32()?;

        let comment_length = reader.read_u16()?;
        if comment_length != 0 {
            // Not expecting comments on packages.
            return Err(io::Error::new(io::ErrorKind::InvalidData, "comments are not supported for packages zip"));
        }

        // Now we can start parsing all Central Directory Headers.
        // Seek to the first Central Directory Header, reading is ready.
        reader.seek(SeekFrom::Start(central_directory_offset as u64))?;

        // For decoding the package structure we use a buffered reader to optimize
        // our random reads.
        let mut reader = BufReader::new(reader);

        // At start, we only read file names and optimize their storage, the actual file
        // header, size, flags will be read only when the file is accessed, here we only
        // read file name and store the offset header.
        // On average in World of Tanks packages, there is 70 bytes per file name.
        let mut name_buffer = Vec::with_capacity(number_of_files as usize * 70);
        let mut file_infos = Vec::with_capacity(number_of_files as usize);

        for _ in 0..number_of_files {

            if reader.read_u32()? != CENTRAL_DIRECTORY_HEADER_SIGNATURE {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid central directory header signature"));
            }

            // Skip most of the header that we don't care at this point.
            reader.seek_relative(20)?;
            // Uncompressed size is used as 
            let uncompressed_size = reader.read_u32()?;
            // Then we read all variable lengths.
            let file_name_len = reader.read_u16()?;
            // Read both fields at once because we want ot check that it's zero.
            let extra_field_file_comment_len = reader.read_u32()?;
            // Skip again, disk num, file attrs.
            reader.seek_relative(8)?;
            // Then read the offset of the local file header.
            let relative_offset = reader.read_u32()?;

            // Extra field and comment are not supported nor used by Wargaming.
            if extra_field_file_comment_len != 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "comments on files are not supported for packages zip"));
            }
            
            // Start by increasing the buffer capacity.
            let name_offset = name_buffer.len() as u32;  // FIXME: Checked cast
            name_buffer.resize(name_buffer.len() + file_name_len as usize, 0);
            let this_name_buffer = &mut name_buffer[name_offset as usize..][..file_name_len as usize];
            reader.read_exact(this_name_buffer)?;

            // If the name buffer is empty or ends with a slash, just ignore that because
            // it's a folder and don't keep folders. We rollback changes to name buffer
            // and continue on next iteration.
            if let None | Some(b'/') = this_name_buffer.last() {
                name_buffer.truncate(name_offset as usize);
                continue;
            }
            
            // Push the metadata to the files array.
            file_infos.push(PackageFileInternalInfo {
                name_offset,
                name_len: file_name_len,
                header_offset: relative_offset,
                size: uncompressed_size,
            });

        }
        
        let name_buffer = String::from_utf8(name_buffer).unwrap();

        Ok(Self { 
            inner: reader.into_inner(), 
            name_buffer: Arc::from(name_buffer),
            file_infos: Arc::from(file_infos),
        })

    }

    /// A fast clone of this package reader into a new fully independent reader, this 
    /// will reuse the file list of the current reader. **The caller must ensure** that
    /// this reader points to the same data as the current reader, if not the case,
    /// file informations may be wrong and subsequent file reads are likely to return 
    /// error, **this will never panic and not cause any UB!**
    pub fn clone_with<NewR: Read + Seek>(&self, reader: NewR) -> PackageReader<NewR> {
        PackageReader { 
            inner: reader, 
            name_buffer: Arc::clone(&self.name_buffer),
            file_infos: Arc::clone(&self.file_infos),
        }
    }

    /// Return the number of files in the package.
    #[inline]
    pub fn len(&self) -> usize {
        self.file_infos.len()
    }

    /// Return an iterator over all file info in the package. The position of files in 
    /// this iterator is their index that can be used when reading from index, using
    /// the [`Self::read_by_index()`] method.
    pub fn infos(&self) -> impl Iterator<Item = PackageFileInfo<'_>> {
        self.file_infos.iter().map(|info| {
            PackageFileInfo {
                name: &self.name_buffer[info.name_offset as usize..][..info.name_len as usize],
                size: info.size,
            }
        })
    }

    /// Get file information from its index.
    pub fn info_by_index(&self, file_index: usize) -> Option<PackageFileInfo<'_>> {
        self.file_infos.get(file_index).map(|info| {
            PackageFileInfo {
                name: &self.name_buffer[info.name_offset as usize..][..info.name_len as usize],
                size: info.size,
            }
        })
    }

    // Find a file index from its name, this function check all names so it may take some
    // time, it is preferable to keep an index 
    pub fn index_by_name(&self, file_name: &str) -> Option<usize> {
        self.infos().position(|info| info.name == file_name)
    }

    /// Open a package file by its name and return a borrowed reader if successful.
    pub fn read_by_name(&mut self, file_name: &str) -> io::Result<PackageFileReader<&'_ mut R>> {
        let file_index = self.index_by_name(file_name)
            .ok_or(io::Error::from(io::ErrorKind::NotFound))?;
        self.read_by_index(file_index)
    }

    /// Open a package file by its index and return a borrowed reader if successful.
    /// 
    /// Note that the returned reader has no buffered over the original reader given at
    /// construction, you should handle buffering if necessary.
    pub fn read_by_index(&mut self, file_index: usize) -> io::Result<PackageFileReader<&'_ mut R>> {

        let info = self.file_infos.get(file_index)
            .ok_or(io::Error::from(io::ErrorKind::NotFound))?;

        // Start to the start of the header.
        self.inner.seek(SeekFrom::Start(info.header_offset as u64))?;
        if self.inner.read_u32()? != LOCAL_FILE_HEADER_SIGNATURE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid local file header signature"));
        }

        // Skip version needed to extract.
        self.inner.seek(SeekFrom::Current(2))?;
        let flags = self.inner.read_u16()?;
        let compression_method = self.inner.read_u16()?;
        // Skip file time/date/crc32
        self.inner.seek(SeekFrom::Current(2 + 2 + 4))?;
        let compressed_size = self.inner.read_u32()?;
        let uncompressed_size = self.inner.read_u32()?;
        // Skip file name len + extra field length because it has already been checked.
        self.inner.seek(SeekFrom::Current(4 + info.name_len as i64))?;

        // Incoherent uncompressed size, different from central directory header!
        if uncompressed_size != info.size {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("incoherent size in local file header vs central directory header: {uncompressed_size} != {}", info.size)));
        }

        // Packages don't compress files.
        if compression_method != 0 || compressed_size != uncompressed_size {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("compressed files are not valid for packages zip: {compressed_size} (compressed) != {uncompressed_size} (uncompressed)")));
        }

        // Packages has no flag, no delayed crc32/size, no compression, no encryption.
        // We mask the bit 11, we don't know why it is used but apparently it can mean
        // to decode the filename/comment using UTF-8. So yes, we already do that.
        if flags & !2048 != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("flags on files are not valid for packages zip: {flags:016b}")));
        }
        
        // Now the reader's cursor is at data start, return the file reader.
        Ok(PackageFileReader {
            inner: &mut self.inner,
            initial_len: compressed_size,
            remaining_len: compressed_size,
        })

    }

}


/// Information about a package file that can be read.
#[derive(Debug, Clone)]
pub struct PackageFileInfo<'a> {
    /// The file name that should be used when reading.
    pub name: &'a str,
    /// The size of this file when read.
    pub size: u32,
}

/// A handle for reading a file in a package.
#[derive(Debug)]
pub struct PackageFileReader<R: Read + Seek> {
    /// Underlying reader.
    inner: R,
    /// Full length of this file.
    initial_len: u32,
    /// Remaining length to read from the file.
    remaining_len: u32,
}

impl<R: Read + Seek> PackageFileReader<R> {

    /// A fast copy of this package file reader. **The caller must ensure** that the
    /// new reader points to the same blob of data as the current one and has exact
    /// same seek boundaries. If not, this will result in incorrect yet safe data read.
    /// 
    /// This function immediately tries to seek to the same position, so it may error
    /// out if seek fails.
    /// 
    /// This method takes self as mutable reference because it needs to read the current
    /// seek position and it requires mutability.
    pub fn try_clone_with<NewR: Read + Seek>(&mut self, mut reader: NewR) -> io::Result<PackageFileReader<NewR>> {
        reader.seek(SeekFrom::Start(self.inner.stream_position()?))?;
        Ok(PackageFileReader {
            inner: reader,
            initial_len: self.initial_len,
            remaining_len: self.remaining_len,
        })
    }

}

impl<R: Read + Seek> Read for PackageFileReader<R> {

    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // If remaining length is zero, this will just do nothing.
        let len = buf.len().min(self.remaining_len as usize);
        let len = self.inner.read(&mut buf[..len])?;
        self.remaining_len -= len as u32;
        Ok(len)
    }

    #[inline]
    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        if (self.remaining_len as usize) < buf.len() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.inner.read_exact(buf)?;
        self.remaining_len -= buf.len() as u32;
        Ok(())
    }

}

impl<R: Read + Seek> Seek for PackageFileReader<R> {

    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {

        // Calculate the past length that has been read so far.
        let position = self.initial_len - self.remaining_len;

        let delta = match pos {
            SeekFrom::Start(offset) => {

                if (self.initial_len as u64) < offset {
                    return Err(io::ErrorKind::InvalidInput.into());
                }

                -(position as i64) + offset as i64

            }
            SeekFrom::End(offset) => {
                
                if offset > 0 || offset < -(self.initial_len as i64) {
                    return Err(io::ErrorKind::InvalidInput.into());
                }

                (self.remaining_len as i64) + offset

            }
            SeekFrom::Current(offset) => {

                // If we go forward but we don't have enough data.
                if offset > 0 && (self.remaining_len as i64) < offset {
                    return Err(io::ErrorKind::InvalidInput.into());
                } else if offset < 0 && (position as i64) < -offset {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
                
                offset

            }
        };

        self.inner.seek(SeekFrom::Current(delta))?;
        self.remaining_len = (self.remaining_len as i64 - delta) as u32;
        Ok((self.initial_len - self.remaining_len) as u64)

    }

    #[inline]
    fn stream_position(&mut self) -> io::Result<u64> {
        Ok((self.initial_len - self.remaining_len) as u64)
    }

}

impl<R: Read + Seek + fmt::Debug> fmt::Debug for PackageReader<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PackageReader")
            .field("inner", &self.inner)
            .field("name_buffer", &self.name_buffer.len())
            .field("file_infos", &self.file_infos.len()).finish()
    }
}
