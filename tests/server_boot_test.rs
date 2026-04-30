use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn server_responds_200_on_health_endpoint() {
    // Use ephemeral port (0) for CI; integration against :443 is manual.
    let cert_tmp = tempfile::tempdir().unwrap();
    let paths =
        warp_byok_proxy::cert::generate_self_signed(cert_tmp.path(), &["127.0.0.1"]).unwrap();

    let (addr, shutdown) =
        warp_byok_proxy::server::spawn_test_server("127.0.0.1:0", &paths.cert_pem, &paths.key_pem)
            .await
            .unwrap();

    // Self-signed client
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let resp = client
        .get(format!("https://{addr}/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    shutdown.send(()).ok();
}
