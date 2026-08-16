//! FUSE implementation for game resources.

use std::io::{self, Read, Seek};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use std::ffi::OsStr;

use fuser::{
    AccessFlags, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo, MountOption, OpenAccMode, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, Request, SessionACL,
};

use wgtk::res::{ResFilesystem, ResReadDir, ResReadFile, ResStat};

use crate::{CliOptions, CliResult, ResFuseArgs};


/// Resources are immutable while the filesystem is mounted, so the kernel can cache
/// entries and attributes for a long time.
const TTL: Duration = Duration::from_secs(60);


pub(super) fn cmd_res_fuse(_opts: CliOptions, args: ResFuseArgs, fs: &ResFilesystem) -> CliResult<()> {

    let handler = Handler::new(fs.clone());

    let mut config = Config::default();
    config.mount_options.push(MountOption::FSName("wgtk-res".to_string()));
    config.mount_options.push(MountOption::RO);
    config.mount_options.push(MountOption::NoExec);
    config.mount_options.push(MountOption::NoDev);
    config.mount_options.push(MountOption::NoSuid);
    config.mount_options.push(MountOption::NoAtime);

    fuser::mount(handler, args.mount_path, &config)
        .map_err(|e| format!("Failed to mount FUSE filesystem: {e}"))

}


pub struct Handler {
    pub fs: ResFilesystem,
    /// Two-ways mapping between inode numbers and resource paths, FUSE only gives us
    /// inode numbers so we need to remember every path we have exposed to the kernel.
    inodes: RwLock<Inodes>,
    /// All currently opened files and directories, mapped from their file handle.
    nodes: RwLock<HashMap<u64, Arc<Node>>>,
    /// Next file handle to be allocated, handle zero is never used.
    next_handle: AtomicU64,
}

/// Mapping of inode numbers to resource paths and back.
#[derive(Debug)]
struct Inodes {
    /// Path of each known inode, the inode number is the index plus one, so the root
    /// directory (with its empty path) has the inode number 1, as required by FUSE.
    paths: Vec<Arc<str>>,
    /// Reverse mapping of the paths above.
    numbers: HashMap<Arc<str>, u64>,
}

/// An opened file or directory, referenced by its file handle.
#[derive(Debug)]
pub struct Node {
    path: Arc<str>,
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
    /// Current position of the inner reader, used to avoid seeking on sequential reads.
    offset: u64,
    size: u64,
}

#[derive(Debug)]
pub struct DirRead {
    inner: ResReadDir,
    /// Entries already pulled out of the iterator, the kernel reads a directory with
    /// successive calls at increasing offsets, and may restart from any offset, so we
    /// need to keep every entry we have yielded.
    entries: Vec<DirEntry>,
    /// Set when the inner iterator has been fully consumed.
    complete: bool,
}

#[derive(Debug)]
struct DirEntry {
    ino: INodeNo,
    name: String,
    kind: FileType,
}

impl Handler {

    pub fn new(fs: ResFilesystem) -> Self {

        // The root directory is the empty path and is always inode 1.
        let root: Arc<str> = Arc::from("");

        Self {
            fs,
            inodes: RwLock::new(Inodes {
                paths: vec![Arc::clone(&root)],
                numbers: HashMap::from([(root, 1)]),
            }),
            nodes: RwLock::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
        }

    }

    /// Get the resource path of the given inode number, if known.
    fn path(&self, ino: INodeNo) -> Option<Arc<str>> {
        let index = ino.0.checked_sub(1)? as usize;
        self.inodes.read().unwrap().paths.get(index).cloned()
    }

    /// Get the inode number of the given resource path, allocating a new one if this
    /// path has never been exposed to the kernel.
    fn inode(&self, path: &str) -> INodeNo {

        let inodes = self.inodes.read().unwrap();
        if let Some(&ino) = inodes.numbers.get(path) {
            return INodeNo(ino);
        }

        drop(inodes);
        let mut inodes = self.inodes.write().unwrap();

        // Another thread may have inserted it in between both locks.
        if let Some(&ino) = inodes.numbers.get(path) {
            return INodeNo(ino);
        }

        let path: Arc<str> = Arc::from(path);
        inodes.paths.push(Arc::clone(&path));
        let ino = inodes.paths.len() as u64;  // Index plus one...
        inodes.numbers.insert(path, ino);
        INodeNo(ino)

    }

    /// Get the inode number of the parent directory of the given path, the root
    /// directory being its own parent.
    fn parent_inode(&self, path: &str) -> INodeNo {
        match path.rsplit_once('/') {
            Some((parent_path, _)) => self.inode(parent_path),
            None => INodeNo::ROOT,
        }
    }

    /// Register a newly opened node and return its file handle.
    fn insert_node(&self, node: Node) -> FileHandle {
        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
        self.nodes.write().unwrap().insert(handle, Arc::new(node));
        FileHandle(handle)
    }

    /// Get a previously opened node from its file handle.
    fn node(&self, fh: FileHandle) -> Option<Arc<Node>> {
        self.nodes.read().unwrap().get(&fh.0).map(Arc::clone)
    }

    /// Forget a previously opened node, closing the underlying reader.
    fn remove_node(&self, fh: FileHandle) {
        self.nodes.write().unwrap().remove(&fh.0);
    }

    /// Build the file attributes of a resource, resources have no timestamp and are
    /// owned by whoever is asking for them, they are always read-only.
    fn attr(&self, req: &Request, ino: INodeNo, stat: &ResStat) -> FileAttr {
        FileAttr {
            ino,
            size: stat.size(),
            blocks: stat.size().div_ceil(512),
            atime: SystemTime::UNIX_EPOCH,
            mtime: SystemTime::UNIX_EPOCH,
            ctime: SystemTime::UNIX_EPOCH,
            crtime: SystemTime::UNIX_EPOCH,
            kind: if stat.is_dir() { FileType::Directory } else { FileType::RegularFile },
            perm: if stat.is_dir() { 0o555 } else { 0o444 },
            nlink: if stat.is_dir() { 2 } else { 1 },
            uid: req.uid(),
            gid: req.gid(),
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

}

/// Map a resource filesystem error to an error number, a missing resource is expected
/// and therefore not reported, anything else is unexpected and worth printing.
fn errno(err: &io::Error, path: &str) -> Errno {
    if err.kind() == io::ErrorKind::NotFound {
        Errno::ENOENT
    } else {
        eprintln!("Resource error: {err} ({path})");
        Errno::EIO
    }
}

impl Filesystem for Handler {

    fn lookup(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {

        let Some(parent_path) = self.path(parent) else {
            return reply.error(Errno::ENOENT);
        };

        let Some(name) = name.to_str() else {
            return reply.error(Errno::ENOENT);
        };

        let path = if parent_path.is_empty() {
            name.to_string()
        } else {
            format!("{parent_path}/{name}")
        };

        match self.fs.stat(&path) {
            Ok(stat) => {
                let ino = self.inode(&path);
                reply.entry(&TTL, &self.attr(req, ino, &stat), Generation(0));
            }
            Err(e) => reply.error(errno(&e, &path)),
        }

    }

    fn getattr(&self, req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {

        let Some(path) = self.path(ino) else {
            return reply.error(Errno::ENOENT);
        };

        match self.fs.stat(&*path) {
            Ok(stat) => reply.attr(&TTL, &self.attr(req, ino, &stat)),
            Err(e) => reply.error(errno(&e, &path)),
        }

    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {

        if flags.acc_mode() != OpenAccMode::O_RDONLY {
            return reply.error(Errno::EROFS);
        }

        let Some(path) = self.path(ino) else {
            return reply.error(Errno::ENOENT);
        };

        let stat = match self.fs.stat(&*path) {
            Ok(stat) => stat,
            Err(e) => return reply.error(errno(&e, &path)),
        };

        if stat.is_dir() {
            return reply.error(Errno::EISDIR);
        }

        let inner = match self.fs.read(&*path) {
            Ok(read) => read,
            Err(e) => return reply.error(errno(&e, &path)),
        };

        let handle = self.insert_node(Node {
            path,
            read: NodeRead::File(Mutex::new(FileRead {
                inner,
                offset: 0,
                size: stat.size(),
            })),
        });

        reply.opened(handle, FopenFlags::empty());

    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: ReplyData,
    ) {

        let Some(node) = self.node(fh) else {
            return reply.error(Errno::EBADF);
        };

        let NodeRead::File(read) = &node.read else {
            return reply.error(Errno::EISDIR);
        };

        let mut read = read.lock().unwrap();

        // Clamp to the file's end so we never allocate more than what can be read.
        let len = read.size.saturating_sub(offset).min(size as u64) as usize;
        if len == 0 {
            return reply.data(&[]);
        }

        if read.offset != offset {
            match read.inner.seek(io::SeekFrom::Start(offset)) {
                Ok(_) => read.offset = offset,
                Err(e) => return reply.error(errno(&e, &node.path)),
            }
        }

        // FUSE expects exactly the requested amount of data, except on end of file, so
        // we loop until the buffer is full because a single read may be short.
        let mut buf = vec![0u8; len];
        let mut cursor = 0;

        while cursor < len {
            match read.inner.read(&mut buf[cursor..]) {
                Ok(0) => break,
                Ok(n) => cursor += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    // The reader position is unknown after a failed read, force a seek
                    // on the next read of this handle.
                    read.offset = u64::MAX;
                    return reply.error(errno(&e, &node.path));
                }
            }
        }

        read.offset = offset + cursor as u64;
        buf.truncate(cursor);
        reply.data(&buf);

    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.remove_node(fh);
        reply.ok();
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {

        let Some(path) = self.path(ino) else {
            return reply.error(Errno::ENOENT);
        };

        let inner = match self.fs.read_dir(&*path) {
            Ok(read) => read,
            Err(e) => return reply.error(errno(&e, &path)),
        };

        let handle = self.insert_node(Node {
            path,
            read: NodeRead::Dir(Mutex::new(DirRead {
                inner,
                entries: Vec::new(),
                complete: false,
            })),
        });

        reply.opened(handle, FopenFlags::empty());

    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {

        let Some(node) = self.node(fh) else {
            return reply.error(Errno::EBADF);
        };

        let NodeRead::Dir(read) = &node.read else {
            return reply.error(Errno::ENOTDIR);
        };

        let mut read = read.lock().unwrap();

        // Offsets 0 and 1 are the implicit '.' and '..' entries, the real entries follow
        // and each entry is added with the offset the kernel should resume from.
        let mut next = offset;
        loop {

            let full = match next {
                0 => reply.add(ino, 1, FileType::Directory, "."),
                1 => reply.add(self.parent_inode(&node.path), 2, FileType::Directory, ".."),
                _ => {

                    let index = (next - 2) as usize;

                    // Lazily pull entries out of the iterator until we reach the one
                    // being requested, entries are kept for later calls.
                    while !read.complete && read.entries.len() <= index {
                        match read.inner.next() {
                            None => read.complete = true,
                            Some(Err(e)) => {
                                eprintln!("Failed to read dir entry: {e} ({})", node.path);
                                read.complete = true;
                            }
                            Some(Ok(entry)) => {
                                let entry = DirEntry {
                                    ino: self.inode(&entry.path()),
                                    name: entry.name().to_string(),
                                    kind: if entry.stat().is_dir() {
                                        FileType::Directory
                                    } else {
                                        FileType::RegularFile
                                    },
                                };
                                read.entries.push(entry);
                            }
                        }
                    }

                    let Some(entry) = read.entries.get(index) else {
                        break;
                    };

                    reply.add(entry.ino, next + 1, entry.kind, &entry.name)

                }
            };

            if full {
                break;
            }

            next += 1;

        }

        reply.ok();

    }

    fn releasedir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        reply: ReplyEmpty,
    ) {
        self.remove_node(fh);
        reply.ok();
    }

    fn access(&self, _req: &Request, ino: INodeNo, mask: AccessFlags, reply: ReplyEmpty) {

        let Some(path) = self.path(ino) else {
            return reply.error(Errno::ENOENT);
        };

        if mask.contains(AccessFlags::W_OK) {
            return reply.error(Errno::EACCES);
        }

        match self.fs.stat(&*path) {
            // Only directories can be traversed, files are not executable.
            Ok(stat) if mask.contains(AccessFlags::X_OK) && !stat.is_dir() => {
                reply.error(Errno::EACCES)
            }
            Ok(_) => reply.ok(),
            Err(e) => reply.error(errno(&e, &path)),
        }

    }

}
