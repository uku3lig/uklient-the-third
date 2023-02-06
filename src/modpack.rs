use crate::UklientError::{MetaError, UnknownTypeError, ZipError};
use crate::{get_latest_fabric, get_latest_quilt, Result, UklientError};
use daedalus::modded::LoaderVersion;
use ferinth::Ferinth;
use fs_extra::{
    dir::{copy as copy_dir, CopyOptions as DirCopyOptions},
    file::{move_file, CopyOptions as FileCopyOptions},
};
use itertools::Itertools;
use libium::modpack::extract_zip;
use libium::modpack::modrinth::deser_metadata;
use libium::modpack::modrinth::read_metadata_file;
use libium::upgrade::Downloadable;
use libium::version_ext::VersionExt;
use libium::HOME;
use reqwest::Client;
use std::fs::File;
use std::{
    ffi::OsString,
    fs::read_dir,
    path::{Path, PathBuf},
    sync::Arc,
};
use theseus::prelude::{ModLoader, ProfileMetadata};
use tokio::{
    fs::{copy, create_dir_all, remove_file},
    sync::Semaphore,
    task::JoinSet,
};
use tracing::{info, warn};

pub async fn get_metadata(
    id: &str,
    game_version: &str,
) -> Result<ProfileMetadata> {
    let modrinth = Ferinth::default();

    let info = modrinth.get_project(id).await?;
    let versions = modrinth
        .list_versions_filtered(id, None, Some(&[game_version]), None)
        .await?;

    if let Some(meta) = versions.first() {
        let loader_info = match meta.loaders.first() {
            Some(l) => LoaderInfo::from(l, &game_version.into()).await?,
            None => return Err(MetaError("loader")),
        };

        Ok(ProfileMetadata {
            name: format!("{}-{}", info.title, meta.name),
            loader: loader_info.loader,
            loader_version: Some(loader_info.version),
            game_version: game_version.into(),
            format_version: 1,
            icon: None,
        })
    } else {
        Err(MetaError("modpack"))
    }
}

#[derive(Debug)]
struct LoaderInfo {
    loader: ModLoader,
    version: LoaderVersion,
}

impl LoaderInfo {
    async fn from(value: &str, game_version: &String) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "fabric" => Ok(Self {
                loader: ModLoader::Fabric,
                version: get_latest_fabric(game_version).await?,
            }),
            "quilt" => Ok(Self {
                loader: ModLoader::Quilt,
                version: get_latest_quilt(game_version).await?,
            }),
            _ => Err(MetaError("unknown loader")),
        }
    }
}

// code BLATANTLY stolen from ferium

pub async fn install_modpack(
    output_dir: &Path,
    id: &str,
    game_version: String,
) -> Result<()> {
    let modrinth = Ferinth::default();

    let version = modrinth
        .list_versions(id)
        .await?
        .iter()
        .find(|v| v.game_versions.contains(&game_version))
        .ok_or(MetaError("modpack"))?
        .clone();

    info!("Found modpack version {}", version.name);

    let mut version_file: Downloadable = version.into_version_file().into();
    version_file.output = version_file.filename().into();

    let cache_dir = HOME.join(".config").join("uklient").join(".cache");
    create_dir_all(&cache_dir).await?;

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

    let tmp_dir = HOME
        .join(".config")
        .join("uklient")
        .join(".tmp")
        .join(metadata.name);
    extract_zip(modpack_file, &tmp_dir)
        .await
        .map_err(|_| ZipError)?;
    let overrides = read_overrides(&tmp_dir.join("overrides"))?;

    let mut to_download: Vec<Downloadable> = Vec::new();
    for file in metadata.files {
        to_download.push(file.into());
    }

    clean(&output_dir.join("mods"), &mut to_download, &mut Vec::new()).await?;
    clean(
        &output_dir.join("resourcepacks"),
        &mut to_download,
        &mut Vec::new(),
    )
    .await?;

    if to_download.is_empty() && overrides.is_empty() {
        info!("Everything is up to date!");
        Ok(())
    } else {
        download(output_dir.into(), to_download, overrides).await
    }
}

fn read_overrides(directory: &Path) -> Result<Vec<(OsString, PathBuf)>> {
    let mut to_install = Vec::new();
    for file in read_dir(directory)? {
        let file = file?;
        to_install.push((file.file_name(), file.path()));
    }
    Ok(to_install)
}

async fn clean(
    directory: &Path,
    to_download: &mut Vec<Downloadable>,
    to_install: &mut Vec<(OsString, PathBuf)>,
) -> Result<()> {
    let dupes = find_dupes_by_key(to_download, Downloadable::filename);
    if !dupes.is_empty() {
        warn!(
            "{} duplicate files were found {}. Remove the mod it belongs to",
            dupes.len(),
            dupes
                .into_iter()
                .map(|i| to_download.swap_remove(i).filename())
                .format(", ")
        );
    }
    create_dir_all(directory.join(".old")).await?;
    for file in read_dir(directory)? {
        let file = file?;
        // If it's a file
        if file.file_type()?.is_file() {
            let filename = file.file_name();
            let filename = filename.to_string_lossy();
            let filename = filename.as_ref();
            // If it is already downloaded
            if let Some(index) = to_download
                .iter()
                .position(|thing| filename == thing.filename())
            {
                // Don't download it
                to_download.swap_remove(index);
            // Likewise, if it is already installed
            } else if let Some(index) =
                to_install.iter().position(|thing| filename == thing.0)
            {
                // Don't install it
                to_install.swap_remove(index);
            // Or else, move the file to `directory`/.old
            // If the file is a `.part` file or if the move failed, delete the file
            } else if filename.ends_with("part")
                || move_file(
                    file.path(),
                    directory.join(".old").join(filename),
                    &FileCopyOptions::new(),
                )
                .is_err()
            {
                remove_file(file.path()).await?;
            }
        }
    }
    Ok(())
}

async fn download(
    output_dir: PathBuf,
    to_download: Vec<Downloadable>,
    to_install: Vec<(OsString, PathBuf)>,
) -> Result<()> {
    create_dir_all(&*output_dir).await?;
    let mut tasks = JoinSet::new();
    let semaphore = Arc::new(Semaphore::new(75));
    let client = Arc::new(Client::new());
    let output_dir = Arc::new(output_dir);
    for downloadable in to_download {
        let permit = semaphore.clone().acquire_owned().await?;
        let output_dir = output_dir.clone();
        let client = client.clone();
        tasks.spawn(async move {
            let _permit = permit;
            info!("Downloading {}", downloadable.filename());
            downloadable.download(&client, &output_dir, |_| {}).await?;
            Ok::<(), UklientError>(())
        });
    }
    while let Some(res) = tasks.join_next().await {
        res??;
    }
    for installable in to_install {
        if installable.1.is_file() {
            copy(installable.1, output_dir.join(&installable.0)).await?;
        } else if installable.1.is_dir() {
            let mut copy_options = DirCopyOptions::new();
            copy_options.overwrite = true;
            copy_dir(installable.1, &*output_dir, &copy_options)?;
        } else {
            return Err(UnknownTypeError(installable.0));
        }
        info!("Installed {}", installable.0.to_string_lossy());
    }

    Ok(())
}

fn find_dupes_by_key<T, V, F>(slice: &mut [T], key: F) -> Vec<usize>
where
    V: Eq + Ord,
    F: Fn(&T) -> V,
{
    let mut indices = Vec::new();
    if slice.len() < 2 {
        return indices;
    }
    slice.sort_unstable_by_key(&key);
    for i in 0..(slice.len() - 1) {
        if key(&slice[i]) == key(&slice[i + 1]) {
            indices.push(i);
        }
    }
    indices.reverse();
    indices
}
