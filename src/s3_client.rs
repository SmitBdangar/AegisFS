use anyhow::{Context, Result};
use aws_config::{Region, meta::credentials::StaticCredentialsProvider};
use aws_sdk_s3::{Client as S3Client, Config as S3Config, config::Credentials};
use bytes::Bytes;
use std::sync::Arc;
use tracing::{debug, error, warn};

use crate::config::S3Config as ConfigS3Config;

pub struct S3Storage {
    client: S3Client,
    bucket: String,
    prefix: String,
}

impl S3Storage {
    pub async fn new(config: ConfigS3Config) -> Result<Self> {
        let region = Region::new(config.region.clone());
        
        let mut s3_config_builder = aws_config::Config::builder()
            .region(region);
        
        // Use provided credentials if available, otherwise use default credential chain
        if let (Some(access_key), Some(secret_key)) = (config.access_key_id, config.secret_access_key) {
            let creds = Credentials::new(
                access_key,
                secret_key,
                None,
                None,
                "aegis-fs",
            );
            let provider = StaticCredentialsProvider::new(creds);
            s3_config_builder = s3_config_builder.credentials_provider(provider);
        }
        
        // Set custom endpoint if provided
        if let Some(endpoint) = config.endpoint {
            s3_config_builder = s3_config_builder.endpoint_url(endpoint);
        }
        
        let aws_config = s3_config_builder.load().await;
        let client = S3Client::from_conf(S3Config::builder()
            .with_config(&aws_config)
            .build());
        
        let prefix = config.prefix.unwrap_or_default();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            warn!("S3 prefix should end with '/', adding it automatically");
        }
        
        Ok(Self {
            client,
            bucket: config.bucket,
            prefix,
        })
    }

    fn object_key(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        format!("{}{}", self.prefix, path)
    }

    pub async fn get(&self, path: &str) -> Result<Option<Bytes>> {
        let key = self.object_key(path);
        debug!("Getting object: {}", key);
        
        match self.client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(output) => {
                let data = output
                    .body
                    .collect()
                    .await
                    .context("Failed to read S3 object body")?
                    .into_bytes();
                Ok(Some(data))
            }
            Err(aws_sdk_s3::error::SdkError::ServiceError(err)) => {
                if err.err().is_no_such_key() {
                    Ok(None)
                } else {
                    error!("S3 error: {:?}", err);
                    Err(anyhow::anyhow!("S3 error: {}", err.err()))
                }
            }
            Err(e) => {
                error!("S3 error: {:?}", e);
                Err(anyhow::anyhow!("S3 error: {}", e))
            }
        }
    }

    pub async fn put(&self, path: &str, data: Bytes) -> Result<()> {
        let key = self.object_key(path);
        debug!("Putting object: {}", key);
        
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(data.into())
            .send()
            .await
            .context("Failed to put object to S3")?;
        
        Ok(())
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let key = self.object_key(path);
        debug!("Deleting object: {}", key);
        
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .context("Failed to delete object from S3")?;
        
        Ok(())
    }

    pub async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let key_prefix = self.object_key(prefix);
        debug!("Listing objects with prefix: {}", key_prefix);
        
        let mut objects = Vec::new();
        let mut continuation_token = None;
        
        loop {
            let mut request = self.client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&key_prefix);
            
            if let Some(token) = continuation_token {
                request = request.continuation_token(token);
            }
            
            let output = request
                .send()
                .await
                .context("Failed to list objects from S3")?;
            
            if let Some(contents) = output.contents() {
                for obj in contents {
                    if let Some(key) = obj.key() {
                        // Remove the prefix from the key to get the relative path
                        let relative_path = if key.starts_with(&self.prefix) {
                            &key[self.prefix.len()..]
                        } else {
                            key
                        };
                        objects.push(relative_path.to_string());
                    }
                }
            }
            
            continuation_token = output.next_continuation_token().map(|s| s.to_string());
            if continuation_token.is_none() {
                break;
            }
        }
        
        Ok(objects)
    }

    pub async fn exists(&self, path: &str) -> Result<bool> {
        let key = self.object_key(path);
        
        match self.client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(aws_sdk_s3::error::SdkError::ServiceError(err)) => {
                if err.err().is_not_found() {
                    Ok(false)
                } else {
                    Err(anyhow::anyhow!("S3 error: {}", err.err()))
                }
            }
            Err(e) => Err(anyhow::anyhow!("S3 error: {}", e)),
        }
    }
}

