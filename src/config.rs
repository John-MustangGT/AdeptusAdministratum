use anyhow::{Context, Result};
use serde::Deserialize;
use std::{net::IpAddr, path::PathBuf};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub data: DataConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: IpAddr,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct DataConfig {
    #[serde(default = "default_tabularium_dir")]
    pub tabularium_dir: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

impl Default for DataConfig {
    fn default() -> Self {
        Self {
            tabularium_dir: default_tabularium_dir(),
        }
    }
}

fn default_host() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}

fn default_port() -> u16 {
    3000
}

fn default_tabularium_dir() -> PathBuf {
    PathBuf::from("tabularium")
}

impl Config {
    /// Load configuration from `config.toml` in the current directory.
    /// Falls back to defaults if the file is absent.
    pub fn load() -> Result<Self> {
        let path = std::path::Path::new("config.toml");
        if !path.exists() {
            tracing::info!("No config.toml found — using defaults");
            return Ok(Config {
                server: ServerConfig::default(),
                data: DataConfig::default(),
            });
        }
        let raw = std::fs::read_to_string(path)
            .context("Failed to read config.toml")?;
        let cfg: Config = toml::from_str(&raw)
            .context("Failed to parse config.toml")?;
        Ok(cfg)
    }
}
