# Ferron Example Modules — E2E test suite

This directory contains the end-to-end test suite for the example modules in
`ferron3-example-modules`, written in Rust. It uses [Testcontainers](https://testcontainers.com/)
to spin up Ferron (built from `fixture` + `Dockerfile.test`) and verifies
behavior with `reqwest` and `tokio`.

## Prerequisites

- **Docker** - tests rely on Docker to run Ferron. Ensure the Docker daemon is running.
- **Rust** - standard Rust toolchain (Cargo).
- **`protoc`** - not required for these tests (no gRPC), but required for the main Ferron E2E suite.

## How to run tests

To run the entire suite:

```bash
cargo test
```

To run a specific test file (e.g. `http_examples`):

```bash
cargo test --test http_examples
```

Each test file is defined as `[[test]]` in `e2e/Cargo.toml`. Tests are in `tests/`.

## Rebuilding the Ferron test image

The test suite automatically builds a Docker image `e2e-test-ferron-example` from
the local source code (see `tests/common/mod.rs::build_ferron_image`). The build
is cached via `BuildImageOptions::skip_if_exists(true)` and an in-process
`LazyLock` cache. If the image is not rebuilt, tests will use the already-built
image, which might not reflect the latest changes.

To force a rebuild of the web server Docker image (e.g. after modifying Rust
sources or `Dockerfile.test`), you need to remove the existing image. On Linux
hosts, you can run:

```bash
docker rm -f $(docker ps -a --filter ancestor=e2e-test-ferron-example -q) 2>/dev/null || true
docker image rm e2e-test-ferron-example 2>/dev/null || true
```

The next time you run `cargo test`, the image will be rebuilt.

## How helpers work

- `tests/common/mod.rs` provides:
  - `build_ferron_image()` - builds `e2e-test-ferron-example:latest` from `e2e/Dockerfile.test` with the repository root as build context (via `glob` + `with_file`), cached in a `LazyLock<Mutex<Option<GenericImage>>>` and via `skip_if_exists(true)`.
  - `create_ferron_container(webroot, config)` - starts the image, mounts `webroot` at `/var/www/ferron` and `config` at `/etc/ferron.conf`, exposes port 80 and waits for `GET /` to return any 2xx-4xx.
  - `create_ferron_container_with_echo(webroot, config)` - same but also exposes `9090` for the echo server example.
  - `create_temp_dir()`, `create_temp_file()`, `write_file()`, `create_dir()` - helpers that set permissive modes (`0o777`/`0o666`) on Unix so the container's `nobody` user can read them.

## How to write tests

Tests live in `tests/`. Each file is an integration test defined in `Cargo.toml`:

```toml
[[test]]
name = "my_new_feature"
path = "tests/my_new_feature.rs"
```

Implement tests using `common` to spawn Ferron and `reqwest` (or `tokio::net::TcpStream`)
to verify behavior. See `tests/http_examples.rs` and `tests/echo_server.rs` for
examples that exercise `ferron-http-header-append`, `ferron-http-hello`, and
`ferron-echo-server`.

For HTTP tests, create a temporary webroot and config file, start Ferron, then
issue requests:

```rust
mod common;
use common::{create_ferron_container, create_temp_dir, create_temp_file, write_file};

#[tokio::test]
async fn my_test() {
    let webroot = create_temp_dir();
    write_file(webroot.path().join("index.html"), b"hello").unwrap();
    let mut config = create_temp_file();
    config.as_file_mut().write_all(br#"*:80 { root "/var/www/ferron" }"#).unwrap();
    let container = create_ferron_container(webroot.path(), config.path()).await.unwrap();
    let port = container.get_host_port_ipv4(80.into()).await.unwrap();
    // ... reqwest assertions ...
}
```
