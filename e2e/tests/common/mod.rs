#![allow(dead_code)]

#[cfg(unix)]
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(not(unix))]
use std::path::{Path, PathBuf};

use testcontainers::{
    core::{wait::HttpWaitStrategy, BuildImageOptions, ContainerPort, Mount, WaitFor},
    runners::{AsyncBuilder, AsyncRunner},
    ContainerAsync, GenericBuildableImage, GenericImage, ImageExt, TestcontainersError,
};
use tokio::sync::Mutex;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

static FERRON_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Create a Ferron container that mounts the given webroot and config.
///
/// The container exposes port 80 (HTTP) and waits until `GET /` returns any
/// 2xx-4xx response. It also mounts the webroot at `/var/www/ferron` and the
/// config at `/etc/ferron.conf`. Reuses the previously built image if available
/// (see [`build_ferron_image`]).
pub async fn create_ferron_container(
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network("bridge")
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .with_env_var("FERRON_ROOT", "/var/www/ferron")
        .start()
        .await
}

/// Create a Ferron container that also exposes the echo server port (9090).
///
/// Used for the `echo-server` example which listens on `0.0.0.0:9090`. The
/// container exposes both 80 and 9090, but the wait strategy still checks
/// port 80 (`GET /`) so the HTTP pipeline is ready before tests run.
pub async fn create_ferron_container_with_echo(
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_exposed_port(ContainerPort::Tcp(9090))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network("bridge")
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .with_env_var("FERRON_ROOT", "/var/www/ferron")
        .start()
        .await
}

#[cfg(unix)]
pub fn create_temp_dir() -> tempfile::TempDir {
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());
    tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o777))
        .tempdir()
        .unwrap()
}

#[cfg(not(unix))]
pub fn create_temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[cfg(unix)]
pub fn create_temp_file() -> tempfile::NamedTempFile {
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());
    tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666))
        .tempfile()
        .unwrap()
}

#[cfg(not(unix))]
pub fn create_temp_file() -> tempfile::NamedTempFile {
    tempfile::NamedTempFile::new().unwrap()
}

/// Build (or reuse) the Ferron example image `e2e-test-ferron-example`.
///
/// The image is built from `e2e/Dockerfile.test` with the repository root
/// as build context. The build is cached via `BuildImageOptions::skip_if_exists`
/// and also cached in-process via a `LazyLock<Mutex>`. To force a rebuild
/// after changing Rust sources, remove the image:
///
/// ```bash
/// docker rm -f $(docker ps -a --filter ancestor=e2e-test-ferron-example -q) 2>/dev/null || true
/// docker image rm e2e-test-ferron-example 2>/dev/null || true
/// ```
///
/// The next `cargo test` will rebuild it.
pub async fn build_ferron_image() -> Result<GenericImage, TestcontainersError> {
    let mut ferron_image = FERRON_IMAGE.lock().await;
    if let Some(image) = ferron_image.as_ref() {
        return Ok(image.clone());
    }
    let mut builder = GenericBuildableImage::new("e2e-test-ferron-example", "latest")
        .with_dockerfile(concat!(env!("CARGO_MANIFEST_DIR"), "/Dockerfile.test"));
    // Copy the repository root into the build context, excluding artifacts
    // that would bloat the image or cause cache invalidation.
    for entry in glob::glob(concat!(env!("CARGO_MANIFEST_DIR"), "/../*")).unwrap() {
        let entry = entry.unwrap();
        let dest = entry.file_name().unwrap().to_str().unwrap().to_string();
        if dest != "target"
            && dest != ".git"
            && dest != "e2e"
            && dest != "fuzz"
            && dest != "cross-build"
            && !dest.starts_with("Dockerfile.")
        {
            builder = builder.with_file(entry, format!("./{dest}"));
        }
    }
    // Also include the `e2e` directory's Dockerfile (not the whole e2e target)
    // The build already has the Dockerfile via `with_dockerfile`, but we keep
    // the filter above that excludes `e2e` to avoid copying `e2e/target`.
    // Re-add only the Dockerfile for completeness if needed.
    let ferron_image_built = builder
        .build_image_with(BuildImageOptions::new().with_skip_if_exists(true))
        .await?;
    ferron_image.replace(ferron_image_built.clone());
    Ok(ferron_image_built)
}

pub fn write_file(path: PathBuf, content: &[u8]) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o666)
        .open(path);
    #[cfg(unix)]
    let result = file.and_then(|mut file| file.write_all(content));
    #[cfg(not(unix))]
    let result = std::fs::write(path, content);

    result
}

pub fn create_dir(path: PathBuf) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    let result = std::fs::DirBuilder::new().mode(0o777).create(path);
    #[cfg(not(unix))]
    let result = std::fs::create_dir(path);

    result
}
