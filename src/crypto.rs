use anyhow::{Context, Result};
use aes_gcm::{
    aes::Aes256,
    AesGcm, Key, KeyInit, Nonce, NonceSizeUser,
};
use aes_gcm::aead::{Aead, NewAead};
use rand::RngCore;
use sha2::{Sha256, Digest};
use std::fs;

pub type EncryptionKey = [u8; 32];

pub fn generate_key() -> Result<EncryptionKey> {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    Ok(key)
}

pub fn load_key<P: AsRef<std::path::Path>>(path: P) -> Result<EncryptionKey> {
    let hex_key = fs::read_to_string(path.as_ref())
        .with_context(|| format!("Failed to read key file: {:?}", path.as_ref()))?;
    let hex_key = hex_key.trim();
    let key_bytes = hex::decode(hex_key)
        .context("Failed to decode hex key")?;
    
    if key_bytes.len() != 32 {
        anyhow::bail!("Key must be 64 hex characters (32 bytes)");
    }
    
    let mut key = [0u8; 32];
    key.copy_from_slice(&key_bytes);
    Ok(key)
}

pub fn derive_key_from_password(password: &str, salt: &[u8]) -> EncryptionKey {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hasher.update(salt);
    let hash = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&hash);
    key
}

pub struct Encryptor {
    cipher: AesGcm<Aes256>,
}

impl Encryptor {
    pub fn new(key: EncryptionKey) -> Self {
        let key = Key::<AesGcm<Aes256>>::from_slice(&key);
        let cipher = AesGcm::new(key);
        Self { cipher }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        // Generate a random nonce for each encryption
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        let mut ciphertext = self.cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;
        
        // Prepend nonce to ciphertext
        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.append(&mut ciphertext);
        
        Ok(result)
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if ciphertext.len() < 12 {
            anyhow::bail!("Ciphertext too short");
        }
        
        let nonce = Nonce::from_slice(&ciphertext[0..12]);
        let encrypted_data = &ciphertext[12..];
        
        let plaintext = self.cipher
            .decrypt(nonce, encrypted_data)
            .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))?;
        
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let key = generate_key().unwrap();
        let encryptor = Encryptor::new(key);
        
        let plaintext = b"Hello, World!";
        let ciphertext = encryptor.encrypt(plaintext).unwrap();
        let decrypted = encryptor.decrypt(&ciphertext).unwrap();
        
        assert_eq!(plaintext, decrypted.as_slice());
    }
}

