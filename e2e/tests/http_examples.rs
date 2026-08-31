//! E2E tests for the HTTP example modules (`ferron-http-header-append` and `ferron-http-hello`).
//!
//! These tests boot the `e2e-test-ferron-example` image (built from
//! `e2e/Dockerfile.test`) via testcontainers, mount a temporary webroot and
//! config, and verify that the example stages behave correctly.

use std::io::Write;

use testcontainers::core::ContainerPort;

mod common;

use common::{create_ferron_container, create_temp_dir, create_temp_file, write_file};

/// Helper to initialize crypto provider for `reqwest` with `rustls-no-provider`.
fn init_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[tokio::test]
async fn hello_stage_serves_default_greeting() {
    init_crypto();

    let webroot = create_temp_dir();
    write_file(webroot.path().join("index.html"), b"<h1>index</h1>").unwrap();

    let mut config = create_temp_file();
    config
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
    hello_path /hello
    hello_message "Hello from Ferron!"
}
"#,
        )
        .unwrap();

    let container = create_ferron_container(webroot.path(), config.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    // The hello stage should intercept /hello and return the greeting.
    let resp = client.get(format!("{base}/hello")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "Hello from Ferron!");

    // Other paths continue through the pipeline and serve static files.
    let resp = client
        .get(format!("{base}/index.html"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("index"));
}

#[tokio::test]
async fn hello_stage_custom_message_and_path() {
    init_crypto();

    let webroot = create_temp_dir();
    write_file(webroot.path().join("index.html"), b"index").unwrap();

    let mut config = create_temp_file();
    config
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
    hello_path /greet
    hello_message "Greetings, traveler!"
}
"#,
        )
        .unwrap();

    let container = create_ferron_container(webroot.path(), config.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    let resp = client.get(format!("{base}/greet")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "Greetings, traveler!");

    // Default /hello should not match when hello_path is /greet.
    let resp = client.get(format!("{base}/hello")).send().await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn header_append_stage_adds_header() {
    init_crypto();

    let webroot = create_temp_dir();
    write_file(webroot.path().join("index.html"), b"hello world").unwrap();

    let mut config = create_temp_file();
    config
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
    example_header hello-world
}
"#,
        )
        .unwrap();

    let container = create_ferron_container(webroot.path(), config.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/index.html"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // The example header stage appends X-Example-Header in run_inverse.
    let header = resp
        .headers()
        .get("x-example-header")
        .expect("x-example-header missing")
        .to_str()
        .unwrap();
    assert_eq!(header, "hello-world");
}

#[tokio::test]
async fn header_and_hello_together() {
    init_crypto();

    let webroot = create_temp_dir();
    write_file(webroot.path().join("index.html"), b"index").unwrap();

    let mut config = create_temp_file();
    config
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
    example_header hello-world
    hello_path /hello
    hello_message "Hello!"
}
"#,
        )
        .unwrap();

    let container = create_ferron_container(webroot.path(), config.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    // hello response should also have the header (hello stage sets response,
    // header stage adds header in run_inverse).
    let resp = client.get(format!("{base}/hello")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let header = resp
        .headers()
        .get("x-example-header")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(header, "hello-world");
    assert_eq!(resp.text().await.unwrap(), "Hello!");

    // static file also has header
    let resp = client
        .get(format!("{base}/index.html"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("x-example-header").unwrap(),
        "hello-world"
    );
}
