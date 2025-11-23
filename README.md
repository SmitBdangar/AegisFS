# AegisFS

**Transparent encryption proxy filesystem for S3 and remote filesystems**

AegisFS is a Rust-based FUSE filesystem that provides transparent encryption for files stored in S3-compatible object storage. All files are automatically encrypted before being uploaded and decrypted when accessed, ensuring your data remains secure even if your S3 bucket is compromised.

## Features

- 🔐 **Transparent Encryption**: Files are automatically encrypted/decrypted using AES-256-GCM
- ☁️ **S3 Compatible**: Works with AWS S3 and any S3-compatible storage (MinIO, DigitalOcean Spaces, etc.)
- 🚀 **High Performance**: Built with Rust for maximum performance and safety
- 🔑 **Key Management**: Simple key file-based encryption key management
- 📁 **POSIX Compatible**: Standard filesystem operations (read, write, delete, mkdir, etc.)
- 🛡️ **Secure by Default**: All data is encrypted at rest in the cloud

## Installation

### Prerequisites

- Rust 1.70+ (install from [rustup.rs](https://rustup.rs/))
- FUSE library (required for mounting filesystems)
  - **Linux**: `sudo apt-get install fuse3` (Debian/Ubuntu) or `sudo yum install fuse3` (RHEL/CentOS)
  - **macOS**: Install [macFUSE](https://osxfuse.github.io/)
  - **Windows**: Not currently supported (FUSE limitations)

### Build from Source

```bash
git clone https://github.com/yourusername/AegisFS.git
cd AegisFS
cargo build --release
```

The binary will be at `target/release/aegis-fs`.

## Quick Start

### 1. Generate an Encryption Key

First, generate a secure encryption key:

```bash
./target/release/aegis-fs generate-key --output aegis-fs.key
```

**⚠️ IMPORTANT**: Keep this key file secure! Without it, you cannot decrypt your files. Consider backing it up to a secure location.

### 2. Create Configuration File

Create a configuration file `aegis-fs.toml`:

```toml
[s3]
bucket = "my-encrypted-bucket"
region = "us-east-1"
# Optional: Custom endpoint for S3-compatible services
# endpoint = "https://s3.amazonaws.com"
# Optional: Credentials (if not using AWS credential chain)
# access_key_id = "your-access-key"
# secret_access_key = "your-secret-key"
prefix = ""  # Optional prefix for all objects

[encryption]
key_file = "aegis-fs.key"
algorithm = "aes256-gcm"

[cache]
directory = "/tmp/aegis-fs-cache"
max_size_mb = 1024
```

### 3. Mount the Filesystem

Create a mount point and mount the filesystem:

```bash
mkdir -p /mnt/aegis-fs
./target/release/aegis-fs mount --mountpoint /mnt/aegis-fs --config aegis-fs.toml
```

Now you can use `/mnt/aegis-fs` like any other directory. All files written here will be automatically encrypted and stored in S3.

### 4. Unmount

Press `Ctrl+C` in the terminal where AegisFS is running, or use:

```bash
fusermount -u /mnt/aegis-fs  # Linux
umount /mnt/aegis-fs         # macOS
```

## Configuration

### S3 Configuration

- `bucket`: The S3 bucket name (required)
- `region`: AWS region (required)
- `endpoint`: Optional custom endpoint for S3-compatible services
- `access_key_id`: Optional AWS access key (uses credential chain if not set)
- `secret_access_key`: Optional AWS secret key (uses credential chain if not set)
- `prefix`: Optional prefix for all objects in the bucket

### Encryption Configuration

- `key_file`: Path to the encryption key file (required)
- `algorithm`: Encryption algorithm (currently only `aes256-gcm`)

### Cache Configuration (Optional)

- `directory`: Local cache directory for improved performance
- `max_size_mb`: Maximum cache size in megabytes

## Usage Examples

### Basic File Operations

```bash
# Write a file (automatically encrypted)
echo "Hello, World!" > /mnt/aegis-fs/hello.txt

# Read a file (automatically decrypted)
cat /mnt/aegis-fs/hello.txt

# List files
ls -la /mnt/aegis-fs/

# Copy files
cp /path/to/local/file.txt /mnt/aegis-fs/

# Create directories
mkdir -p /mnt/aegis-fs/documents/2024
```

### Using with Applications

Any application that works with regular files can use AegisFS:

```bash
# Edit files with your favorite editor
vim /mnt/aegis-fs/notes.md

# Use with backup tools
rsync -av /home/user/documents/ /mnt/aegis-fs/backups/

# Mount in Docker containers
docker run -v /mnt/aegis-fs:/data myapp
```

## Security Considerations

1. **Key Management**: Store your encryption key securely. Consider using a password manager or hardware security module (HSM) for production deployments.

2. **Key Backup**: Without the encryption key, your data is unrecoverable. Ensure you have secure backups of your key file.

3. **Permissions**: The key file should have restrictive permissions:
   ```bash
   chmod 600 aegis-fs.key
   ```

4. **Network Security**: While data is encrypted at rest, ensure your connection to S3 is secure (HTTPS/TLS).

5. **Access Control**: Use IAM policies to restrict access to your S3 bucket.

## Architecture

AegisFS consists of several components:

- **FUSE Layer**: Provides the POSIX filesystem interface
- **Encryption Layer**: Handles AES-256-GCM encryption/decryption
- **S3 Client**: Manages communication with S3-compatible storage
- **Storage Backend**: Abstract interface for storage operations

```
Application
    ↓
FUSE Interface
    ↓
AegisFS
    ↓
Encryption Layer (AES-256-GCM)
    ↓
S3 Storage (Encrypted Data)
```

## Development

### Building

```bash
cargo build
```

### Running Tests

```bash
cargo test
```

### Code Structure

- `src/main.rs`: CLI entry point and command handling
- `src/config.rs`: Configuration management
- `src/crypto.rs`: Encryption/decryption implementation
- `src/s3_client.rs`: S3 storage backend
- `src/filesystem.rs`: FUSE filesystem implementation
- `src/storage.rs`: Storage backend trait

## Limitations

- Currently supports single-threaded operations (FUSE limitation)
- Large file handling may require optimization for production use
- Directory operations are simplified (uses marker files)
- No built-in caching yet (planned feature)

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Acknowledgments

- Built with [fuser](https://github.com/cberner/fuser) for FUSE support
- Uses [aws-sdk-s3](https://github.com/awslabs/aws-sdk-rust) for S3 integration
- Encryption powered by [aes-gcm](https://github.com/RustCrypto/AEADs)

## Support

For issues, questions, or contributions, please open an issue on GitHub.

