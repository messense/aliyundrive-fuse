//! FUSE adaptor
//!
//! https://github.com/gz/btfs is used as a reference.
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::time::UNIX_EPOCH;
use std::{collections::BTreeMap, time::Duration};

use bytes::Bytes;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyOpen, Request, FUSE_ROOT_ID,
};
use tracing::debug;

use crate::drive::{AliyunDrive, AliyunFile};
use crate::error::Error;
use crate::file_cache::FileCache;

const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u64 = 4194304;

#[derive(Debug, Clone)]
pub struct Inode {
    children: BTreeMap<OsString, u64>,
    parent: u64,
}

impl Inode {
    fn new(parent: u64) -> Self {
        Self {
            children: BTreeMap::new(),
            parent,
        }
    }

    fn add_child(&mut self, name: OsString, inode: u64) {
        self.children.insert(name, inode);
    }
}

pub struct AliyunDriveFileSystem {
    drive: AliyunDrive,
    file_cache: FileCache,
    files: BTreeMap<u64, AliyunFile>,
    inodes: BTreeMap<u64, Inode>,
    next_inode: u64,
    next_fh: u64,
}

impl AliyunDriveFileSystem {
    pub fn new(drive: AliyunDrive, read_buffer_size: usize) -> Self {
        let file_cache = FileCache::new(drive.clone(), read_buffer_size);
        Self {
            drive,
            file_cache,
            files: BTreeMap::new(),
            inodes: BTreeMap::new(),
            next_inode: 1,
            next_fh: 2,
        }
    }

    /// Next inode number
    fn next_inode(&mut self) -> u64 {
        self.next_inode = self.next_inode.wrapping_add(1);
        self.next_inode
    }

    /// Next file handler
    fn next_fh(&mut self) -> u64 {
        self.next_fh = self.next_fh.wrapping_add(1);
        self.next_fh
    }

    fn init(&mut self) -> Result<(), Error> {
        let mut root_file = AliyunFile::new_root();
        let (used_size, _) = self.drive.get_quota().map_err(|_| Error::ApiCallFailed)?;
        root_file.size = used_size;
        let root_inode = Inode::new(0);
        self.inodes.insert(FUSE_ROOT_ID, root_inode);
        self.files.insert(FUSE_ROOT_ID, root_file);
        Ok(())
    }

    fn lookup(&mut self, parent: u64, name: &OsStr) -> Result<FileAttr, Error> {
        let mut parent_inode = self
            .inodes
            .get(&parent)
            .ok_or(Error::ParentNotFound)?
            .clone();
        if parent_inode.children.is_empty() {
            // Parent inode isn't loaded yet
            debug!(parent = parent, "readdir missing parent in lookup");
            self.readdir(parent, 0)?;
            parent_inode = self
                .inodes
                .get(&parent)
                .ok_or(Error::ParentNotFound)?
                .clone();
        }
        let inode = parent_inode
            .children
            .get(name)
            .ok_or(Error::ChildNotFound)?;
        let file = self.files.get(inode).ok_or(Error::NoEntry)?;
        Ok(file.to_file_attr(*inode))
    }

    fn readdir(&mut self, ino: u64, offset: i64) -> Result<Vec<(u64, FileType, String)>, Error> {
        let mut entries = Vec::new();
        let mut inode = self.inodes.get(&ino).ok_or(Error::NoEntry)?.clone();

        if offset == 0 {
            entries.push((ino, FileType::Directory, ".".to_string()));
            entries.push((inode.parent, FileType::Directory, String::from("..")));

            let file = self.files.get(&ino).ok_or(Error::NoEntry)?;
            let parent_file_id = &file.id;
            let files = self
                .drive
                .list_all(parent_file_id)
                .map_err(|_| Error::ApiCallFailed)?;
            debug!(
                inode = ino,
                "total {} files in directory {}",
                files.len(),
                file.name
            );

            let mut to_remove = inode.children.keys().cloned().collect::<Vec<_>>();
            for file in &files {
                let name = OsString::from(file.name.clone());
                if inode.children.contains_key(&name) {
                    // file already exists
                    to_remove.retain(|n| n != &name);
                } else {
                    let new_inode = self.next_inode();
                    inode.add_child(name, new_inode);
                    self.files.insert(new_inode, file.clone());
                    self.inodes
                        .entry(new_inode)
                        .or_insert_with(|| Inode::new(ino));
                }
            }

            if !to_remove.is_empty() {
                for name in to_remove {
                    if let Some(ino_remove) = inode.children.remove(&name) {
                        debug!(inode = ino_remove, name = %Path::new(&name).display(), "remove outdated inode");
                        self.files.remove(&ino_remove);
                        self.inodes.remove(&ino_remove);
                    }
                }
            }
            self.inodes.insert(ino, inode.clone());
        }

        for child_ino in inode.children.values().skip(offset as usize) {
            let file = self.files.get(child_ino).ok_or(Error::ChildNotFound)?;
            entries.push((*child_ino, file.r#type.into(), file.name.clone()));
        }
        Ok(entries)
    }

    fn read(&mut self, ino: u64, fh: u64, offset: i64, size: u32) -> Result<Bytes, Error> {
        let file = self.files.get(&ino).ok_or(Error::NoEntry)?;
        debug!(inode = ino, name = %file.name, fh = fh, offset = offset, size = size, "read");
        if offset >= file.size as i64 {
            return Ok(Bytes::new());
        }
        let size = std::cmp::min(size, file.size.saturating_sub(offset as u64) as u32);
        self.file_cache.read(fh, offset, size)
    }
}

impl Filesystem for AliyunDriveFileSystem {
    fn init(
        &mut self,
        _req: &Request<'_>,
        _config: &mut fuser::KernelConfig,
    ) -> Result<(), libc::c_int> {
        if let Err(e) = self.init() {
            return Err(e.into());
        }
        Ok(())
    }

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let dirname = Path::new(name);
        debug!(parent = parent, name = %dirname.display(), "lookup");
        match self.lookup(parent, name) {
            Ok(attr) => reply.entry(&TTL, &attr, 0),
            Err(e) => reply.error(e.into()),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        if let Some(file) = self.files.get(&ino) {
            debug!(inode = ino, name = %file.name, "getattr");
            reply.attr(&TTL, &file.to_file_attr(ino))
        } else {
            debug!(inode = ino, "getattr");
            reply.error(libc::ENOENT);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!(inode = ino, offset = offset, "readdir");
        match self.readdir(ino, offset) {
            Ok(entries) => {
                // Offset of 0 means no offset.
                // Non-zero offset means the passed offset has already been seen,
                // and we should start after it.
                let offset_add = if offset == 0 { 0 } else { offset + 1 };
                for (i, (ino, kind, name)) in entries.into_iter().enumerate() {
                    let buffer_full = reply.add(ino, offset_add + i as i64, kind, name);
                    if buffer_full {
                        break;
                    }
                }
                reply.ok();
            }
            Err(e) => reply.error(e.into()),
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
        if let Some((file_id, file_name, file_size)) = self
            .files
            .get(&ino)
            .map(|f| (f.id.clone(), f.name.clone(), f.size))
        {
            debug!(inode = ino, name = %file_name, "open file");
            let fh = self.next_fh();
            self.file_cache.open(fh, file_id, file_size);
            reply.opened(fh, 0);
        } else {
            debug!(inode = ino, "open file");
            reply.error(libc::ENOENT);
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        debug!(inode = ino, fh = fh, "release file");
        self.file_cache.release(fh);
        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        match self.read(ino, fh, offset, size) {
            Ok(data) => reply.data(&data),
            Err(e) => reply.error(e.into()),
        }
    }
}

impl From<crate::drive::FileType> for FileType {
    fn from(typ: crate::drive::FileType) -> Self {
        use crate::drive::FileType as AliyunFileType;

        match typ {
            AliyunFileType::Folder => FileType::Directory,
            AliyunFileType::File => FileType::RegularFile,
        }
    }
}

impl AliyunFile {
    fn to_file_attr(&self, ino: u64) -> FileAttr {
        let kind = self.r#type.into();
        let perm = if matches!(kind, FileType::Directory) {
            0o755
        } else {
            0o644
        };
        let nlink = if ino == FUSE_ROOT_ID { 2 } else { 1 };
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        let blksize = BLOCK_SIZE;
        let blocks = self.size / blksize + 1;
        FileAttr {
            ino,
            size: self.size,
            blocks,
            atime: UNIX_EPOCH,
            mtime: *self.updated_at,
            ctime: *self.created_at,
            crtime: *self.created_at,
            kind,
            perm,
            nlink,
            uid,
            gid,
            rdev: 0,
            blksize: blksize as u32,
            flags: 0,
        }
    }
}
