//! # Daedalus
//!
//! Daedalus is a library which provides models and methods to fetch metadata about games

#![warn(missing_docs, unused_import_braces, missing_debug_implementations)]

use once_cell::sync::OnceCell;

/// Models and methods for fetching metadata for Minecraft
pub mod minecraft;
/// Models and methods for fetching metadata for Minecraft mod loaders
pub mod modded;

/// Your branding, used for the user agent and similar
#[derive(Debug)]
pub struct Branding {
    /// The name of your application
    pub header_value: String,
    /// The string to replace in the name of the application
    pub dummy_replace_string: String,
}

/// The branding of your application
pub static BRANDING: OnceCell<Branding> = OnceCell::new();

impl Branding {
    /// Creates a new branding instance
    pub fn new(name: String, email: String) -> Branding {
        let email = format!(
            "{}/daedalus/{} <{}>",
            name,
            env!("CARGO_PKG_VERSION"),
            email
        );
        let dummy_replace_string = format!("${{{}.gameVersion}}", name);

        Branding {
            header_value: email,
            dummy_replace_string,
        }
    }

    /// Returns the branding instance
    pub fn set_branding(branding: Branding) -> Result<(), Error> {
        BRANDING
            .set(branding)
            .map_err(|_| Error::BrandingAlreadySet)
    }
}

impl Default for Branding {
    fn default() -> Self {
        Branding::new("unbranded".to_string(), "unbranded".to_string())
    }
}

#[derive(thiserror::Error, Debug)]
/// An error type representing possible errors when fetching metadata
pub enum Error {
    #[error("Failed to validate file checksum at url {url} with hash {hash} after {tries} tries")]
    /// A checksum was failed to validate for a file
    ChecksumFailure {
        /// The checksum's hash
        hash: String,
        /// The URL of the file attempted to be downloaded
        url: String,
        /// The amount of tries that the file was downloaded until failure
        tries: u32,
    },
    /// There was an error while deserializing metadata
    #[error("Error while deserializing JSON")]
    SerdeError(#[from] serde_json::Error),
    /// There was a network error when fetching an object
    #[error("Unable to fetch {item}")]
    FetchError {
        /// The internal reqwest error
        inner: reqwest::Error,
        /// The item that was failed to be fetched
        item: String,
    },
    /// There was an error when managing async tasks
    #[error("Error while managing asynchronous tasks")]
    TaskError(#[from] tokio::task::JoinError),
    /// Error while parsing input
    #[error("{0}")]
    ParseError(String),
    /// The branding has already been set
    #[error("Branding already set")]
    BrandingAlreadySet,
    /// Invalid Minecraft Java Profile
    #[error("Invalid Minecraft Java Profile")]
    InvalidMinecraftJavaProfile(String),
}

/// Converts a maven artifact to a path
pub fn get_path_from_artifact(artifact: &str) -> Result<String, Error> {
    let name_items = artifact.split(':').collect::<Vec<&str>>();

    let package = name_items.first().ok_or_else(|| {
        Error::ParseError(format!(
            "Unable to find package for library {}",
            &artifact
        ))
    })?;
    let name = name_items.get(1).ok_or_else(|| {
        Error::ParseError(format!(
            "Unable to find name for library {}",
            &artifact
        ))
    })?;

    if name_items.len() == 3 {
        let version_ext = name_items
            .get(2)
            .ok_or_else(|| {
                Error::ParseError(format!(
                    "Unable to find version for library {}",
                    &artifact
                ))
            })?
            .split('@')
            .collect::<Vec<&str>>();
        let version = version_ext.first().ok_or_else(|| {
            Error::ParseError(format!(
                "Unable to find version for library {}",
                &artifact
            ))
        })?;
        let ext = version_ext.get(1);

        Ok(format!(
            "{}/{}/{}/{}-{}.{}",
            package.replace('.', "/"),
            name,
            version,
            name,
            version,
            ext.unwrap_or(&"jar")
        ))
    } else {
        let version = name_items.get(2).ok_or_else(|| {
            Error::ParseError(format!(
                "Unable to find version for library {}",
                &artifact
            ))
        })?;

        let data_ext = name_items
            .get(3)
            .ok_or_else(|| {
                Error::ParseError(format!(
                    "Unable to find data for library {}",
                    &artifact
                ))
            })?
            .split('@')
            .collect::<Vec<&str>>();
        let data = data_ext.first().ok_or_else(|| {
            Error::ParseError(format!(
                "Unable to find data for library {}",
                &artifact
            ))
        })?;
        let ext = data_ext.get(1);

        Ok(format!(
            "{}/{}/{}/{}-{}-{}.{}",
            package.replace('.', "/"),
            name,
            version,
            name,
            version,
            data,
            ext.unwrap_or(&"jar")
        ))
    }
}

/// Downloads a file from specified mirrors
pub async fn download_file_mirrors(
    base: &str,
    mirrors: &[&str],
    sha1: Option<&str>,
) -> Result<bytes::Bytes, Error> {
    if mirrors.is_empty() {
        return Err(Error::ParseError("No mirrors provided!".to_string()));
    }

    for (index, mirror) in mirrors.iter().enumerate() {
        let result = download_file(&format!("{}{}", mirror, base), sha1).await;

        if result.is_ok() || (result.is_err() && index == (mirrors.len() - 1)) {
            return result;
        }
    }

    unreachable!()
}

/// Downloads a file with retry and checksum functionality
pub async fn download_file(
    url: &str,
    sha1: Option<&str>,
) -> Result<bytes::Bytes, Error> {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Ok(header) = reqwest::header::HeaderValue::from_str(
        &BRANDING.get_or_init(Branding::default).header_value,
    ) {
        headers.insert(reqwest::header::USER_AGENT, header);
    }
    let client = reqwest::Client::builder()
        .tcp_keepalive(Some(std::time::Duration::from_secs(10)))
        .timeout(std::time::Duration::from_secs(15))
        .default_headers(headers)
        .build()
        .map_err(|err| Error::FetchError {
            inner: err,
            item: url.to_string(),
        })?;

    for attempt in 1..=4 {
        let result = client.get(url).send().await;

        match result {
            Ok(x) => {
                let bytes = x.bytes().await;

                if let Ok(bytes) = bytes {
                    if let Some(sha1) = sha1 {
                        if &*get_hash(bytes.clone()).await? != sha1 {
                            if attempt <= 3 {
                                continue;
                            } else {
                                return Err(Error::ChecksumFailure {
                                    hash: sha1.to_string(),
                                    url: url.to_string(),
                                    tries: attempt,
                                });
                            }
                        }
                    }

                    return Ok(bytes);
                } else if attempt <= 3 {
                    continue;
                } else if let Err(err) = bytes {
                    return Err(Error::FetchError {
                        inner: err,
                        item: url.to_string(),
                    });
                }
            }
            Err(_) if attempt <= 3 => continue,
            Err(err) => {
                return Err(Error::FetchError {
                    inner: err,
                    item: url.to_string(),
                })
            }
        }
    }

    unreachable!()
}

/// Computes a checksum of the input bytes
pub async fn get_hash(bytes: bytes::Bytes) -> Result<String, Error> {
    let hash =
        tokio::task::spawn_blocking(|| sha1::Sha1::from(bytes).hexdigest())
            .await?;

    Ok(hash)
}
