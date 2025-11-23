use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub s3: S3Config,
    pub encryption: EncryptionConfig,
    pub cache: Option<CacheConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionConfig {
    pub key_file: String,
    pub algorithm: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub directory: String,
    pub max_size_mb: Option<u64>,
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read config file: {:?}", path.as_ref()))?;
        let config: Config = toml::from_str(&content)
            .context("Failed to parse config file")?;
        Ok(config)
    }

    pub fn default_config() -> String {
        r#"
[s3]
bucket = "my-bucket"
region = "us-east-1"
# endpoint = "https://s3.amazonaws.com"  # Optional custom endpoint
# access_key_id = "your-access-key"      # Optional, uses AWS credentials chain if not set
# secret_access_key = "your-secret-key"  # Optional, uses AWS credentials chain if not set
prefix = ""  # Optional prefix for all objects

[encryption]
key_file = "aegis-fs.key"
algorithm = "aes256-gcm"  # aes256-gcm or chacha20-poly1305

[cache]
directory = "/tmp/aegis-fs-cache"
max_size_mb = 1024
"#
        .to_string()
    }
}

