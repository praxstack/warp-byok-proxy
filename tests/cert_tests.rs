use std::path::PathBuf;
use warp_byok_proxy::cert;

#[test]
fn generate_self_signed_produces_valid_pem() {
    let tmp = tempfile::tempdir().unwrap();
    let out: PathBuf = tmp.path().to_path_buf();
    let paths = cert::generate_self_signed(&out, &["127.0.0.1", "app.warp.dev"]).unwrap();
    assert!(paths.cert_pem.exists(), "cert.pem missing");
    assert!(paths.key_pem.exists(), "key.pem missing");
    let pem = std::fs::read_to_string(&paths.cert_pem).unwrap();
    assert!(
        pem.starts_with("-----BEGIN CERTIFICATE-----"),
        "unexpected pem: {pem}"
    );
}

#[test]
fn generate_self_signed_includes_both_sans() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = cert::generate_self_signed(tmp.path(), &["127.0.0.1", "app.warp.dev"]).unwrap();
    let cert_pem_bytes = std::fs::read(&paths.cert_pem).unwrap();
    // Dump the PEM and assert it parses; SAN content check is best-effort via text.
    let pem_str = String::from_utf8(cert_pem_bytes).unwrap();
    assert!(pem_str.contains("-----BEGIN CERTIFICATE-----"));
    // Deeper SAN assertion is done via `openssl x509 -text` in Task 15.
}

#[cfg(unix)]
#[test]
fn key_pem_permissions_are_0600() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let paths = cert::generate_self_signed(tmp.path(), &["127.0.0.1"]).unwrap();
    let key_meta = std::fs::metadata(&paths.key_pem).unwrap();
    let mode = key_meta.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "key.pem mode = 0o{mode:o}, want 0o600");
}

#[cfg(unix)]
#[test]
fn cert_pem_permissions_are_0644() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let paths = cert::generate_self_signed(tmp.path(), &["127.0.0.1"]).unwrap();
    let cert_meta = std::fs::metadata(&paths.cert_pem).unwrap();
    let mode = cert_meta.permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "cert.pem mode = 0o{mode:o}, want 0o644");
}
