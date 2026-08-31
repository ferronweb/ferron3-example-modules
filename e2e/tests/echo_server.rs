//! E2E tests for the custom echo server module (`ferron-echo-server`).
//!
//! The echo server is a `Module` that spawns a Tokio TCP listener on
//! `0.0.0.0:9090` and echoes bytes. The tests verify that a TCP client can
//! connect, send data, and receive the same bytes back.

use std::io::Write;

use testcontainers::core::ContainerPort;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod common;

use common::{create_ferron_container_with_echo, create_temp_dir, create_temp_file, write_file};

fn init_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[tokio::test]
async fn echo_server_echoes_bytes() {
    init_crypto();

    let webroot = create_temp_dir();
    write_file(webroot.path().join("index.html"), b"<h1>hello</h1>").unwrap();

    let mut config = create_temp_file();
    config
        .as_file_mut()
        .write_all(
            br#"
{
    echo_server {
        listen "0.0.0.0:9090"
    }
}

*:80 {
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();

    let container = create_ferron_container_with_echo(webroot.path(), config.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(9090))
        .await
        .unwrap();
    let addr = format!("127.0.0.1:{port}");

    // Connect and verify echo.
    let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let payload = b"hello echo";
    stream.write_all(payload).await.unwrap();
    stream.flush().await.unwrap();
    // Shutdown write side to signal EOF for echo server's copy.
    stream.shutdown().await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    assert_eq!(buf, payload);
}

#[tokio::test]
async fn echo_server_multiple_connections() {
    init_crypto();

    let webroot = create_temp_dir();
    write_file(webroot.path().join("index.html"), b"index").unwrap();

    let mut config = create_temp_file();
    config
        .as_file_mut()
        .write_all(
            br#"
{
    echo_server {
        listen "0.0.0.0:9090"
    }
}

*:80 {
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();

    let container = create_ferron_container_with_echo(webroot.path(), config.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(9090))
        .await
        .unwrap();
    let addr = format!("127.0.0.1:{port}");

    // Verify two sequential connections both echo correctly.
    for msg in [b"first" as &[u8], b"second payload"] {
        let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
        stream.write_all(msg).await.unwrap();
        stream.shutdown().await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, msg);
    }

    // Also verify HTTP still works alongside the echo server.
    let http_port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{http_port}/index.html"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "index");
}
