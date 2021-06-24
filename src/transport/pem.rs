//! Utilities for reading PEM files as [`Certificate`]s and [`PrivateKey`]s, as necessary to
//! initialize TLS.

use std::{fs::File, io, io::Read, path::Path};
use tokio_rustls::rustls::{Certificate, PrivateKey};

/// Read the file at `path` into memory as a vector of PEM-encoded `CERTIFICATE`s, silently skipping
/// any entries in the file which are not labeled `CERTIFICATE`.
pub fn read_certificates(path: impl AsRef<Path>) -> Result<Vec<Certificate>, io::Error> {
    let mut file = File::open(&path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;

    let mut certificates = Vec::new();
    for pem::Pem { contents, .. } in pem::parse_many(contents)
        .into_iter()
        .filter(|p| p.tag == "CERTIFICATE")
    {
        certificates.push(Certificate(contents));
    }
    Ok(certificates)
}

/// Read the file at `path` as a single PEM-encoded `CERTIFICATE`.
#[cfg(feature = "allow_explicit_certificate_trust")]
pub fn read_single_certificate(path: impl AsRef<Path>) -> Result<Certificate, io::Error> {
    let mut file = File::open(&path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;

    let pem = pem::parse(contents).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid PEM encoding in certificate: {}", e),
        )
    })?;
    if pem.tag == "CERTIFICATE" {
        Ok(Certificate(pem.contents))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("not labeled as a certificate: '{}'", pem.tag),
        ))
    }
}

/// Read the file at `path` as a single PEM-encoded `PRIVATE KEY`.
pub fn read_private_key(path: impl AsRef<Path>) -> Result<PrivateKey, io::Error> {
    let mut file = File::open(&path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;

    let pem = pem::parse(contents).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid PEM encoding in private key: {}", e),
        )
    })?;
    if pem.tag == "PRIVATE KEY" {
        Ok(PrivateKey(pem.contents))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("not labeled as a private key: '{}'", pem.tag),
        ))
    }
}
