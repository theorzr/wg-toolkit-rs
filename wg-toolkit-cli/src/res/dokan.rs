//! Dokan userspace filesystem forwarding resources.

use std::io::{self, Read, Seek};
use std::sync::{Mutex, RwLock};
use std::collections::HashMap;
use std::time::SystemTime;

use winapi::shared::ntstatus;
use winapi::um::winnt;

use dokan::{
    CreateFileInfo, DiskSpaceInfo, FileInfo, FileSystemHandler, FileSystemMounter,
    FillDataError, FillDataResult, FindData, MountOptions, OperationInfo, OperationResult,
    VolumeInfo
};

use widestring::{U16CStr, U16CString};

use wgtk::res::fs::{ResFilesystem, ResReadDir, ResReadFile};

use crate::{CliOptions, CliResult, ResDokanArgs};


pub(super) fn cmd_res_dokan(_opts: CliOptions, args: ResDokanArgs, fs: &ResFilesystem) -> CliResult<()> {

    let handler = Handler::new(fs.clone());

    let mount_point = U16CString::from_str(&args.mount_path)
        .map_err(|_| format!("Invalid mount point containing nul-character!"))?;

    let mount_options = MountOptions {
        ..Default::default()
    };

    dokan::init();
    
    let mut mounter = FileSystemMounter::new(&handler, &mount_point, &mount_options);

    let mounted = mounter.mount()
        .map_err(|e| format!("Failed to mount Dokan filesystem: {e}"))?;

    drop(mounted);

    dokan::shutdown();

    Ok(())

}


pub struct Handler {
    pub fs: ResFilesystem,
    pub file_indices: RwLock<HashMap<String, u64>>,
}

#[derive(Debug)]
pub struct Node {
    path: String,
    index: u64,
    size: u64,
    read: NodeRead,
}

#[derive(Debug)]
pub enum NodeRead {
    File(Mutex<FileRead>),
    Dir(Mutex<DirRead>),
}

#[derive(Debug)]
pub struct FileRead {
    inner: ResReadFile,
    offset: u64,
}

#[derive(Debug)]
pub struct DirRead {
    inner: ResReadDir,
}

impl Handler {
    pub fn new(fs: ResFilesystem) -> Self {
        Self {
            fs,
            file_indices: RwLock::new(HashMap::new()),
        }
    }
}

impl<'c, 'h: 'c> FileSystemHandler<'c, 'h> for Handler {
    
    type Context = Node;

    fn create_file(
        &'h self,
        file_name: &U16CStr,
        _security_context: &dokan::IO_SECURITY_CONTEXT,
        _desired_access: winnt::ACCESS_MASK,
        _file_attributes: u32,
        _share_access: u32,
        create_disposition: u32,
        create_options: u32,
        _info: &mut dokan::OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<CreateFileInfo<Self::Context>> {
        
        if create_disposition != dokan_sys::win32::FILE_OPEN {
            eprintln!("Invalid create_disposition: 0x{create_disposition:04X} (expecting FILE_OPEN)");
            return Err(ntstatus::STATUS_ACCESS_DENIED);
        }

        if create_options & dokan_sys::win32::FILE_DELETE_ON_CLOSE != 0 {
            eprintln!("Invalid create_options: 0x{create_options:04X} (unsupported FILE_DELETE_ON_CLOSE)");
            return Err(ntstatus::STATUS_INVALID_PARAMETER);
        }

        let path = {
            let mut buf = String::with_capacity(file_name.len());
            for (i, ch) in char::decode_utf16(file_name.as_slice().iter().copied()).enumerate() {
                match ch {
                    Ok('\\') if i == 0 => continue,
                    Ok('\\') => buf.push('/'),
                    Ok(ch) => buf.push(ch),
                    Err(_) => return Err(ntstatus::STATUS_OBJECT_NAME_INVALID),
                }
            }
            buf
        };

        match self.fs.stat(&path) {
            Ok(stat) => {

                let indices = self.file_indices.read().unwrap();
                let index = match indices.get(&path).copied() {
                    Some(index) => index,
                    None => {
                        drop(indices);
                        let mut indices = self.file_indices.write().unwrap();
                        let new_index = indices.len() as u64 + 1;  // +1 to avoid index 0...
                        indices.insert(path.clone(), new_index);
                        new_index
                    }
                };

                if stat.is_file() {

                    if create_options & dokan_sys::win32::FILE_DIRECTORY_FILE != 0 {
                        return Err(ntstatus::STATUS_NOT_A_DIRECTORY);
                    }

                    let read = match self.fs.read(&path) {
                        Ok(dir) => dir,
                        Err(e) => {
                            eprintln!("Failed to open file: {e} ({path})");
                            return Err(ntstatus::STATUS_UNSUCCESSFUL);
                        }
                    };

                    // eprintln!("Open file: {path}");

                    return Ok(CreateFileInfo {
                        context: Node {
                            path,
                            index,
                            size: stat.size(),
                            read: NodeRead::File(Mutex::new(FileRead {
                                inner: read,
                                offset: 0,
                            })),
                        },
                        is_dir: false,
                        new_file_created: false,
                    });

                } else {

                    if create_options & dokan_sys::win32::FILE_NON_DIRECTORY_FILE != 0 {
                        return Err(ntstatus::STATUS_FILE_IS_A_DIRECTORY);
                    }

                    let read = match self.fs.read_dir(&path) {
                        Ok(dir) => dir,
                        Err(e) => {
                            eprintln!("Failed to open dir: {e} ({path})");
                            return Err(ntstatus::STATUS_UNSUCCESSFUL);
                        }
                    };

                    // eprintln!("Open dir: {path}");

                    return Ok(CreateFileInfo {
                        context: Node {
                            path,
                            index,
                            size: 0,
                            read: NodeRead::Dir(Mutex::new(DirRead {
                                inner: read,
                            })),
                        },
                        is_dir: true,
                        new_file_created: false,
                    });

                }

            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ntstatus::STATUS_OBJECT_NAME_NOT_FOUND);
            }
            Err(e) => {
                eprintln!("Failed to stat: {e} ({path})");
                return Err(ntstatus::STATUS_UNSUCCESSFUL);
            }
        }

    }
    
    fn cleanup(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) {
    }
    
    fn close_file(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) {
        // match *context.lock().unwrap() {
        //     Context::File { ref file_path, .. } => {
        //         eprintln!("Close file: {file_path}");
        //     }
        //     Context::Dir { ref file_path, .. } => {
        //         eprintln!("Close dir: {file_path}");
        //     }
        // }
    }
    
    fn read_file(
        &'h self,
        _file_name: &U16CStr,
        offset: i64,
        buffer: &mut [u8],
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<u32> {

        let NodeRead::File(read) = &context.read else {
            return Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST);
        };

        let mut read = read.lock().unwrap();

        if offset < 0 {
            eprintln!("Negative read file offset: {offset} ({})", context.path);
            return Err(ntstatus::STATUS_INVALID_PARAMETER);
        }

        if buffer.len() > u32::MAX as usize {
            eprintln!("Buffer is too large: {} ({})", buffer.len(), context.path);
            return Err(ntstatus::STATUS_INVALID_PARAMETER);
        }

        let offset = offset as u64;

        if read.offset != offset {
            match read.inner.seek(io::SeekFrom::Start(offset)) {
                Ok(_) => {
                    read.offset = offset;
                }
                Err(e) => {
                    eprintln!("Failed to seek file: {e} ({})", context.path);
                    return Err(ntstatus::STATUS_UNSUCCESSFUL);
                }
            }
        }

        match read.inner.read(buffer) {
            Ok(len) => {
                read.offset += len as u64;
                Ok(len as u32)
            }
            Err(e) => {
                eprintln!("Failed to read file: {e} ({})", context.path);
                return Err(ntstatus::STATUS_UNSUCCESSFUL);
            }
        }

    }
    
    fn write_file(
        &'h self,
        _file_name: &U16CStr,
        _offset: i64,
        _buffer: &[u8],
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<u32> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn flush_file_buffers(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn get_file_information(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<FileInfo> {
        Ok(FileInfo {
            attributes: match context.read {
                NodeRead::File(_) => winnt::FILE_ATTRIBUTE_NORMAL,
                NodeRead::Dir(_) => winnt::FILE_ATTRIBUTE_DIRECTORY,
            },
            creation_time: SystemTime::UNIX_EPOCH,
            last_access_time: SystemTime::UNIX_EPOCH,
            last_write_time: SystemTime::UNIX_EPOCH,
            file_size: context.size,
            number_of_links: 1,
            file_index: context.index,
        })
    }

    fn find_files(
        &'h self,
        _file_name: &U16CStr,
        mut fill_find_data: impl FnMut(&FindData) -> FillDataResult,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<()> {
        
        let NodeRead::Dir(read) = &context.read else {
            return Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST);
        };

        let mut read = read.lock().unwrap();

        // eprintln!("Find files: {file_path}");

        for entry in &mut read.inner {

            let Ok(entry) = entry else { continue };
            let Ok(entry_path) = U16CString::from_str(entry.name()) else { continue };
            
            let stat = entry.stat();
            let find_data = FindData {
                attributes: if stat.is_dir() {
                    winnt::FILE_ATTRIBUTE_DIRECTORY
                } else {
                    winnt::FILE_ATTRIBUTE_NORMAL
                },
                creation_time: SystemTime::UNIX_EPOCH,
                last_access_time: SystemTime::UNIX_EPOCH,
                last_write_time: SystemTime::UNIX_EPOCH,
                file_size: stat.size(),
                file_name: entry_path,
            };

            match fill_find_data(&find_data) {
                Ok(()) => continue,
                Err(FillDataError::NameTooLong) => continue,
                Err(FillDataError::BufferFull) => return Err(ntstatus::STATUS_BUFFER_OVERFLOW),
            }
        }

        Ok(())

    }
    
    fn find_files_with_pattern(
        &'h self,
        _file_name: &U16CStr,
        _pattern: &U16CStr,
        _fill_find_data: impl FnMut(&FindData) -> FillDataResult,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_NOT_IMPLEMENTED)
    }
    
    fn set_file_attributes(
        &'h self,
        _file_name: &U16CStr,
        _file_attributes: u32,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn set_file_time(
        &'h self,
        _file_name: &U16CStr,
        _creation_time: dokan::FileTimeOperation,
        _last_access_time: dokan::FileTimeOperation,
        _last_write_time: dokan::FileTimeOperation,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn delete_file(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn delete_directory(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn move_file(
        &'h self,
        _file_name: &U16CStr,
        _new_file_name: &U16CStr,
        _replace_if_existing: bool,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn set_end_of_file(
        &'h self,
        _file_name: &U16CStr,
        _offset: i64,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn set_allocation_size(
        &'h self,
        _file_name: &U16CStr,
        _alloc_size: i64,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn lock_file(
        &'h self,
        _file_name: &U16CStr,
        _offset: i64,
        _length: i64,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn unlock_file(
        &'h self,
        _file_name: &U16CStr,
        _offset: i64,
        _length: i64,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_NOT_IMPLEMENTED)
    }
    
    fn get_disk_free_space(
        &'h self,
        _info: &OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<DiskSpaceInfo> {
        Ok(DiskSpaceInfo {
            byte_count: 0,
            free_byte_count: 0,
            available_byte_count: 0,
        })
    }
    
    fn get_volume_information(
        &'h self,
        _info: &OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<VolumeInfo> {
        Ok(VolumeInfo {
            name: U16CString::from_str("wgtk res").unwrap(),
            serial_number: 0,
            max_component_length: 255,  // ARBITRARY
            fs_flags: 
                winnt::FILE_CASE_PRESERVED_NAMES |
                winnt::FILE_CASE_SENSITIVE_SEARCH |
                winnt::FILE_UNICODE_ON_DISK |
                winnt::FILE_READ_ONLY_VOLUME,
            fs_name: U16CString::from_str("NTFS").unwrap(),
        })
    }
    
    fn mounted(
        &'h self,
        _mount_point: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<()> {
        Ok(())
    }
    
    fn unmounted(&'h self, _info: &OperationInfo<'c, 'h, Self>) -> OperationResult<()> {
        Ok(())
    }
    
    fn get_file_security(
        &'h self,
        _file_name: &U16CStr,
        _security_information: u32,
        _security_descriptor: winnt::PSECURITY_DESCRIPTOR,
        _buffer_length: u32,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<u32> {
        Err(ntstatus::STATUS_NOT_IMPLEMENTED)
    }
    
    fn set_file_security(
        &'h self,
        _file_name: &U16CStr,
        _security_information: u32,
        _security_descriptor: winnt::PSECURITY_DESCRIPTOR,
        _buffer_length: u32,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }
    
    fn find_streams(
        &'h self,
        _file_name: &U16CStr,
        _fill_find_stream_data: impl FnMut(&dokan::FindStreamData) -> FillDataResult,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Self::Context,
    ) -> OperationResult<()> {
        Err(ntstatus::STATUS_INVALID_DEVICE_REQUEST)
    }

}