//! FUSE adaptor
//!
//! https://github.com/gz/btfs is used as a reference.
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::time::UNIX_EPOCH;
use std::{collections::BTreeMap, time::Duration};

use fuser::consts::FOPEN_DIRECT_IO;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
    Request, FUSE_ROOT_ID,
};
use tracing::debug;

use crate::drive::{AliyunDrive, AliyunFile};

const TTL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct Inode {
    name: OsString,
    children: BTreeMap<OsString, u64>,
    parent: u64,
}

impl Inode {
    fn new(name: OsString, parent: u64) -> Self {
        Self {
            name,
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
}

impl Filesystem for AliyunDriveFileSystem {
    fn init(
        &mut self,
        _req: &Request<'_>,
        _config: &mut fuser::KernelConfig,
    ) -> Result<(), libc::c_int> {
        let root_file = AliyunFile::new_root();
        let root_inode = Inode::new(OsString::from(root_file.name.clone()), FUSE_ROOT_ID);
        self.inodes.insert(FUSE_ROOT_ID, root_inode);
        self.files.insert(FUSE_ROOT_ID, root_file);
        Ok(())
    }

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let dirname = Path::new(name);
        debug!(parent = parent, name = %dirname.display(), "lookup");
        let parent_inode = self.inodes.get(&parent).unwrap();
        let inode = parent_inode.children.get(name).unwrap();
        let file = self.files.get(&inode).unwrap();
        reply.entry(&TTL, &file.to_file_attr(*inode), 0);
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        debug!(inode = ino, "getattr");
        if let Some(file) = self.files.get(&ino) {
            reply.attr(&TTL, &file.to_file_attr(ino))
        } else {
            reply.error(libc::ENOENT);
        }
    }

    fn open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        reply.opened(0, FOPEN_DIRECT_IO);
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
        let mut inode = self.inodes.get(&ino).unwrap().clone();
        let file = self.files.get(&ino).unwrap();
        let parent_file_id = &file.id;
        if offset == 0 {
            let files = self.drive.list_all(parent_file_id).unwrap();
            for file in &files {
                let new_inode = self.next_inode();
                inode.add_child(OsString::from(file.name.clone()), new_inode);
                self.files.insert(new_inode, file.clone());
                self.inodes
                    .entry(new_inode)
                    .or_insert_with(|| Inode::new(OsString::from(file.name.clone()), ino));
            }
            self.inodes.insert(ino, inode);
            for (index, file) in files.iter().skip(offset as usize).enumerate() {
                let buffer_full = reply.add(
                    ino,
                    offset + index as i64 + 1,
                    file.r#type.into(),
                    &file.name,
                );
                if buffer_full {
                    break;
                }
            }
        }
        reply.ok();
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
        let file = self.files.get(&ino).unwrap();
        if offset >= file.size as i64 {
            reply.data(&[]);
            return;
        }
        let download_url = self.drive.get_download_url(&file.id).unwrap();
        let size = std::cmp::min(size, file.size.saturating_sub(offset as u64) as u32);
        let data = self
            .drive
            .download(&download_url, offset as _, size as _)
            .unwrap();
        reply.data(&data);
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
        FileAttr {
            ino,
            size: self.size,
            blocks: 0,
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
            blksize: 512,
            flags: 0,
        }
    }
}
