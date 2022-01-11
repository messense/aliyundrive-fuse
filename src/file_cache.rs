use std::collections::BTreeMap;

use bytes::Bytes;
use tracing::debug;

use crate::error::Error;
use crate::AliyunDrive;

#[derive(Debug)]
struct CachedFile {
    file_id: String,
    file_size: u64,
    start_pos: i64,
    buffer: Bytes,
}

#[derive(Debug)]
pub struct FileCache {
    drive: AliyunDrive,
    read_buffer_size: usize,
    // file handle -> cached file
    cache: BTreeMap<u64, CachedFile>,
}

impl FileCache {
    pub fn new(drive: AliyunDrive, read_buffer_size: usize) -> Self {
        Self {
            drive,
            read_buffer_size,
            cache: BTreeMap::new(),
        }
    }

    fn read_chunk(&self, file_id: &str, file_size: u64, offset: i64) -> Result<Bytes, Error> {
        let size = std::cmp::min(
            self.read_buffer_size,
            file_size.saturating_sub(offset as u64) as usize,
        );
        let download_url = self
            .drive
            .get_download_url(file_id)
            .map_err(|_| Error::ApiCallFailed)?;
        let data = self
            .drive
            .download(&download_url, offset as _, size)
            .map_err(|_| Error::ApiCallFailed)?;
        Ok(data)
    }

    pub fn read(&mut self, fh: u64, offset: i64, size: u32) -> Result<Bytes, Error> {
        let cached = self.cache.get(&fh).ok_or(Error::NoEntry)?;
        let start_pos = cached.start_pos;
        let end_pos = offset + i64::from(size);
        let buf_size = cached.buffer.len();
        debug!(
            fh = fh,
            offset = offset,
            size = size,
            buffer_start = start_pos,
            buffer_size = buf_size,
            "read file cache"
        );
        if offset >= start_pos && end_pos <= start_pos + buf_size as i64 {
            let buf_start = (offset - start_pos) as usize;
            let buf_end = buf_start + size as usize;
            let data = cached.buffer.slice(buf_start..buf_end);
            return Ok(data);
        }
        let chunk = self.read_chunk(&cached.file_id, cached.file_size, offset)?;
        let new_cached = CachedFile {
            file_id: cached.file_id.clone(),
            file_size: cached.file_size,
            start_pos: offset,
            buffer: chunk.clone(),
        };
        self.cache.insert(fh, new_cached);

        // chunk size maybe less than size
        let size = if chunk.len() >= size as usize {
            size as usize
        } else {
            chunk.len()
        };
        Ok(chunk.slice(..size as usize))
    }

    pub fn open(&mut self, fh: u64, file_id: String, file_size: u64) {
        let file = CachedFile {
            file_id,
            file_size,
            start_pos: 0,
            buffer: Bytes::new(),
        };
        self.cache.insert(fh, file);
    }

    pub fn release(&mut self, fh: u64) {
        self.cache.remove(&fh);
    }
}
