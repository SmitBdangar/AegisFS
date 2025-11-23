use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{error, info};
use tracing_subscriber;

mod config;
mod crypto;
mod filesystem;
mod s3_client;
mod storage;

use config::Config;
use filesystem::AegisFS;

#[derive(Parser)]
#[command(name = "aegis-fs")]
#[command(about = "Transparent encryption proxy filesystem for S3 and remote filesystems")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Mount the filesystem
    Mount {
        /// Mount point directory
        #[arg(short, long)]
        mountpoint: PathBuf,
        /// Configuration file path
        #[arg(short, long, default_value = "aegis-fs.toml")]
        config: PathBuf,
    },
    /// Generate a new encryption key
    GenerateKey {
        /// Output file for the key
        #[arg(short, long, default_value = "aegis-fs.key")]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Mount { mountpoint, config } => {
            info!("Loading configuration from {:?}", config);
            let cfg = Config::load(&config)?;
            
            info!("Initializing AegisFS...");
            let fs = AegisFS::new(cfg).await?;
            
            info!("Mounting filesystem at {:?}", mountpoint);
            info!("Press Ctrl+C to unmount");
            
            let options = vec![
                fuser::MountOption::FSName("aegis-fs".to_string()),
                fuser::MountOption::AllowOther,
            ];
            
            fuser::mount2(fs, &mountpoint, &options)?;
            
            info!("Filesystem unmounted");
        }
        Commands::GenerateKey { output } => {
            info!("Generating encryption key...");
            let key = crypto::generate_key()?;
            std::fs::write(&output, hex::encode(key))?;
            info!("Key saved to {:?}", output);
            info!("Keep this key secure! It's required to decrypt your files.");
        }
    }

    Ok(())
}

