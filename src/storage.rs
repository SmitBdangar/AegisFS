use async_trait::async_trait;
use bytes::Bytes;
use anyhow::Result;

#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn get(&self, path: &str) -> Result<Option<Bytes>>;
    async fn put(&self, path: &str, data: Bytes) -> Result<()>;
    async fn delete(&self, path: &str) -> Result<()>;
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;
    async fn exists(&self, path: &str) -> Result<bool>;
}

