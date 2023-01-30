mod java;
mod modpack;

use crate::java::get_java_settings;
use crate::modpack::get_metadata;
use crate::UklientError::MetaError;
use daedalus::modded::LoaderVersion;
use std::ffi::OsString;
use tracing::{debug, info, warn};

use libium::HOME;
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use theseus::auth::Credentials;
use theseus::data::{MemorySettings, WindowSize};
use theseus::profile;
use theseus::profile::Profile;
use thiserror::Error;
use tokio::sync::oneshot;

type Result<T> = std::result::Result<T, UklientError>;

const FABRIC_META_URL: &str = "https://meta.fabricmc.net/v2";
const QUILT_META_URL: &str = "https://meta.quiltmc.org/v3";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let java = get_java_settings().await;

    let base_path: PathBuf = HOME.join(".uklient");
    let paths = [&base_path, &base_path.join("mods")];
    for path in paths {
        fs::create_dir_all(path)?;
        debug!("Created directory {path:?}");
    }

    let game_version = "1.19.3".to_string();
    let metadata = get_metadata("JR0bkFKa", game_version.as_str()).await?;
    debug!("Found {}", metadata.loader);

    let mc_profile = Profile {
        path: base_path.clone(),
        metadata,
        java: Some(java),
        memory: Some(MemorySettings {
            maximum: (4 * 1024) as u32,
            ..MemorySettings::default()
        }),
        resolution: Some(WindowSize(1280, 720)),
        hooks: None,
    };

    profile::add(mc_profile).await?;
    let cred = connect_account().await?;
    info!("Connected account {}", cred.username);

    modpack::install_modpack(&base_path, "JR0bkFKa", game_version).await?;
    info!("Sucessfully installed modpack");

    let process = profile::run(&base_path, &cred).await?;
    if let Some(pid) = process.id() {
        info!("PID: {pid}");
    } else {
        warn!("NO PID? no bitches");
    }

    process.wait_with_output().await?;
    info!("Goodbye!");

    Ok(())
}

pub async fn get_latest_fabric(mc_version: &String) -> Result<LoaderVersion> {
    let downloaded = daedalus::download_file(
        format!("{FABRIC_META_URL}/versions/loader/{mc_version}").as_str(),
        None,
    )
    .await?;

    let versions: Vec<LoaderVersionElement> =
        serde_json::from_slice(&downloaded)?;
    let latest = versions.get(0).ok_or(MetaError("fabric"))?.loader.clone();
    let manifest_url = format!(
        "{}/versions/loader/{}/{}/profile/json",
        FABRIC_META_URL, mc_version, latest.version
    );

    Ok(LoaderVersion {
        id: latest.version,
        stable: latest.stable,
        url: manifest_url,
    })
}

pub async fn get_latest_quilt(mc_version: &String) -> Result<LoaderVersion> {
    let downloaded = daedalus::download_file(
        format!("{QUILT_META_URL}/versions/loader/{mc_version}").as_str(),
        None,
    )
    .await?;

    let versions: Vec<LoaderVersionElement> =
        serde_json::from_slice(&downloaded)?;
    let latest = versions.get(0).ok_or(MetaError("quilt"))?.loader.clone();
    let manifest_url = format!(
        "{}/versions/loader/{}/{}/profile/json",
        QUILT_META_URL, mc_version, latest.version
    );

    Ok(LoaderVersion {
        id: latest.version,
        stable: latest.stable,
        url: manifest_url,
    })
}

async fn connect_account() -> Result<Credentials> {
    let credentials_path = Path::new("./credentials.json");

    if credentials_path.try_exists()? {
        let credentials: Result<Credentials> = {
            let file = File::open(credentials_path)?;
            let creds: Credentials =
                serde_json::from_reader(BufReader::new(file))?;

            Ok(theseus::auth::refresh(creds.id, true).await?)
        };

        if let Ok(creds) = credentials {
            return Ok(creds);
        }
    }

    let (tx, rx) = oneshot::channel::<url::Url>();
    let flow = tokio::spawn(theseus::auth::authenticate(tx));

    let url = rx.await?;
    webbrowser::open(url.as_str())?;

    let creds = flow.await??;
    let file = File::create(credentials_path)?;
    serde_json::to_writer(BufWriter::new(file), &creds)?;

    Ok(creds)
}

#[derive(Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum UklientError {
    #[error("Java could not be located")]
    JavaLocateError(#[from] java_locator::errors::JavaLocatorError),
    #[error("tokio recv error")]
    RecvError(#[from] oneshot::error::RecvError),
    #[error("browser error :3")]
    IoError(#[from] std::io::Error),
    #[error("fs_extra error")]
    FsExtraError(#[from] fs_extra::error::Error),
    #[error("tokio join error")]
    JoinError(#[from] tokio::task::JoinError),
    #[error("theseus error")]
    TheseusError(#[from] theseus::Error),
    #[error("daedalus error")]
    DaedalusError(#[from] daedalus::Error),
    #[error("json error")]
    JsonError(#[from] serde_json::Error),
    #[error("libium error")]
    LibiumError(#[from] libium::upgrade::Error),
    #[error("libium modpack error")]
    LibiumModpackError(#[from] libium::upgrade::modpack_downloadable::Error),
    #[error("ferinth error")]
    FerinthError(#[from] ferinth::Error),
    #[error("zip error")]
    ZipError,
    #[error("no {0} versions were found")]
    MetaError(&'static str),
    #[error("unknown type")]
    UnknownTypeError(OsString),
    #[error("acquire error")]
    AcquireError(#[from] tokio::sync::AcquireError),
    #[error("reqwest error")]
    ReqwestError(#[from] reqwest::Error),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
/// A version of Minecraft that fabric supports
struct GameVersion {
    /// The version number of the game
    pub version: String,
    /// Whether the Minecraft version is stable or not
    pub stable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct LoaderVersionElement {
    pub loader: MetaLoaderVersion,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct MetaLoaderVersion {
    /// The separator to get the build number
    pub separator: String,
    /// The build number
    pub build: u32,
    /// The maven artifact
    pub maven: String,
    /// The version number of the fabric loader
    pub version: String,
    /// Whether the loader is stable or not
    #[serde(default = "bool::default")]
    pub stable: bool,
}
