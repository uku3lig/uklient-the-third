use crate::Result;
use flate2::bufread::GzDecoder;
use libium::HOME;
use reqwest::Client;
use std::fs::File;
use std::{io::BufReader, path::PathBuf};
use tar::Archive;
use theseus::profile::JavaSettings;
use tokio::fs::{rename, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};

// TODO fetch the latest java version
const JAVA_NAME: &'static str = "17-0-6-tem";
const JAVA_URL: &'static str =
    "https://api.sdkman.io/2/broker/download/java/17.0.6-tem/";

pub async fn get_java_settings() -> JavaSettings {
    let java_name = if cfg!(windows) { "javaw" } else { "java" };

    // TODO fork java_locator to look for multiple java versions (cf. prism's implementation of the java locator)
    // TODO look for already existing installations of java in .config/uklient
    let java_path = match java_locator::locate_file(java_name) {
        Ok(java_home) => {
            info!("Found Java: {java_home:?}");
            Some(PathBuf::from(java_home).join(java_name))
        }
        Err(_) => {
            warn!("Java not found, downloading it");
            if let Ok(java_home) = download_java().await {
                Some(PathBuf::from(java_home).join(java_name))
            } else {
                error!("Could not download java :breh:");
                None
            }
        }
    };

    JavaSettings {
        install: java_path,
        extra_arguments: None,
    }
}

async fn download_java() -> Result<PathBuf> {
    if !cfg!(unix) {
        todo!("windows bad")
    }

    let client = Client::new();

    let mut download_url = String::from(JAVA_URL);
    download_url.push_str(std::env::consts::OS);

    let tmp_dir = HOME.join(".config").join("uklient").join(".tmp");
    let java_dir = HOME.join(".config").join("uklient");

    let mut response = client.get(download_url).send().await?;
    // TODO this is platform specific
    let out_file_path = tmp_dir.join(JAVA_NAME).with_extension("tar.gz");
    let temp_file_path = out_file_path.with_extension("part");
    let mut temp_file = OpenOptions::new()
        .read(true)
        .write(true)
        .append(true)
        .create(true)
        .open(&temp_file_path)
        .await?;

    info!("Downloading Java {JAVA_NAME}");

    while let Some(chunk) = response.chunk().await? {
        temp_file.write_all(&chunk).await?;
    }
    rename(&temp_file_path, &out_file_path).await?;

    info!("Finished downloading Java!");

    let reader = BufReader::new(File::open(&out_file_path)?);
    let tar = GzDecoder::new(reader);
    let mut archive = Archive::new(tar);
    archive.unpack(&java_dir)?;

    // TODO
    Ok(java_dir.join("jdk-17.0.6+10").join("bin"))
}
