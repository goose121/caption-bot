use std::error::Error;
use std::path::{PathBuf, Path};
use std::fs::File;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub bot_token: String,
    pub application_id: u64,
    pub model_path: PathBuf,
    pub webhook_url: String,
    pub db_path: PathBuf
}

impl Config {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Config, Box<dyn Error + Send + Sync>> {
        Ok(serde_yaml::from_reader(File::open(path.as_ref())?)?)
    }
}
