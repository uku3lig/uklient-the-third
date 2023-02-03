use crate::{Result, UklientError, STYLE_BYTE};
use flate2::bufread::GzDecoder;
use indicatif::ProgressBar;
use itertools::Itertools;
use libium::modpack::extract_zip;
use libium::HOME;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env::consts::{ARCH, OS};
use std::fs::File;
use std::ops::Deref;
use std::path::Path;
use std::time::Duration;
use std::{io::BufReader, path::PathBuf};
use tar::Archive;
use theseus::profile::JavaSettings;
use tokio::fs::{rename, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{error, info};

pub async fn get_java_settings(java_version: u8) -> JavaSettings {
    let java_name = if cfg!(windows) { "javaw.exe" } else { "java" };

    // TODO fork java_locator to look for multiple java versions (cf. prism's implementation of the java locator)
    let mut java_path =
        if let Some(java_home_path) = find_local_java(java_version) {
            info!("Found uklient Java: {java_home_path:?}");
            Some(java_home_path.join("bin").join(java_name))
        } else if let Ok(java_home) = java_locator::locate_file(java_name) {
            info!("Found Java: {java_home:?}");
            Some(PathBuf::from(java_home).join(java_name))
        } else {
            None
        };

    if java_path.is_none()
        || get_java_version(&java_path.clone().unwrap())
            .await
            .unwrap_or(0)
            != java_version
    {
        java_path = match download_java(java_version).await {
            Ok(java_bin_path) => {
                info!("Found downloaded Java: {java_bin_path:?}");
                Some(java_bin_path.join(java_name))
            },
            Err(e) => {
                error!("Error while downloading java: {e}");
                None
            }
        };
    }

    if let Some(p) = java_path.clone() {
        info!("Java version: {}", get_java_version(&p).await.unwrap_or(0));
    }

    JavaSettings {
        install: java_path,
        extra_arguments: None,
    }
}

async fn download_java(java_version: u8) -> Result<PathBuf> {
    let client = Client::new();
    let java_version = get_latest_java(java_version).await?;
    let download_url = format!(
        "https://api.adoptium.net/v3/binary/version/{java_version}/{OS}/{ARCH}/jdk/hotspot/normal/eclipse"
    );

    let tmp_dir = HOME.join(".config").join("uklient").join(".tmp");
    let java_dir = HOME.join(".config").join("uklient");

    let mut response = client.get(download_url).send().await?;

    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    let out_file_path = tmp_dir
        .join(java_version.replace('.', "-"))
        .with_extension(extension);

    let temp_file_path = out_file_path.with_extension("part");
    let mut temp_file = OpenOptions::new()
        .read(true)
        .write(true)
        .append(true)
        .create(true)
        .open(&temp_file_path)
        .await?;

    info!("Downloading Java {java_version}");
    let progress_bar = ProgressBar::new(response.content_length().unwrap_or(0))
        .with_style(STYLE_BYTE.clone());
    progress_bar.enable_steady_tick(Duration::from_millis(100));

    while let Some(chunk) = response.chunk().await? {
        temp_file.write_all(&chunk).await?;
        progress_bar.inc(chunk.len() as u64);
    }
    rename(&temp_file_path, &out_file_path).await?;

    progress_bar.finish();
    info!("Finished downloading Java!");

    let file = File::open(&out_file_path)?;
    if cfg!(windows) {
        extract_zip(file, &java_dir)
            .await
            .map_err(|_| UklientError::ZipError)?;
    } else {
        let reader = BufReader::new(file);
        let tar = GzDecoder::new(reader);
        let mut archive = Archive::new(tar);
        archive.unpack(&java_dir)?;
    }

    java_dir
        .read_dir()?
        .filter_map(|res| res.map(|dir| dir.path().join("bin")).ok())
        .find(|p| p.is_dir())
        .ok_or(UklientError::JavaNotFoundError)
}

async fn get_latest_java(java_version: u8) -> Result<String> {
    let client = Client::new();
    let url = format!(
        "https://api.adoptium.net/v3/info/release_names?project=jdk&release_type=ga&version=[{java_version},{})",
        java_version+1
    );

    let response = client.get(url).send().await?;
    let content: ReleaseNames = response.json().await?;

    content
        .releases
        .first()
        .cloned()
        .ok_or(UklientError::JavaNotFoundError)
}

fn find_local_java(java_version: u8) -> Option<PathBuf> {
    let uklient_dir = HOME.join(".config").join("uklient");
    let pattern =
        Regex::new(format!(r"jdk-{java_version}(?:\.\d+)+(?:\+\d+)?").as_str())
            .unwrap();

    if let Ok(dir) = uklient_dir.read_dir() {
        let java_name = dir
            .filter_map(|res| res.ok())
            .filter_map(|e| e.path().file_name().map(|s| s.to_os_string()))
            .filter(|n| pattern.find(&n.to_string_lossy()).is_some())
            .sorted()
            .rev()
            .next();

        java_name.map(|name| uklient_dir.join(name))
    } else {
        None
    }
}

async fn get_java_version(exec_path: &Path) -> Result<u8> {
    let regex = Regex::new(r#"version "(\d+\.\d+\.\d+)(?:_\d+)?""#).unwrap();

    let mut command = Command::new(exec_path.as_os_str());
    command.arg("-version");

    let output = command.output().await?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(String::from_utf8_lossy(&output.stderr).deref());

    if let Some(version) = regex.captures(text.as_str()).and_then(|c| c.get(1))
    {
        let mut parts = version.as_str().split('.');
        let major = parts
            .next()
            .and_then(|s| s.parse::<u8>().ok())
            .ok_or(UklientError::MetaError("java major"))?;

        match major {
            0 => Err(UklientError::MetaError("java major")),
            1 => match parts.next().and_then(|s| s.parse::<u8>().ok()) {
                Some(n) => Ok(n),
                None => Err(UklientError::MetaError("java minor")),
            },
            v => Ok(v),
        }
    } else {
        Err(UklientError::MetaError("java version not found"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleaseNames {
    releases: Vec<String>,
}
