use anyhow::{Context, Result};
use fuser::{FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request};
use libc::ENOENT;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use tracing::{debug, error, warn};

use crate::config::Config;
use crate::crypto::{Encryptor, load_key};
use crate::s3_client::S3Storage;
use crate::storage::StorageBackend;

const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u64 = 4096;
const ROOT_INO: u64 = 1;

pub struct AegisFS {
    storage: Arc<S3Storage>,
    encryptor: Arc<Encryptor>,
    inode_map: Arc<Mutex<HashMap<u64, String>>>, // ino -> path
    path_map: Arc<Mutex<HashMap<String, u64>>>,  // path -> ino
    next_ino: Arc<Mutex<u64>>,
}

impl AegisFS {
    pub async fn new(config: Config) -> Result<Self> {
        let key = load_key(&config.encryption.key_file)
            .context("Failed to load encryption key")?;
        let encryptor = Arc::new(Encryptor::new(key));
        
        let storage = Arc::new(S3Storage::new(config.s3).await?);
        
        let mut path_map = HashMap::new();
        path_map.insert("".to_string(), ROOT_INO);
        
        let mut inode_map = HashMap::new();
        inode_map.insert(ROOT_INO, "".to_string());
        
        Ok(Self {
            storage,
            encryptor,
            inode_map: Arc::new(Mutex::new(inode_map)),
            path_map: Arc::new(Mutex::new(path_map)),
            next_ino: Arc::new(Mutex::new(2)),
        })
    }
    
    fn get_or_create_ino(&self, path: &str) -> u64 {
        let mut path_map = self.path_map.lock().unwrap();
        let mut inode_map = self.inode_map.lock().unwrap();
        let mut next_ino = self.next_ino.lock().unwrap();
        
        if let Some(&ino) = path_map.get(path) {
            ino
        } else {
            let ino = *next_ino;
            *next_ino += 1;
            path_map.insert(path.to_string(), ino);
            inode_map.insert(ino, path.to_string());
            ino
        }
    }
    
    fn get_path(&self, ino: u64) -> Option<String> {
        if ino == ROOT_INO {
            return Some("".to_string());
        }
        let inode_map = self.inode_map.lock().unwrap();
        inode_map.get(&ino).cloned()
    }

    fn normalize_path(&self, path: &str) -> String {
        path.trim_start_matches('/').to_string()
    }

    async fn read_file(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let encrypted = match self.storage.get(path).await? {
            Some(data) => data,
            None => return Ok(None),
        };
        
        let decrypted = self.encryptor.decrypt(&encrypted)
            .context("Failed to decrypt file")?;
        
        Ok(Some(decrypted))
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> Result<()> {
        let encrypted = self.encryptor.encrypt(data)
            .context("Failed to encrypt file")?;
        
        self.storage.put(path, encrypted.into()).await?;
        Ok(())
    }

    async fn delete_file(&self, path: &str) -> Result<()> {
        self.storage.delete(path).await?;
        Ok(())
    }

    async fn list_directory(&self, path: &str) -> Result<Vec<String>> {
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{}/", path)
        };
        
        let mut entries = self.storage.list(&prefix).await?;
        
        // Filter to only direct children
        let mut children = Vec::new();
        let path_depth = if path.is_empty() { 0 } else { path.matches('/').count() + 1 };
        
        for entry in entries {
            let entry_depth = entry.matches('/').count() + 1;
            if entry_depth == path_depth + 1 {
                // Extract just the filename
                if let Some(filename) = entry.strip_prefix(&prefix) {
                    children.push(filename.to_string());
                }
            }
        }
        
        Ok(children)
    }
}

impl Filesystem for AegisFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        debug!("lookup: parent={}, name={}", parent, name_str);

        // Root directory
        if parent == 1 {
            if name_str == "." || name_str == ".." {
                reply.entry(&TTL, &self.get_root_attr(), 0);
                return;
            }
        }

        let parent_path = if parent == ROOT_INO {
            ""
        } else {
            match self.get_path(parent) {
                Some(p) => &p,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            }
        };
        
        let path = if parent_path.is_empty() {
            name_str.to_string()
        } else {
            format!("{}/{}", parent_path, name_str)
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        match rt.block_on(self.storage.exists(&path)) {
            Ok(true) => {
                let ino = self.get_or_create_ino(&path);
                let attr = rt.block_on(async {
                    self.get_file_attr_for_path(&path, ino).await
                });
                reply.entry(&TTL, &attr, 0);
            }
            Ok(false) => {
                // Check if it's a directory
                let dir_path = format!("{}/", path);
                match rt.block_on(self.storage.list(&dir_path)) {
                    Ok(entries) if !entries.is_empty() => {
                        let ino = self.get_or_create_ino(&path);
                        let attr = self.get_dir_attr_for_path(ino);
                        reply.entry(&TTL, &attr, 0);
                    }
                    _ => {
                        reply.error(ENOENT);
                    }
                }
            }
            Err(e) => {
                error!("Error checking file existence: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        debug!("getattr: ino={}", ino);

        if ino == ROOT_INO {
            reply.attr(&TTL, &self.get_root_attr());
            return;
        }
        
        let path = match self.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        
        let rt = tokio::runtime::Runtime::new().unwrap();
        let attr = rt.block_on(async {
            // Check if it's a directory
            let dir_path = format!("{}/", path);
            match self.storage.list(&dir_path).await {
                Ok(entries) if !entries.is_empty() => {
                    self.get_dir_attr_for_path(ino)
                }
                _ => {
                    // It's a file, get its size
                    match self.read_file(&path).await {
                        Ok(Some(data)) => {
                            let mut attr = self.get_file_attr_for_path(&path, ino).await;
                            attr.size = data.len() as u64;
                            attr
                        }
                        _ => self.get_file_attr_for_path(&path, ino).await,
                    }
                }
            }
        });
        
        reply.attr(&TTL, &attr);
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir: ino={}, offset={}", ino, offset);

        if ino != ROOT_INO {
            // Handle subdirectories
            let path = match self.get_path(ino) {
                Some(p) => p,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            };
            
            // Add . and ..
            if offset == 0 {
                if reply.add(ino, 0, FileType::Directory, ".") {
                    return;
                }
            }
            if offset <= 1 {
                let parent_ino = if path.contains('/') {
                    // Extract parent path and get its ino
                    if let Some(parent_path) = path.rsplitn(2, '/').nth(1) {
                        *self.path_map.lock().unwrap().get(parent_path).unwrap_or(&ROOT_INO)
                    } else {
                        ROOT_INO
                    }
                } else {
                    ROOT_INO
                };
                if reply.add(parent_ino, 1, FileType::Directory, "..") {
                    return;
                }
            }
            
            let rt = tokio::runtime::Runtime::new().unwrap();
            match rt.block_on(self.list_directory(&path)) {
                Ok(entries) => {
                    let mut current_offset = 2;
                    for entry in entries {
                        if current_offset > offset {
                            let entry_path = if path.is_empty() {
                                entry.clone()
                            } else {
                                format!("{}/{}", path, entry)
                            };
                            let entry_ino = self.get_or_create_ino(&entry_path);
                            if reply.add(entry_ino, current_offset, FileType::RegularFile, entry.as_str()) {
                                return;
                            }
                        }
                        current_offset += 1;
                    }
                    reply.ok();
                }
                Err(e) => {
                    error!("Error listing directory: {}", e);
                    reply.error(libc::EIO);
                }
            }
            return;
        }

        // Add . and ..
        if offset == 0 {
            if reply.add(1, 0, FileType::Directory, ".") {
                return;
            }
        }
        if offset <= 1 {
            if reply.add(1, 1, FileType::Directory, "..") {
                return;
            }
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        match rt.block_on(self.list_directory("")) {
            Ok(entries) => {
                let mut current_offset = 2;
                for entry in entries {
                    if current_offset > offset {
                        let ino = current_offset as u64;
                        if reply.add(ino, current_offset, FileType::RegularFile, entry.as_str()) {
                            return;
                        }
                    }
                    current_offset += 1;
                }
                reply.ok();
            }
            Err(e) => {
                error!("Error listing directory: {}", e);
                reply.error(libc::EIO);
            }
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
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read: ino={}, offset={}, size={}", ino, offset, size);

        if ino == ROOT_INO {
            reply.error(libc::EISDIR);
            return;
        }

        let path = match self.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        
        let rt = tokio::runtime::Runtime::new().unwrap();
        match rt.block_on(self.read_file(&path)) {
            Ok(Some(data)) => {
                let offset = offset as usize;
                if offset >= data.len() {
                    reply.data(&[]);
                    return;
                }
                
                let end = std::cmp::min(offset + size as usize, data.len());
                reply.data(&data[offset..end]);
            }
            Ok(None) => {
                reply.error(ENOENT);
            }
            Err(e) => {
                error!("Error reading file: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: fuser::ReplyWrite,
    ) {
        debug!("write: ino={}, offset={}, size={}", ino, offset, data.len());

        if ino == ROOT_INO {
            reply.error(libc::EISDIR);
            return;
        }

        let path = match self.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        
        // For simplicity, we'll do a read-modify-write
        // In production, implement proper file handle management
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut file_data = match rt.block_on(self.read_file(&path)) {
            Ok(Some(data)) => data,
            Ok(None) => Vec::new(),
            Err(e) => {
                error!("Error reading file for write: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        let offset = offset as usize;
        if offset + data.len() > file_data.len() {
            file_data.resize(offset + data.len(), 0);
        }
        file_data[offset..offset + data.len()].copy_from_slice(data);

        match rt.block_on(self.write_file(&path, &file_data)) {
            Ok(_) => {
                reply.written(data.len() as u32);
            }
            Err(e) => {
                error!("Error writing file: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _flags: i32,
        reply: ReplyEntry,
    ) {
        debug!("create: parent={}, name={:?}, mode={}", parent, name, mode);

        if parent != ROOT_INO {
            let parent_path = match self.get_path(parent) {
                Some(p) => p,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            };
            
            let name_str = match name.to_str() {
                Some(s) => s,
                None => {
                    reply.error(libc::EINVAL);
                    return;
                }
            };
            
            let path = if parent_path.is_empty() {
                name_str.to_string()
            } else {
                format!("{}/{}", parent_path, name_str)
            };
            
            let rt = tokio::runtime::Runtime::new().unwrap();
            
            // Create empty file
            match rt.block_on(self.write_file(&path, &[])) {
                Ok(_) => {
                    let ino = self.get_or_create_ino(&path);
                    let attr = self.get_file_attr_for_path(&path, ino).await;
                    reply.entry(&TTL, &attr, 0);
                }
                Err(e) => {
                    error!("Error creating file: {}", e);
                    reply.error(libc::EIO);
                }
            }
            return;
        }

        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        let path = name_str.to_string();
        let rt = tokio::runtime::Runtime::new().unwrap();
        
        // Create empty file
        match rt.block_on(self.write_file(&path, &[])) {
            Ok(_) => {
                let ino = self.get_or_create_ino(&path);
                let attr = rt.block_on(async { self.get_file_attr_for_path(&path, ino).await });
                reply.entry(&TTL, &attr, 0);
            }
            Err(e) => {
                error!("Error creating file: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        debug!("unlink: parent={}, name={:?}", parent, name);

        let parent_path = if parent == ROOT_INO {
            ""
        } else {
            match self.get_path(parent) {
                Some(p) => &p,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            }
        };

        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        let path = if parent_path.is_empty() {
            name_str.to_string()
        } else {
            format!("{}/{}", parent_path, name_str)
        };
        
        let rt = tokio::runtime::Runtime::new().unwrap();
        
        // Remove from maps
        if let Some(ino) = self.path_map.lock().unwrap().remove(&path) {
            self.inode_map.lock().unwrap().remove(&ino);
        }
        
        match rt.block_on(self.delete_file(&path)) {
            Ok(_) => {
                reply.ok();
            }
            Err(e) => {
                error!("Error deleting file: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        reply: ReplyEntry,
    ) {
        debug!("mkdir: parent={}, name={:?}", parent, name);

        let parent_path = if parent == ROOT_INO {
            ""
        } else {
            match self.get_path(parent) {
                Some(p) => &p,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            }
        };

        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        let path = if parent_path.is_empty() {
            name_str.to_string()
        } else {
            format!("{}/{}", parent_path, name_str)
        };

        // Create a marker file to represent the directory
        let marker_path = format!("{}/.dir", path);
        let rt = tokio::runtime::Runtime::new().unwrap();
        
        match rt.block_on(self.write_file(&marker_path, &[])) {
            Ok(_) => {
                let ino = self.get_or_create_ino(&path);
                let attr = self.get_dir_attr_for_path(ino);
                reply.entry(&TTL, &attr, 0);
            }
            Err(e) => {
                error!("Error creating directory: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn rmdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        debug!("rmdir: parent={}, name={:?}", parent, name);

        let parent_path = if parent == ROOT_INO {
            ""
        } else {
            match self.get_path(parent) {
                Some(p) => &p,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            }
        };

        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        let path = if parent_path.is_empty() {
            name_str.to_string()
        } else {
            format!("{}/{}", parent_path, name_str)
        };

        // Delete directory marker and all contents
        let dir_path = format!("{}/", path);
        let rt = tokio::runtime::Runtime::new().unwrap();
        
        match rt.block_on(self.storage.list(&dir_path)) {
            Ok(entries) => {
                for entry in entries {
                    if let Err(e) = rt.block_on(self.storage.delete(&entry)) {
                        warn!("Failed to delete {}: {}", entry, e);
                    }
                }
                // Delete directory marker
                let marker = format!("{}/.dir", path);
                let _ = rt.block_on(self.storage.delete(&marker));
                
                // Remove from maps
                if let Some(ino) = self.path_map.lock().unwrap().remove(&path) {
                    self.inode_map.lock().unwrap().remove(&ino);
                }
                
                reply.ok();
            }
            Err(e) => {
                error!("Error listing directory for removal: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
}

impl AegisFS {
    fn get_root_attr(&self) -> fuser::FileAttr {
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap();
        
        fuser::FileAttr {
            ino: 1,
            size: 4096,
            blocks: 1,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            flags: 0,
            blksize: BLOCK_SIZE as u32,
        }
    }

    fn get_dir_attr(&self) -> fuser::FileAttr {
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap();
        
        fuser::FileAttr {
            ino: 2,
            size: 4096,
            blocks: 1,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            flags: 0,
            blksize: BLOCK_SIZE as u32,
        }
    }

    async fn get_file_attr_for_path(&self, path: &str, ino: u64) -> fuser::FileAttr {
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap();
        
        let size = match self.read_file(path).await {
            Ok(Some(data)) => data.len() as u64,
            _ => 0,
        };
        
        fuser::FileAttr {
            ino,
            size,
            blocks: (size + BLOCK_SIZE - 1) / BLOCK_SIZE,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::RegularFile,
            perm: 0o644,
            nlink: 1,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            flags: 0,
            blksize: BLOCK_SIZE as u32,
        }
    }
    
    fn get_dir_attr_for_path(&self, ino: u64) -> fuser::FileAttr {
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap();
        
        fuser::FileAttr {
            ino,
            size: 4096,
            blocks: 1,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            flags: 0,
            blksize: BLOCK_SIZE as u32,
        }
    }
}

