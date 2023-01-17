use crate::UklientError::{MetaError, NoHomeError};
use daedalus::modded::LoaderVersion;
use serde::{Deserialize, Serialize};
use std::fs;
use std::ops::Index;
use std::path::PathBuf;
use theseus::auth::Credentials;
use theseus::data::{
    JavaSettings, MemorySettings, ModLoader, ProfileMetadata, WindowSize,
};
use theseus::profile;
use theseus::profile::Profile;
use thiserror::Error;
use tokio::sync::oneshot;

type Result<T> = std::result::Result<T, UklientError>;

const META_URL: &str = "https://meta.fabricmc.net/v2";

#[tokio::main]
async fn main() -> Result<()> {
    let java_name = if cfg!(windows) { "javaw" } else { "java" };
    let java_path: PathBuf = [java_locator::locate_file(java_name)?, java_name.to_string()].iter().collect();

    println!("Found Java: {:?}", java_path);
    let java = JavaSettings {
        install: Some(java_path),
        extra_arguments: None,
    };

    let base_path: PathBuf = [home::home_dir().ok_or(NoHomeError)?, ".uklient".into()].iter().collect();
    fs::create_dir_all(&base_path)?;
    println!("Created directory {:?}", base_path);

    let fabric_version = get_latest_fabric().await?;
    println!("Found fabric version {}", fabric_version.id);

    let mc_profile = Profile {
        path: base_path.clone(),
        metadata: ProfileMetadata {
            name: "uku's pvp modpack".into(),
            loader: ModLoader::Fabric,
            loader_version: Some(fabric_version),
            game_version: "1.19.3".into(),
            format_version: 1,
            icon: None,
        },
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
    println!("Connected account {}", cred.username);

    let process = profile::run(&base_path, &cred).await?;
    if let Some(pid) = process.id() {
        println!("PID: {}", pid);
    } else {
        println!("NO PID? no bitches");
    }

    process.wait_with_output().await?;
    println!("Goodbye!");

    Ok(())
}

async fn get_latest_fabric() -> Result<LoaderVersion> {
    let downloaded = daedalus::download_file(
        format!("{}/versions/", META_URL).as_str(),
        None,
    )
    .await?;
    let versions: FabricVersions = serde_json::from_slice(&downloaded)?;
    let latest = versions.loader.get(0).ok_or(MetaError("fabric"))?.clone();
    let latest_mc = versions.game.get(0).ok_or(MetaError("minecraft"))?.clone();
    let manifest_url = format!(
        "{}/versions/loader/{}/{}/profile/json",
        META_URL, latest_mc.version, latest.version
    );

    Ok(LoaderVersion {
        id: latest.version,
        stable: latest.stable,
        url: manifest_url,
    })
}

async fn connect_account() -> Result<Credentials> {
    let (tx, rx) = oneshot::channel::<url::Url>();
    let flow = tokio::spawn(theseus::auth::authenticate(tx));

    let url = rx.await?;
    webbrowser::open(url.as_str())?;

    Ok(flow.await??)
}

#[derive(Error, Debug)]
enum UklientError {
    #[error("Java could not be located")]
    JavaLocateError(#[from] java_locator::errors::JavaLocatorError),
    #[error("tokio recv error")]
    RecvError(#[from] oneshot::error::RecvError),
    #[error("browser error :3")]
    IoError(#[from] std::io::Error),
    #[error("tokio join error")]
    JoinError(#[from] tokio::task::JoinError),
    #[error("theseus error")]
    TheseusError(#[from] theseus::Error),
    #[error("daedalus error")]
    DaedalusError(#[from] daedalus::Error),
    #[error("json error")]
    JsonError(#[from] serde_json::Error),
    #[error("no {0} versions were found")]
    MetaError(&'static str),
    #[error("no home dir was found")]
    NoHomeError,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
/// Versions of fabric components
struct FabricVersions {
    /// Versions of Minecraft that fabric supports
    pub game: Vec<FabricGameVersion>,
    /// Available versions of the fabric loader
    pub loader: Vec<FabricLoaderVersion>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
/// A version of Minecraft that fabric supports
struct FabricGameVersion {
    /// The version number of the game
    pub version: String,
    /// Whether the Minecraft version is stable or not
    pub stable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
/// A version of the fabric loader
struct FabricLoaderVersion {
    /// The separator to get the build number
    pub separator: String,
    /// The build number
    pub build: u32,
    /// The maven artifact
    pub maven: String,
    /// The version number of the fabric loader
    pub version: String,
    /// Whether the loader is stable or not
    pub stable: bool,
}
