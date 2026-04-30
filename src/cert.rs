use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub struct CertPaths {
    pub cert_pem: PathBuf,
    pub key_pem: PathBuf,
}

fn write_with_mode(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: mode() only applies when open(2) creates a new inode. If a
        // file already exists at `path`, the kernel reuses its existing inode
        // and mode — truncate resets content but not permissions. Removing
        // first ensures we create fresh and the declared mode takes effect.
        if let Err(e) = std::fs::remove_file(path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(anyhow::Error::from(e)
                    .context(format!("remove {} before write", path.display())));
            }
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true).mode(mode);
        let mut f = opts
            .open(path)
            .with_context(|| format!("open {}", path.display()))?;
        f.write_all(contents)
            .with_context(|| format!("write {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
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

    // Phase 0: SANs come from a hardcoded list in main.rs. If Phase B exposes
    // SANs via user config, add DNS-label validation (e.g. regex) to reject
    // malformed inputs like "1.2.3" that fail IpAddr parsing and then fall
    // through to DnsName::try_from as a nonsensical label.
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
    write_with_mode(&cert_pem_path, cert.pem().as_bytes(), 0o644)?;
    write_with_mode(&key_pem_path, key.serialize_pem().as_bytes(), 0o600)?;

    Ok(CertPaths {
        cert_pem: cert_pem_path,
        key_pem: key_pem_path,
    })
}

/// macOS-only. Adds the cert to the system keychain so Warp's `hyper` client trusts it.
/// Requires sudo.
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
