//! FUSE adaptor
//!
//! https://github.com/gz/btfs is used as a reference.
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::time::UNIX_EPOCH;
use std::{collections::BTreeMap, time::Duration};

use bytes::Bytes;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
    FUSE_ROOT_ID,
};
use tracing::debug;

use crate::drive::{AliyunDrive, AliyunFile};

const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub enum Error {
    NoEntry,
    ParentNotFound,
    ChildNotFound,
    ApiCallFailed,
}

impl From<Error> for libc::c_int {
    fn from(e: Error) -> Self {
        match e {
            Error::NoEntry => libc::ENOENT,
            Error::ParentNotFound => libc::ENOENT,
            Error::ChildNotFound => libc::ENOENT,
            Error::ApiCallFailed => libc::EIO,
        }
    }
}

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
    files: BTreeMap<u64, AliyunFile>,
    inodes: BTreeMap<u64, Inode>,
    next_inode: u64,
}

impl AliyunDriveFileSystem {
    pub fn new(drive: AliyunDrive) -> Self {
        Self {
            drive,
            files: BTreeMap::new(),
            inodes: BTreeMap::new(),
            next_inode: 1,
        }
    }

    fn next_inode(&mut self) -> u64 {
        self.next_inode += 1;
        self.next_inode
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
        let parent_inode = self.inodes.get(&parent).ok_or(Error::ParentNotFound)?;
        let inode = parent_inode
            .children
            .get(name)
            .ok_or(Error::ChildNotFound)?;
        let file = self.files.get(inode).ok_or(Error::NoEntry)?;
        Ok(file.to_file_attr(*inode))
    }

    fn readdir(&mut self, ino: u64) -> Result<Vec<(u64, FileType, String)>, Error> {
        let mut entries = vec![(ino, FileType::Directory, ".".to_string())];

        let mut inode = self.inodes.get(&ino).ok_or(Error::NoEntry)?.clone();
        entries.push((inode.parent, FileType::Directory, String::from("..")));

        let file = self.files.get(&ino).ok_or(Error::NoEntry)?;
        let parent_file_id = &file.id;
        let files = self
            .drive
            .list_all(parent_file_id)
            .map_err(|_| Error::ApiCallFailed)?;
        for file in &files {
            let new_inode = self.next_inode();
            inode.add_child(OsString::from(file.name.clone()), new_inode);
            self.files.insert(new_inode, file.clone());
            self.inodes
                .entry(new_inode)
                .or_insert_with(|| Inode::new(ino));
            entries.push((new_inode, file.r#type.into(), file.name.clone()));
        }
        self.inodes.insert(ino, inode);
        Ok(entries)
    }

    fn read(&mut self, ino: u64, offset: i64, size: u32) -> Result<Bytes, Error> {
        let file = self.files.get(&ino).ok_or(Error::NoEntry)?;
        if offset >= file.size as i64 {
            return Ok(Bytes::new());
        }
        let download_url = self
            .drive
            .get_download_url(&file.id)
            .map_err(|_| Error::ApiCallFailed)?;
        let size = std::cmp::min(size, file.size.saturating_sub(offset as u64) as u32);
        let data = self
            .drive
            .download(&download_url, offset as _, size as _)
            .map_err(|_| Error::ApiCallFailed)?;
        Ok(data)
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
        debug!(inode = ino, "getattr");
        if let Some(file) = self.files.get(&ino) {
            reply.attr(&TTL, &file.to_file_attr(ino))
        } else {
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
        match self.readdir(ino) {
            Ok(entries) => {
                // Offset of 0 means no offset.
                // Non-zero offset means the passed offset has already been seen,
                // and we should start after it.
                let to_skip = if offset == 0 { 0 } else { offset + 1 } as usize;
                for (i, (ino, kind, name)) in entries.into_iter().enumerate().skip(to_skip) {
                    let buffer_full = reply.add(ino, i as i64, kind, name);
                    if buffer_full {
                        break;
                    }
                }
                reply.ok();
            }
            Err(e) => reply.error(e.into()),
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        debug!(inode = ino, offset = offset, size = size, "read");
        match self.read(ino, offset, size) {
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
