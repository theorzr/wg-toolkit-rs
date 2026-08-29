//! Virtual read-only filesystem for browsing and reading the game's resources.

use core::fmt;
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::fs::{File, ReadDir};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::{fs, io};

use indexmap::IndexMap;

use super::package::{PackageReader, PackageFileReader};


/// Name of the directory storing packages in the "res/" directory.
const PACKAGES_DIR_NAME: &'static str = "packages";


/// A virtual read-only filesystem you can use to walk through the game's resources. This
/// filesystem is designed to work really fast on systems where it will run for a long
/// time and take advantage of its internal cache, but it will also work on run-once
/// environment such as CLI where the first answer latency must be minimal.
/// 
/// This filesystem is made to be shared between threads and immutably accessed.
/// 
/// Internally, this filesystem has a cache to improve response delay. The challenge is
/// that directories may reside in many packages, but files are present only in one
/// package.
#[derive(Debug, Clone)]
pub struct ResFilesystem {
    /// Shared part of the filesystem, used when returning independent handles like
    /// read dir iterator.
    shared: Arc<Shared>,
}

/// Immutable shared data 
#[derive(Debug)]
struct Shared {
    /// Path to the "res/" directory.
    dir_path: PathBuf,
    /// Mutable part of the shared data, behind mutex.
    mutable: Mutex<SharedMut>,
}

/// Mutex shared part of the resource filesystem.
#[derive(Debug)]
struct SharedMut {
    /// Pending packages to be opened and cached.
    pending_package_path: Vec<PathBuf>,
    /// Cache for opened package files.
    package_reader_cache: IndexMap<PathBuf, PackageReader<File>>,
    /// Package open errors are silently ignored when reading files and directories, so
    /// this vector contains the errors that may happen and can later be retrieved.
    package_open_errors: Vec<(PathBuf, io::Error)>,
    /// Cache for known files and directories.
    node_cache: NodeCache,
}

impl ResFilesystem {

    /// Create a new resources filesystem with the given options. This function is
    /// blocking while it is doing a rudimentary early indexing, so this may take some
    /// time.
    pub fn new<P: Into<PathBuf>>(dir_path: P) -> io::Result<Self> {

        let dir_path = dir_path.into();
        let mut pending_package_cache = Vec::new();

        for entry in fs::read_dir(dir_path.join(PACKAGES_DIR_NAME))? {
            
            let entry = entry?;
            let entry_type = entry.file_type()?;
            if !entry_type.is_file() {
                continue;
            }

            if !entry.file_name().as_encoded_bytes().ends_with(b".pkg") {
                continue;
            }

            pending_package_cache.push(entry.path());

        }

        Ok(Self { 
            shared: Arc::new(Shared {
                dir_path,
                mutable: Mutex::new(SharedMut {
                    pending_package_path: pending_package_cache,
                    package_reader_cache: IndexMap::new(),
                    package_open_errors: Vec::new(),
                    node_cache: NodeCache::new(),
                }),
            }),
        })

    }

    /// Get various information about a given path, wether its a directory or file, its
    /// size or the number of children the directory has.
    pub fn stat<P: AsRef<str>>(&self, node_path: P) -> io::Result<ResStat> {
        
        let node_path = node_path.as_ref();
        if node_path.starts_with('/') || node_path.ends_with('/') {
            return Err(io::ErrorKind::NotFound.into());
        }

        let native_file_path = self.shared.dir_path.join(node_path);
        match native_file_path.metadata() {
            Ok(metadata) => {
                return Ok(ResStat {
                    is_dir: metadata.is_dir(),
                    size: if metadata.is_dir() { 0 } else { metadata.len() },
                });
            }
            Err(_) => {}
        }

        self.shared.mutable.lock().unwrap().stat(node_path)

    }

    /// Read a file from its path in the resource filesystem.
    pub fn read<P: AsRef<str>>(&self, file_path: P) -> io::Result<ResReadFile> {

        let file_path = file_path.as_ref();
        if file_path.starts_with('/') || file_path.ends_with('/') {
            return Err(io::ErrorKind::NotFound.into());
        }

        let native_file_path = self.shared.dir_path.join(file_path);
        if native_file_path.is_file() {
            match File::open(native_file_path) {
                Ok(file) => return Ok(ResReadFile(ReadFileInner::Native(file))),
                Err(_) => (), // For now we skip this.
            }
        }

        self.shared.mutable.lock().unwrap()
            .read(file_path)
            .map(|reader| ResReadFile(ReadFileInner::Package(reader)))

    }

    /// Read a directory's entries in the resource filesystem. This function may be 
    /// blocking a short time because it needs to find the first node of that directory.
    /// 
    /// This function may return a file not found error if no package contains this 
    /// directory.
    pub fn read_dir<P: AsRef<str>>(&self, dir_path: P) -> io::Result<ResReadDir> {

        // Instant error if leading separator.
        let dir_path = dir_path.as_ref();
        if dir_path.starts_with('/') || dir_path.starts_with('.') {
            return Err(io::ErrorKind::NotFound.into());
        }

        // Remove an possible trailing separator.
        let dir_path = dir_path.strip_suffix('/').unwrap_or(dir_path);

        let native_dir_path = self.shared.dir_path.join(dir_path);
        let native_read_dir = fs::read_dir(native_dir_path).ok();
        
        let mut mutable = self.shared.mutable.lock().unwrap();
        let mut dir_index = None;

        // Initially we want to know the cache node index, if not found we try to open
        // and index the next pending package.
        while dir_index.is_none() {
            if let Some((find_dir_index, _)) = mutable.node_cache.find_dir(dir_path) {
                dir_index = Some(find_dir_index);
            } else if !mutable.try_open_pending_package() {
                // No package contains this directory, only error if native read dir 
                // also returned an error.
                if native_read_dir.is_none() {
                    return Err(io::ErrorKind::NotFound.into()); 
                } else {
                    break;
                }
            }
        }

        Ok(ResReadDir {
            dir_path: Arc::from(dir_path),
            common: Box::new(CommonReadDir {
                native_read_dir,
                package_read_dir: dir_index.map(|dir_index| PackageReadDir {
                    shared: Arc::clone(&self.shared),
                    dir_index,
                    native_names: HashSet::new(),
                    remaining_names: Vec::new(),
                    last_children_count: 0,
                    last_children_last_node_index: 0,
                }),
            }),
        })
    }

}

impl SharedMut {

    fn try_read(&mut self, file_path: &str) -> io::Result<Option<PackageFileReader<File>>> {
        
        if let Some((_, file_info)) = self.node_cache.find_file(file_path) {
            
            let (
                package_path, 
                package_reader,
            ) = self.package_reader_cache.get_index_mut(file_info.package_index).unwrap();
            let mut file_reader = package_reader.read_by_index(file_info.file_index)?;

            // Now that we have the reader, we want to make it owned, to do that we clone
            // it with a new handle to the underlying package file.
            return file_reader.try_clone_with(File::open(package_path)?).map(Some);

        } else {
            Ok(None)
        }

    }

    /// Open the next pending package and index it into the cache. This returns true if a
    /// pending package have been opened and cached, false if there are no more package.
    /// 
    /// An error is returned if the package could not be opened, this error is not 
    /// critical in itself but the pending package will never be opened again.
    /// 
    /// Errors considered critical are ones that happen on already opened packages.
    fn try_open_pending_package(&mut self) -> bool {

        while let Some(package_path) = self.pending_package_path.pop() {

            let package_file = match File::open(&package_path) {
                Ok(file) => file,
                Err(e) => {
                    self.package_open_errors.push((package_path, e));
                    continue;
                }
            };

            let package_reader = match PackageReader::new(package_file) {
                Ok(reader) => reader,
                Err(e) => {
                    self.package_open_errors.push((package_path, e));
                    continue;
                }
            };

            let (
                package_index, 
                prev_package,
            ) = self.package_reader_cache.insert_full(package_path, package_reader);
            debug_assert!(prev_package.is_none(), "duplicate package reader");
            
            self.node_cache.index_package(package_index, &self.package_reader_cache[package_index]);
            // println!("  cache size: {}", self.node_cache.nodes.len());
            // println!("  dir count: {}", self.node_cache.dir_count);
            // println!("  dir children max count: {}", self.node_cache.dir_children_max_count);
            // println!("  node name max len: {}", self.node_cache.node_name_max_len);

            return true;


        }

        false

    }

    /// See [`ResFilesystem::read()`].
    fn read(&mut self, file_path: &str) -> io::Result<PackageFileReader<File>> {

        loop {

            if let Some(file_reader) = self.try_read(file_path)? {
                return Ok(file_reader);
            }

            if !self.try_open_pending_package() {
                return Err(io::ErrorKind::NotFound.into());
            }

        }

    }

    /// See [`ResFilesystem::stat()`].
    fn stat(&mut self, node_path: &str) -> io::Result<ResStat> {

        loop {

            if let Some((_node_index, node_info)) = self.node_cache.find_node(node_path) {
                // debug_assert!(node_index < u32::MAX as usize, "too much nodes");
                return Ok(ResStat {
                    is_dir: node_info.as_dir().is_some(),
                    size: if let Some(file_info) = node_info.as_file() {
                        self.package_reader_cache[file_info.package_index]
                            .info_by_index(file_info.file_index)
                            .unwrap()
                            .size as u64
                    } else { 0 },
                    // index: node_index as u64,
                })
            }

            if !self.try_open_pending_package() {
                return Err(io::ErrorKind::NotFound.into());
            }
    
        }

        
    }

}


/// A handle to reading a resource file, this abstraction hides the underlying file but
/// it can be either a package file or a native file.
#[derive(Debug)]
pub struct ResReadFile(ReadFileInner);

/// Inner handle to
#[derive(Debug)]
enum ReadFileInner {
    Package(PackageFileReader<File>),
    Native(File),
}

impl Read for ResReadFile {

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.0 {
            ReadFileInner::Package(package) => package.read(buf),
            ReadFileInner::Native(file) => file.read(buf),
        }
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        match &mut self.0 {
            ReadFileInner::Package(package) => package.read_exact(buf),
            ReadFileInner::Native(file) => file.read_exact(buf),
        }
    }

}

impl Seek for ResReadFile {

    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match &mut self.0 {
            ReadFileInner::Package(package) => package.seek(pos),
            ReadFileInner::Native(file) => file.seek(pos),
        }
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        match &mut self.0 {
            ReadFileInner::Package(package) => package.stream_position(),
            ReadFileInner::Native(file) => file.stream_position(),
        }
    }

}


/// A directory read iterator that lazily open packages as iteration advance.
/// 
/// IMPL NOTE: This structure is quite heavy, it may be necessary to box its inner state.
#[derive(Debug)]
pub struct ResReadDir {
    /// Directory path that we are listing. It has no trailing separator!
    dir_path: Arc<str>,
    common: Box<CommonReadDir>,
}

#[derive(Debug)]
struct CommonReadDir {
    /// The native read dir result that maybe used for iteration before the package part.
    native_read_dir: Option<ReadDir>,
    /// The package read dir mode, yielded after the native read dir if present.
    package_read_dir: Option<PackageReadDir>,
}

#[derive(Debug)]
struct PackageReadDir {
    /// Shared resource filesystem data.
    shared: Arc<Shared>,
    /// Directory index in the node cache.
    dir_index: usize,
    /// If a native read dir is being used, then this contains names that should not be
    /// duplicated when returned.
    native_names: HashSet<Arc<str>>,
    /// A vector containing all names to return on next iterations. Name is associated to
    /// the node index in the cache, this
    remaining_names: Vec<(Arc<str>, usize)>,
    /// Total names count to return.
    last_children_count: usize,
    /// This keep the last (exclusive) node index used by children.
    last_children_last_node_index: usize,
}

impl ResReadDir {

    /// Get the path to the directory being read.
    #[inline]
    pub fn path(&self) -> &str {
        &self.dir_path
    }

}

impl Iterator for ResReadDir {

    type Item = io::Result<ResDirEntry>;

    fn next(&mut self) -> Option<Self::Item> {

        if let Some(native_read_dir) = &mut self.common.native_read_dir {
            match native_read_dir.next() {
                Some(Ok(entry)) => {
                    
                    let file_name = entry.file_name();
                    let metadata = match entry.metadata() {
                        Ok(res) => res,
                        Err(e) => return Some(Err(e)),
                    };

                    let file_name = match file_name.to_str() {
                        Some(res) => res,
                        None => return Some(Err(io::ErrorKind::InvalidData.into())),
                    };

                    let name = Arc::<str>::from(file_name);
                    if let Some(package_read_dir) = &mut self.common.package_read_dir {
                        package_read_dir.native_names.insert(Arc::clone(&name));
                    }

                    return Some(Ok(ResDirEntry { 
                        dir_path: Arc::clone(&self.dir_path), 
                        name,
                        stat: ResStat {
                            is_dir: metadata.is_dir(),
                            size: if metadata.is_dir() { 0 } else { metadata.len() },
                        },
                    }))

                },
                Some(Err(e)) => return Some(Err(e)),
                None => self.common.native_read_dir = None,
            }
        }

        if let Some(package_read_dir) = &mut self.common.package_read_dir {

            // Then we search the directory iteratively, and loop over if a pending package
            // has been opened.
            let mut mutable = package_read_dir.shared.mutable.lock().unwrap();

            loop {
                    
                let dir_info = mutable.node_cache.get_dir(package_read_dir.dir_index).unwrap();

                // If the directory info has been updated since the last iteration, we need to 
                // update remaining names. We need to do this kind of detection because we don't
                // exclusively own the filesystem and other read/read_dir may have altered cache.
                if dir_info.children.len() != package_read_dir.last_children_count {

                    debug_assert!(dir_info.children.len() > package_read_dir.last_children_count);

                    let mut max_child_index = 0;
                    for (child_name, &child_index) in &dir_info.children {
                        max_child_index = max_child_index.max(child_index);
                        if child_index >= package_read_dir.last_children_last_node_index {
                            // Don't return names that already have been by native iter.
                            if !package_read_dir.native_names.contains(child_name) {
                                package_read_dir.remaining_names.push((Arc::clone(child_name), child_index));
                            }
                        }
                    }

                    package_read_dir.last_children_count = dir_info.children.len();
                    package_read_dir.last_children_last_node_index = max_child_index + 1;

                }

                if let Some((node_name, node_index)) = package_read_dir.remaining_names.pop() {

                    let node_info = mutable.node_cache.get_node(node_index).unwrap();

                    return Some(Ok(ResDirEntry {
                        dir_path: Arc::clone(&self.dir_path),
                        name: node_name,
                        stat: ResStat {
                            is_dir: node_info.as_dir().is_some(),
                            size: match node_info {
                                NodeInfo::File(file) => {
                                    mutable.package_reader_cache[file.package_index]
                                        .info_by_index(file.file_index)
                                        .unwrap()
                                        .size as u64
                                }
                                NodeInfo::Dir(_) => 0,
                            },
                            // index: node_index as u64,
                        },
                    }));

                }

                // If there are no more file, we try opening more packages.
                if !mutable.try_open_pending_package() {
                    return None; // No more package to open, no more file to return.
                }

            }

        }

        None

    }

}

/// Represent an file or directory entry returned by [`ResReadDir`].
#[derive(Debug)]
pub struct ResDirEntry {
    dir_path: Arc<str>,
    name: Arc<str>,
    stat: ResStat,
}

impl ResDirEntry {

    /// Return the entry name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Reconstruct the full path of the entry for later [`ResFilesystem::read()`] call.
    pub fn path(&self) -> String {
        if self.dir_path.is_empty() {
            self.name.to_string()
        } else {
            format!("{}/{}", self.dir_path, self.name)
        }
    }

    /// Get stat of this entry, embedded within this directory entry structure.
    #[inline]
    pub fn stat(&self) -> &ResStat {
        &self.stat
    }

}

/// Various informations about a file, wether it's a directory or a file and its size on
/// disk (not compressed, package file are not compressed anyway...).
#[derive(Debug)]
pub struct ResStat {
    is_dir: bool,
    size: u64,
    // /// When the node is "native", we shift-left its index by 32 bit and set all low 32
    // /// bits to 1, this implies that "packaged" nodes only have 32 bits (minus 1) to be 
    // /// represented, which is largely enough!
    // index: u64,
}

impl ResStat {

    /// Return true if this entry is a directory.
    #[inline]
    pub fn is_dir(&self) -> bool {
        self.is_dir
    }

    /// Return true if this entry is a file.
    #[inline]
    pub fn is_file(&self) -> bool {
        !self.is_dir
    }

    /// Return the size of this file, this is zero for directories.
    #[inline]
    pub fn size(&self) -> u64 {
        self.size
    }

    // /// Debug-purpose file index within this whole filesystem, unique to each file.
    // #[inline]
    // pub fn index(&self) -> u64 {
    //     self.index
    // }

}


/// The node cache structure.
struct NodeCache {
    /// Inner file informations tree.
    nodes: Vec<NodeInfo>,
    /// Number of directories in all nodes.
    dir_count: usize,
    /// Just for stats.
    dir_children_max_count: usize,
    /// Just for stats.
    node_name_max_len: usize,
}

/// Kind of cached node information, absent, file or directory node.
#[derive(Debug)]
enum NodeInfo {
    // Information about a file.
    File(FileInfo),
    // Information about a directory.
    Dir(DirInfo)
}

#[derive(Debug)]
struct FileInfo {
    // Index of the package that contains the file.
    package_index: usize,
    // Index of the file within the package.
    file_index: usize,
}

#[derive(Debug, Default)]
struct DirInfo {
    /// Children of the directory. Key is a shared string because we clone it when 
    /// iterating directory entries. If this is altered because of a package indexing,
    /// the added children are guaranteed to have a node index that is greater than
    /// any previous one.
    children: BTreeMap<Arc<str>, usize>,
}

impl NodeCache {

    /// Create a new default file cache.
    fn new() -> Self {
        Self {
            nodes: vec![NodeInfo::Dir(DirInfo::default())],
            dir_count: 0,
            dir_children_max_count: 0,
            node_name_max_len: 0,
        }
    }

    /// Index a package in this node cache, note that the caller should avoid calling 
    /// this twice for the same packages.
    fn index_package(&mut self, package_index: usize, package_reader: &PackageReader<File>) {

        let mut last_dir_index = 0;
        let mut last_dir_path = ""; // This contains the end slash when relevant.

        for (file_index, file_info) in package_reader.infos().enumerate() {
            
            let file_name = file_info.name;

            // Always split the file name from the rest of the directory path.
            // NOTE: It is valid to split at 'index == file_path.len()', in this
            // case the 'file_name' will be empty, but this should not happen!
            // Also, 'dir_path' should not start with a sep.
            let (mut dir_path, file_name) = match file_name.rfind('/') {
                Some(last_sep_index) => file_name.split_at(last_sep_index + 1),
                None => ("", file_name),
            };

            debug_assert!(!file_name.is_empty(), "package names should only contains files");

            self.node_name_max_len = self.node_name_max_len.max(file_name.len());

            // If the file don't start with the last dir path, then we can reset index
            // to zero and try re-fetching all the path. If it starts with, then we just
            // shorten the path.
            let mut current_dir_index;
            if dir_path.starts_with(last_dir_path) {
                dir_path = &dir_path[last_dir_path.len()..];
                current_dir_index = last_dir_index;
            } else {
                current_dir_index = 0;
            }

            // If dir path isn't empty, it must contain at least a slash at the end, and
            // we discard it before splitting because we don't want to have a trailing
            // empty 'dir_part'.
            if !dir_path.is_empty() {
                for dir_part in dir_path[..dir_path.len() - 1].split('/') {

                    self.node_name_max_len = self.node_name_max_len.max(dir_part.len());

                    // NOTE: Need to store the inner length here, we use it after to 
                    // avoid borrowing issues.
                    let inner_len = self.nodes.len();
                    let dir = self.nodes[current_dir_index]
                        .as_dir_mut()
                        .expect("trying to make a directory where a file already exists");
                    
                    if let Some(&child_index) = dir.children.get(dir_part) {
                        current_dir_index = child_index;
                    } else {
                        current_dir_index = inner_len;
                        dir.children.insert(Arc::from(dir_part), inner_len);
                        self.dir_children_max_count = self.dir_children_max_count.max(dir.children.len());
                        self.nodes.push(NodeInfo::Dir(DirInfo::default()));
                        self.dir_count += 1;
                    }

                }
            }

            if last_dir_index != current_dir_index {
                last_dir_index = current_dir_index;
                last_dir_path = dir_path;
            }

            // NOTE: Same as above!
            let inner_len = self.nodes.len();
            let dir = self.nodes[current_dir_index]
                .as_dir_mut()
                .expect("current directory should effectively be a directory");

            let prev_child = dir.children.insert(Arc::from(file_name), inner_len);
            self.dir_children_max_count = self.dir_children_max_count.max(dir.children.len());
            debug_assert!(prev_child.is_none(), "overwriting a file");
            self.nodes.push(NodeInfo::File(FileInfo {
                package_index,
                file_index,
            }));

        }

    }

    /// Find a node info in cache from the given path. In general it should not have 
    /// leading nor trailing directory separator. The index of the node within internal
    /// nodes array is already returned.
    fn find_node(&self, node_path: &str) -> Option<(usize, &NodeInfo)> {

        let mut current_node_index = 0;
        if !node_path.is_empty() {
            for node_part in node_path.split('/') {
                current_node_index = *self.nodes[current_node_index]
                    .as_dir()?
                    .children
                    .get(node_part)?;
            }
        }

        Some((current_node_index, &self.nodes[current_node_index]))
        
    }

    /// Same as [`Self::find_node()`] but returns some only if it's a directory.
    fn find_dir(&self, dir_path: &str) -> Option<(usize, &DirInfo)> {
        self.find_node(dir_path).and_then(|(index, info)| 
            info.as_dir().map(|info| (index, info)))
    }

    /// Same as [`Self::find_node()`] but returns some only if it's a file.
    fn find_file(&self, file_path: &str) -> Option<(usize, &FileInfo)> {
        self.find_node(file_path).and_then(|(index, info)| 
            info.as_file().map(|info| (index, info)))
    }

    /// Get a node information from its index.
    fn get_node(&self, index: usize) -> Option<&NodeInfo> {
        self.nodes.get(index)
    }

    /// Get a directory information from its node index (see [`Self::find_dir`]).
    fn get_dir(&self, index: usize) -> Option<&DirInfo> {
        self.get_node(index)?.as_dir()
    }

    /// Get a file information from its node index (see [`Self::find_file`]).
    #[allow(unused)]
    fn get_file(&self, index: usize) -> Option<&FileInfo> {
        self.get_node(index)?.as_file()
    }

}

impl NodeInfo {

    #[inline]
    fn as_file(&self) -> Option<&FileInfo> {
        match self {
            NodeInfo::File(file) => Some(file),
            NodeInfo::Dir(_) => None,
        }
    }

    #[inline]
    fn as_dir(&self) -> Option<&DirInfo> {
        match self {
            NodeInfo::File(_) => None,
            NodeInfo::Dir(dir) => Some(dir),
        }
    }

    // #[inline]
    // fn as_file_mut(&mut self) -> Option<&mut FileInfo> {
    //     match self {
    //         NodeInfo::File(file) => Some(file),
    //         NodeInfo::Dir(_) => None,
    //     }
    // }

    #[inline]
    fn as_dir_mut(&mut self) -> Option<&mut DirInfo> {
        match self {
            NodeInfo::File(_) => None,
            NodeInfo::Dir(dir) => Some(dir),
        }
    }

}

impl fmt::Debug for NodeCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeCache")
            .field("nodes_len", &self.nodes.len())
            .field("dir_count", &self.dir_count)
            .field("dir_children_max_count", &self.dir_children_max_count)
            .field("node_name_max_len", &self.node_name_max_len)
            .finish()
    }
}
