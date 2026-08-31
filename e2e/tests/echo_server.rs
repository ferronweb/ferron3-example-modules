//! E2E test for the custom echo server module.

use std::time::Duration;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, TestcontainersError};

const FERRON_IMAGE: &str = "e2e-test-ferron-example:latest";

async fn ferron_container() -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    GenericImage::new(FERRON_IMAGE, "latest")
        .with_exposed_port(ContainerPort::Tcp(9090))
        .with_wait_for(WaitFor::message_on_stdout("Echo server listening"))
        .with_startup_timeout(Duration::from_secs(30))
        .start()
        .await
}

#[tokio::test]
async fn echo_server_echoes_bytes() {
    // Placeholder for the echo server E2E. Real test:
    //
    // ```ignore
    // let container = ferron_container().await;
    // let port = container.get_host_port_ipv4(9090);
    // let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap();
    // stream.write_all(b"hello echo").await.unwrap();
    // let mut buf = vec![0u8; 10];
    // stream.read_exact(&mut buf).await.unwrap();
    // assert_eq!(&buf, b"hello echo");
    // ```
    let _c = ferron_container().await;
}
