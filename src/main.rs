use std::fs;
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

#[tokio::main]
async fn main() -> Result<()> {
    let java_name = if cfg!(windows) { "javaw" } else { "java" };
    let mut java_path = PathBuf::from(java_locator::locate_file(java_name)?);
    java_path.push(java_name);

    println!("Found Java: {:?}", java_path);

    let java = JavaSettings {
        install: Some(java_path),
        extra_arguments: None,
    };

    let path = PathBuf::from("/home/leo/.uklient");
    fs::create_dir_all(&path)?;

    println!("Created directory {:?}", path);

    let mc_profile = Profile {
        path: path.clone(),
        metadata: ProfileMetadata {
            name: "uku's pvp modpack".into(),
            loader: ModLoader::Fabric,
            loader_version: None,
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

    let process = profile::run(&path, &cred).await?;

    if let Some(pid) = process.id() {
        println!("PID: {}", pid);
    } else {
        println!("NO PID? no bitches");
    }

    process.wait_with_output().await?;
    println!("Goodbye!");

    Ok(())
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
}
