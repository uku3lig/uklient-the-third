use crate::UklientError::{MetaError, ZipError};
use daedalus::modded::LoaderVersion;
use ferinth::Ferinth;

use libium::modpack::modrinth::{deser_metadata, read_metadata_file};
use libium::upgrade::Downloadable;

use libium::version_ext::VersionExt;
use libium::HOME;
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::File;
use std::path::PathBuf;

use reqwest::Client;
use theseus::auth::Credentials;
use theseus::data::{
    JavaSettings, MemorySettings, ModLoader, ProfileMetadata, WindowSize,
};
use theseus::profile;
use theseus::profile::Profile;
use thiserror::Error;
use tokio::sync::oneshot;

type Result<T> = std::result::Result<T, UklientError>;

const FABRIC_META_URL: &str = "https://meta.fabricmc.net/v2";
const QUILT_META_URL: &str = "https://meta.quiltmc.org/v3";

#[tokio::main]
async fn main() -> Result<()> {
    let java_name = if cfg!(windows) { "javaw" } else { "java" };
    let java_path: PathBuf =
        [java_locator::locate_file(java_name)?, java_name.to_string()]
            .iter()
            .collect();

    println!("Found Java: {:?}", java_path);
    let java = JavaSettings {
        install: Some(java_path),
        extra_arguments: None,
    };

    let base_path: PathBuf = [HOME.clone(), ".uklient".into()].iter().collect();
    let paths = [
        &base_path,
        &[base_path.clone(), "mods".into()].iter().collect(),
    ];

    for path in paths {
        fs::create_dir_all(path)?;
        println!("Created directory {:?}", path);
    }

    let game_version = "1.19.3".to_string();
    let loader = ModLoader::Quilt;
    let loader_version = if loader == ModLoader::Quilt {
        get_latest_quilt(&game_version).await
    } else {
        get_latest_fabric(&game_version).await
    }?;
    println!("Found {} version {}", loader, loader_version.id);

    let mc_profile = Profile {
        path: base_path.clone(),
        metadata: ProfileMetadata {
            name: "uku's pvp modpack".into(),
            loader,
            loader_version: Some(loader_version),
            game_version: game_version.clone(),
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

    install_modpack(&base_path, game_version, loader).await?;
    println!("Sucessfully installed modpack");

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

async fn install_modpack(
    base_path: &PathBuf,
    game_version: String,
    loader: ModLoader,
) -> Result<()> {
    let modrinth = Ferinth::default();
    let client = Client::new();
    let loader = format!("{loader}");

    let version = modrinth
        .list_versions("JR0bkFKa")
        .await?
        .iter()
        .filter(|v| v.game_versions.contains(&game_version))
        .filter(|v| v.loaders.iter().any(|s| s.eq_ignore_ascii_case(&loader)))
        .next()
        .ok_or(MetaError("modpack"))?
        .clone();

    println!("Found modpack version {}", version.name);

    let mut version_file: Downloadable = version.into_version_file().into();
    version_file.output = version_file.filename().into();

    let cache_dir = HOME.join(".config").join("uklient").join(".cache");
    fs::create_dir_all(&cache_dir)?;
    let modpack_path = cache_dir.join(&version_file.output);
    if !modpack_path.exists() {
        version_file
            .download(&Client::new(), &cache_dir, |_| {})
            .await?;
    }

    let modpack_file = File::open(modpack_path)?;
    let metadata = deser_metadata(
        &read_metadata_file(&modpack_file).map_err(|_| ZipError)?,
    )?;

    for file in metadata.files {
        let downloadable: Downloadable = file.into();

        let (size, name) = downloadable
            .download(&client, base_path.as_path(), |_| {})
            .await?;
        println!("Downloaded {name} (size: {size})");
    }

    Ok(())
}

async fn get_latest_fabric(mc_version: &String) -> Result<LoaderVersion> {
    let downloaded = daedalus::download_file(
        format!("{}/versions/loader/{}", FABRIC_META_URL, mc_version).as_str(),
        None,
    )
    .await?;

    let versions: Vec<LoaderVersionElement> = serde_json::from_slice(&downloaded)?;
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

async fn get_latest_quilt(mc_version: &String) -> Result<LoaderVersion> {
    let downloaded = daedalus::download_file(
        format!("{}/versions/loader/{}", QUILT_META_URL, mc_version).as_str(),
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
    #[serde(default="bool::default")]
    pub stable: bool,
}
