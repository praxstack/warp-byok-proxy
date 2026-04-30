use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use std::path::{Path, PathBuf};

pub struct CertPaths {
    pub cert_pem: PathBuf,
    pub key_pem: PathBuf,
}

/// Generate a self-signed leaf cert + keypair and write `cert.pem` / `key.pem` into `out_dir`.
///
/// # Errors
///
/// Returns an error if the output directory cannot be created, a SAN DNS name is invalid,
/// keypair or certificate generation fails, or writing the PEM files fails.
pub fn generate_self_signed(out_dir: &Path, sans: &[&str]) -> Result<CertPaths> {
    std::fs::create_dir_all(out_dir).with_context(|| format!("mkdir {}", out_dir.display()))?;

    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "warp-byok-proxy");
    dn.push(DnType::OrganizationName, "praxstack");
    params.distinguished_name = dn;

    for san in sans {
        if let Ok(ip) = san.parse::<std::net::IpAddr>() {
            params.subject_alt_names.push(SanType::IpAddress(ip));
        } else {
            params
                .subject_alt_names
                .push(SanType::DnsName((*san).try_into().context("bad SAN DNS")?));
        }
    }

    let key = KeyPair::generate().context("keypair gen")?;
    let cert = params.self_signed(&key).context("self-sign cert")?;

    let cert_pem_path = out_dir.join("cert.pem");
    let key_pem_path = out_dir.join("key.pem");
    std::fs::write(&cert_pem_path, cert.pem().as_bytes()).context("write cert.pem")?;
    std::fs::write(&key_pem_path, key.serialize_pem().as_bytes()).context("write key.pem")?;

    Ok(CertPaths {
        cert_pem: cert_pem_path,
        key_pem: key_pem_path,
    })
}

/// macOS-only. Adds the cert to the system keychain so Warp's `hyper` client trusts it.
/// Requires sudo. Returns `false` and a hint if `security` is not available.
///
/// # Errors
///
/// Returns an error if spawning `sudo security` fails or the `security add-trusted-cert`
/// invocation exits non-zero (common cause: missing `sudo`, producing macOS error 100).
#[cfg(target_os = "macos")]
pub fn install_to_keychain(cert_pem: &Path) -> Result<()> {
    use std::process::Command;
    let status = Command::new("sudo")
        .arg("security")
        .arg("add-trusted-cert")
        .arg("-d")
        .arg("-r")
        .arg("trustRoot")
        .arg("-k")
        .arg("/Library/Keychains/System.keychain")
        .arg(cert_pem)
        .status()
        .context("spawn `sudo security`")?;
    anyhow::ensure!(
        status.success(),
        "security add-trusted-cert failed. If you didn't type sudo password, rerun with sudo. \
         If you saw error 100, the cert is already installed."
    );
    Ok(())
}

/// Non-macOS stub.
///
/// # Errors
///
/// Always returns an error — keychain trust install is only implemented on macOS in Phase 0.
#[cfg(not(target_os = "macos"))]
pub fn install_to_keychain(_cert_pem: &Path) -> Result<()> {
    anyhow::bail!("cert trust install only implemented on macOS in Phase 0")
}
