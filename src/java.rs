use crate::{Result, UklientError};
use flate2::bufread::GzDecoder;
use libium::HOME;
use regex::Regex;
use reqwest::Client;
use std::env::consts::OS;
use std::fs::File;
use std::{io::BufReader, path::PathBuf};
use tar::Archive;
use theseus::profile::JavaSettings;
use tokio::fs::{rename, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::{error, info};

pub async fn get_java_settings() -> JavaSettings {
    let java_name = if cfg!(windows) { "javaw" } else { "java" };

    // TODO fork java_locator to look for multiple java versions (cf. prism's implementation of the java locator)
    // TODO look for already existing installations of java in .config/uklient
    let java_path = if let Ok(java_home) = java_locator::locate_file(java_name)
    {
        info!("Found Java: {java_home:?}");
        Some(PathBuf::from(java_home).join(java_name))
    } else if let Ok(java_home_path) = download_java().await {
        Some(java_home_path.join(java_name))
    } else {
        error!("Could not download java :breh:");
        None
    };

    JavaSettings {
        install: java_path,
        extra_arguments: None,
    }
}

async fn get_latest_java(java_version: u8) -> Result<String> {
    let pattern =
        Regex::new(format!(r"{java_version}(?:\.\d+)+-tem").as_str()).unwrap();
    let client = Client::new();
    let url = format!(
        "https://api.sdkman.io/2/candidates/java/{OS}/versions/list?installed="
    );

    let response = client.get(url).send().await?;
    let content = response.text().await?;

    if let Some(match_) = pattern.find(content.as_str()) {
        Ok(String::from(match_.as_str()))
    } else {
        Err(UklientError::JavaNotFoundError)
    }
}

async fn download_java() -> Result<PathBuf> {
    if !cfg!(unix) {
        todo!("windows bad")
    }

    let client = Client::new();
    let java_version = get_latest_java(17).await?;
    let download_url = format!(
        "https://api.sdkman.io/2/broker/download/java/{java_version}/{OS}"
    );

    let tmp_dir = HOME.join(".config").join("uklient").join(".tmp");
    let java_dir = HOME.join(".config").join("uklient");

    let mut response = client.get(download_url).send().await?;
    // TODO this is platform specific
    let out_file_path = tmp_dir
        .join(java_version.replace('.', "-"))
        .with_extension("tar.gz");
    let temp_file_path = out_file_path.with_extension("part");
    let mut temp_file = OpenOptions::new()
        .read(true)
        .write(true)
        .append(true)
        .create(true)
        .open(&temp_file_path)
        .await?;

    info!("Downloading Java {java_version}");

    while let Some(chunk) = response.chunk().await? {
        temp_file.write_all(&chunk).await?;
    }
    rename(&temp_file_path, &out_file_path).await?;

    info!("Finished downloading Java!");

    let reader = BufReader::new(File::open(&out_file_path)?);
    let tar = GzDecoder::new(reader);
    let mut archive = Archive::new(tar);
    archive.unpack(&java_dir)?;

    java_dir
        .read_dir()?
        .filter_map(|res| res.map(|dir| dir.path().join("bin")).ok())
        .find(|p| p.is_dir())
        .ok_or(UklientError::JavaNotFoundError)
}
