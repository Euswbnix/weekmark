//! The managed Codex runtime (design §2.3): downloaded on the student's explicit action from
//! OpenAI's own GitHub release, verified against the pin (size, SHA-256, then the code signature),
//! unpacked atomically into `<local>/runtimes/codex/<version>/`, at most two versions kept.
//!
//! - A partial download is never resumed or trusted: a cancelled or failed install deletes it.
//! - Redirects are followed only from github.com to `*.githubusercontent.com`, over HTTPS.
//! - One install at a time across processes (`install.lock`, an OS file lock like `sync.lock`).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use pagelamp_core::brand;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use super::pin::{Pin, PinAsset, Version};
use super::process::RemoveOnDrop;

const LOCK_FILE: &str = "install.lock";
const PARTIAL_PREFIX: &str = ".partial-";
const TMP_SUFFIX: &str = ".tmp";
/// Versions kept after an install: the new one and the previous one, for rollback.
const KEEP_VERSIONS: usize = 2;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// No byte for this long: the download has stalled.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// The unpacked binary is ≈250 MB; anything far larger is not Codex.
const MAX_UNPACKED_BYTES: u64 = 1 << 30;
/// Developer ID team of OpenAI OpCo, LLC (design §2.3).
#[cfg(target_os = "macos")]
const OPENAI_TEAM_ID: &str = "2DC432GLL2";

/// Progress of `Runtime::install` (the facade maps it to its `RuntimeEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallEvent {
    DownloadStarted {
        total_bytes: u64,
    },
    Progress {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Verifying,
    Installing,
    Done {
        version: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("the Codex install was cancelled")]
    Cancelled,
    #[error("Codex isn't available for this computer")]
    UnsupportedPlatform,
    #[error("Codex is being installed by another PageLamp window")]
    Busy,
    #[error("the Codex download failed: {0}")]
    Network(String),
    /// Size, SHA-256 or signature: the file is not the pinned Codex.
    #[error("the downloaded Codex failed its check: {0}")]
    Verify(String),
    #[error("could not install Codex: {0}")]
    Io(String),
}

impl From<std::io::Error> for InstallError {
    fn from(err: std::io::Error) -> Self {
        InstallError::Io(err.to_string())
    }
}

/// An installed Codex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub version: Version,
    pub binary: PathBuf,
}

/// How the unpacked binary's signature is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SignatureRule {
    /// macOS: a valid Developer ID signature of OpenAI's team. Windows: a valid Authenticode
    /// signature if the file has one. Linux: none (the pinned SHA-256 is the check).
    OpenAi,
    /// Synthetic test binaries.
    #[cfg(test)]
    Skip,
}

/// `<local>/runtimes/codex`.
#[derive(Clone, Debug)]
pub struct Runtime {
    root: PathBuf,
}

impl Runtime {
    pub fn new(root: impl Into<PathBuf>) -> Runtime {
        Runtime { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Installed versions, newest first: folders named after a version holding the binary.
    pub fn installed(&self) -> Vec<Installed> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut found: Vec<Installed> = entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().into_string().ok()?;
                let version = Version::parse(&name).filter(|v| v.to_string() == name)?;
                ["codex", "codex.exe"].iter().find_map(|binary| {
                    let path = entry.path().join(binary);
                    path.is_file().then(|| Installed {
                        version: version.clone(),
                        binary: path,
                    })
                })
            })
            .collect();
        found.sort_by(|a, b| b.version.cmp(&a.version));
        found
    }

    /// The installed copy of `version`, if any.
    pub fn get(&self, version: &Version) -> Option<Installed> {
        self.installed().into_iter().find(|i| &i.version == version)
    }

    /// Download, verify and install `asset` of `pin`. Nothing is left behind unless it succeeds.
    pub async fn install(
        &self,
        pin: &Pin,
        asset: &PinAsset,
        install_id: &str,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(InstallEvent) + Send + Sync),
    ) -> Result<Installed, InstallError> {
        let url = pin.download_url(asset);
        self.install_from(
            &pin.version(),
            asset,
            &url,
            SignatureRule::OpenAi,
            install_id,
            cancel,
            on_event,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn install_from(
        &self,
        version: &Version,
        asset: &PinAsset,
        url: &str,
        signature: SignatureRule,
        install_id: &str,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(InstallEvent) + Send + Sync),
    ) -> Result<Installed, InstallError> {
        if !install_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || install_id.is_empty()
        {
            return Err(InstallError::Io("invalid install id".into()));
        }
        pagelamp_core::paths::create_private_dir_all(&self.root)?;
        let _lock = self.lock()?;
        self.clean_leftovers();

        let partial = self.root.join(format!("{PARTIAL_PREFIX}{install_id}"));
        let _partial_guard = RemoveOnDrop::new(partial.clone());
        on_event(InstallEvent::DownloadStarted {
            total_bytes: asset.size,
        });
        let digest = download(url, &partial, asset.size, cancel, on_event).await?;

        on_event(InstallEvent::Verifying);
        if digest != asset.sha256 {
            return Err(InstallError::Verify(format!(
                "SHA-256 mismatch for {}",
                asset.name
            )));
        }
        if cancel.is_cancelled() {
            return Err(InstallError::Cancelled);
        }

        on_event(InstallEvent::Installing);
        let staging = self.root.join(format!("{version}{TMP_SUFFIX}"));
        let staging_guard = RemoveOnDrop::new(staging.clone());
        let binary_name = asset.binary_name();
        {
            let partial = partial.clone();
            let staging = staging.clone();
            tokio::task::spawn_blocking(move || unpack(&partial, &staging, binary_name))
                .await
                .map_err(|err| InstallError::Io(err.to_string()))??;
        }
        let unpacked = staging.join(binary_name);
        {
            let unpacked = unpacked.clone();
            tokio::task::spawn_blocking(move || check_signature(&unpacked, signature))
                .await
                .map_err(|err| InstallError::Io(err.to_string()))??;
        }
        if cancel.is_cancelled() {
            return Err(InstallError::Cancelled);
        }
        let target = self.root.join(version.to_string());
        if target.exists() {
            std::fs::remove_dir_all(&target)?;
        }
        std::fs::rename(&staging, &target)?;
        staging_guard.disarm();
        self.prune(version);
        on_event(InstallEvent::Done {
            version: version.to_string(),
        });
        Ok(Installed {
            version: version.clone(),
            binary: target.join(binary_name),
        })
    }

    /// Delete every installed version (fails if one is running on Windows).
    pub fn remove_all(&self) -> Result<(), InstallError> {
        if !self.root.exists() {
            return Ok(());
        }
        let lock = self.lock()?;
        drop(lock);
        std::fs::remove_dir_all(&self.root)?;
        Ok(())
    }

    fn lock(&self) -> Result<File, InstallError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(LOCK_FILE))?;
        match super::process::try_lock_briefly(&file) {
            Ok(()) => Ok(file),
            Err(TryLockError::WouldBlock) => Err(InstallError::Busy),
            Err(TryLockError::Error(err)) => Err(err.into()),
        }
    }

    /// Partial downloads and staging folders of interrupted installs (called under the lock).
    fn clean_leftovers(&self) {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if name.starts_with(PARTIAL_PREFIX) {
                let _ = std::fs::remove_file(&path);
            } else if name.ends_with(TMP_SUFFIX) && path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }

    /// Keep `just_installed` and the newest other version; best effort (a running binary may
    /// not be deletable on Windows).
    fn prune(&self, just_installed: &Version) {
        let others = self
            .installed()
            .into_iter()
            .filter(|i| &i.version != just_installed);
        for old in others.skip(KEEP_VERSIONS - 1) {
            if let Some(dir) = old.binary.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// Only github.com and its asset host, over HTTPS; at most 5 redirects.
fn download_client() -> Result<reqwest::Client, InstallError> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let allowed = attempt.url().scheme() == "https"
            && attempt.url().host_str().is_some_and(|host| {
                host == "github.com" || host.ends_with(".githubusercontent.com")
            });
        if attempt.previous().len() >= 5 {
            attempt.error("too many redirects")
        } else if allowed {
            attempt.follow()
        } else {
            attempt.error("redirect to an unexpected host")
        }
    });
    reqwest::Client::builder()
        .redirect(policy)
        .user_agent(format!(
            "{}/{}",
            brand::PRODUCT_NAME,
            env!("CARGO_PKG_VERSION")
        ))
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|err| InstallError::Network(err.to_string()))
}

/// Stream `url` into `partial`, returning the lower-case hex SHA-256. Checks the size as it goes.
async fn download(
    url: &str,
    partial: &Path,
    expected_size: u64,
    cancel: &CancellationToken,
    on_event: &(dyn Fn(InstallEvent) + Send + Sync),
) -> Result<String, InstallError> {
    let client = download_client()?;
    let request = client.get(url).send();
    let mut response = tokio::select! {
        _ = cancel.cancelled() => return Err(InstallError::Cancelled),
        response = request => response.map_err(network)?,
    };
    if !response.status().is_success() {
        return Err(InstallError::Network(format!(
            "HTTP {}",
            response.status().as_u16()
        )));
    }
    let mut file = tokio::fs::File::create(partial).await?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut reported: u64 = 0;
    let step = (expected_size / 100).max(1 << 20);
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(InstallError::Cancelled),
            chunk = tokio::time::timeout(IDLE_TIMEOUT, response.chunk()) => chunk
                .map_err(|_| InstallError::Network("the download stalled".into()))?
                .map_err(network)?,
        };
        let Some(chunk) = chunk else { break };
        downloaded += chunk.len() as u64;
        if downloaded > expected_size {
            return Err(InstallError::Verify("larger than the pinned size".into()));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        if downloaded - reported >= step || downloaded == expected_size {
            reported = downloaded;
            on_event(InstallEvent::Progress {
                downloaded_bytes: downloaded,
                total_bytes: expected_size,
            });
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    if downloaded != expected_size {
        return Err(InstallError::Verify(format!(
            "{downloaded} bytes instead of {expected_size}"
        )));
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// A transport error without its URL (public, but logs keep codes, not addresses).
fn network(err: reqwest::Error) -> InstallError {
    InstallError::Network(err.without_url().to_string())
}

/// Decompress the zstd file into `<staging>/<binary_name>` (executable on Unix).
fn unpack(compressed: &Path, staging: &Path, binary_name: &str) -> Result<(), InstallError> {
    if staging.exists() {
        std::fs::remove_dir_all(staging)?;
    }
    std::fs::create_dir_all(staging)?;
    let source = BufReader::new(File::open(compressed)?);
    let decoder = ruzstd::decoding::StreamingDecoder::new(source)
        .map_err(|err| InstallError::Verify(format!("not a zstd file: {err}")))?;
    let target = staging.join(binary_name);
    let mut out = File::create(&target)?;
    let written = std::io::copy(&mut decoder.take(MAX_UNPACKED_BYTES + 1), &mut out)
        .map_err(|err| InstallError::Verify(format!("could not unpack: {err}")))?;
    if written > MAX_UNPACKED_BYTES {
        return Err(InstallError::Verify(
            "the unpacked file is too large".into(),
        ));
    }
    out.flush()?;
    out.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn check_signature(binary: &Path, rule: SignatureRule) -> Result<(), InstallError> {
    match rule {
        #[cfg(test)]
        SignatureRule::Skip => Ok(()),
        SignatureRule::OpenAi => check_openai_signature(binary),
    }
}

/// `codesign --verify --strict` against OpenAI's Developer ID team.
#[cfg(target_os = "macos")]
fn check_openai_signature(binary: &Path) -> Result<(), InstallError> {
    let requirement =
        format!("=anchor apple generic and certificate leaf[subject.OU] = \"{OPENAI_TEAM_ID}\"");
    let status = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "-R"])
        .arg(&requirement)
        .arg(binary)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(InstallError::Verify(
            "not signed by OpenAI's Developer ID".into(),
        ))
    }
}

/// A valid Authenticode signature whose signer is OpenAI (0.158.0's `codex.exe` is signed by
/// "OpenAI OpCo, LLC" through Microsoft's ID Verified code-signing CA; codex-pin.toml).
#[cfg(windows)]
fn check_openai_signature(binary: &Path) -> Result<(), InstallError> {
    match authenticode::verify(binary) {
        authenticode::Status::Valid { signer } if signer == OPENAI_SIGNER => Ok(()),
        authenticode::Status::Valid { .. } => Err(InstallError::Verify(
            "signed by someone other than OpenAI".into(),
        )),
        authenticode::Status::Unsigned => Err(InstallError::Verify("not signed".into())),
        authenticode::Status::Invalid(code) => Err(InstallError::Verify(format!(
            "invalid Authenticode signature (0x{code:08x})"
        ))),
    }
}

/// The Authenticode signer of OpenAI's Windows binaries.
#[cfg(windows)]
const OPENAI_SIGNER: &str = "OpenAI OpCo, LLC";

#[cfg(not(any(target_os = "macos", windows)))]
fn check_openai_signature(_binary: &Path) -> Result<(), InstallError> {
    Ok(())
}

#[cfg(windows)]
mod authenticode {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        TRUST_E_NOSIGNATURE, TRUST_E_PROVIDER_UNKNOWN, TRUST_E_SUBJECT_FORM_UNKNOWN,
    };
    use windows_sys::Win32::Security::Cryptography::{
        CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW,
    };
    use windows_sys::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
        WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY,
        WTD_UI_NONE, WTHelperGetProvCertFromChain, WTHelperGetProvSignerFromChain,
        WTHelperProvDataFromStateData, WinVerifyTrust,
    };

    pub(super) enum Status {
        Valid { signer: String },
        Unsigned,
        Invalid(u32),
    }

    pub(super) fn verify(path: &Path) -> Status {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut file = WINTRUST_FILE_INFO {
            cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: wide.as_ptr(),
            hFile: std::ptr::null_mut(),
            pgKnownSubject: std::ptr::null_mut(),
        };
        // SAFETY: an all-zero WINTRUST_DATA is valid; the fields WinVerifyTrust reads are set.
        let mut data: WINTRUST_DATA = unsafe { std::mem::zeroed() };
        data.cbStruct = size_of::<WINTRUST_DATA>() as u32;
        data.dwUIChoice = WTD_UI_NONE;
        data.fdwRevocationChecks = WTD_REVOKE_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous = WINTRUST_DATA_0 { pFile: &mut file };
        data.dwStateAction = WTD_STATEACTION_VERIFY;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        // SAFETY: `data` points at `file` and `wide`, which outlive both calls; no UI is shown
        // (a null window with WTD_UI_NONE).
        let status = unsafe {
            WinVerifyTrust(
                std::ptr::null_mut(),
                &mut action,
                (&mut data as *mut WINTRUST_DATA).cast(),
            )
        };
        let result = match status {
            // SAFETY: the state data of a successful verification, read before it is closed.
            0 => Status::Valid {
                signer: unsafe { signer_name(data.hWVTStateData) }.unwrap_or_default(),
            },
            TRUST_E_NOSIGNATURE | TRUST_E_SUBJECT_FORM_UNKNOWN | TRUST_E_PROVIDER_UNKNOWN => {
                Status::Unsigned
            }
            other => Status::Invalid(other as u32),
        };
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        // SAFETY: releases the state of the verification above.
        unsafe {
            WinVerifyTrust(
                std::ptr::null_mut(),
                &mut action,
                (&mut data as *mut WINTRUST_DATA).cast(),
            );
        }
        result
    }

    /// The simple display name (the CN) of the first signer's certificate.
    ///
    /// # Safety
    /// `state` must be the `hWVTStateData` of a successful, not yet closed verification.
    unsafe fn signer_name(state: windows_sys::Win32::Foundation::HANDLE) -> Option<String> {
        // SAFETY: per the contract above; every pointer is checked before use.
        unsafe {
            let provider = WTHelperProvDataFromStateData(state);
            if provider.is_null() {
                return None;
            }
            let signer = WTHelperGetProvSignerFromChain(provider, 0, 0, 0);
            if signer.is_null() {
                return None;
            }
            let cert = WTHelperGetProvCertFromChain(signer, 0);
            if cert.is_null() || (*cert).pCert.is_null() {
                return None;
            }
            let context = (*cert).pCert;
            let len = CertGetNameStringW(
                context,
                CERT_NAME_SIMPLE_DISPLAY_TYPE,
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            );
            if len <= 1 {
                return None;
            }
            let mut name = vec![0u16; len as usize];
            CertGetNameStringW(
                context,
                CERT_NAME_SIMPLE_DISPLAY_TYPE,
                0,
                std::ptr::null(),
                name.as_mut_ptr(),
                len,
            );
            Some(String::from_utf16_lossy(&name[..len as usize - 1]))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// What the loopback server answers to every request.
    #[derive(Clone)]
    enum Reply {
        Body(Vec<u8>),
        /// Sends the headers for `total` bytes and the first part, then nothing more.
        Stall {
            first: Vec<u8>,
            total: usize,
        },
        Redirect(String),
    }

    async fn serve(reply: Reply) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let reply = reply.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let (head, body): (String, Vec<u8>) = match &reply {
                        Reply::Body(body) => (
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            ),
                            body.clone(),
                        ),
                        Reply::Stall { first, total } => (
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
                            ),
                            first.clone(),
                        ),
                        Reply::Redirect(location) => (
                            format!(
                                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            ),
                            Vec::new(),
                        ),
                    };
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    if matches!(reply, Reply::Stall { .. }) {
                        tokio::time::sleep(Duration::from_secs(120)).await;
                    }
                });
            }
        });
        format!("http://{address}/codex-demo.zst")
    }

    /// A synthetic "binary" of `len` incompressible bytes (not Codex) and its zstd file.
    fn fake_binary(len: usize, seed: u8) -> (Vec<u8>, Vec<u8>) {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64 ^ u64::from(seed);
        let binary: Vec<u8> = (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect();
        let compressed = ruzstd::encoding::compress_to_vec(
            &binary[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        (binary, compressed)
    }

    fn asset_for(compressed: &[u8]) -> PinAsset {
        PinAsset {
            target: "aarch64-apple-darwin".into(),
            name: "codex-aarch64-apple-darwin.zst".into(),
            size: compressed.len() as u64,
            sha256: Sha256::digest(compressed)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        }
    }

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    /// Everything in the runtime folder except the lock file.
    fn entries(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|name| name != LOCK_FILE)
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    async fn install(
        runtime: &Runtime,
        v: &str,
        asset: &PinAsset,
        url: &str,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(InstallEvent) + Send + Sync),
    ) -> Result<Installed, InstallError> {
        runtime
            .install_from(
                &version(v),
                asset,
                url,
                SignatureRule::Skip,
                "install-1",
                cancel,
                on_event,
            )
            .await
    }

    #[tokio::test]
    async fn installs_a_verified_binary_and_keeps_two_versions() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(temp.path().join("runtimes/codex"));
        let (binary, compressed) = fake_binary(3 << 20, 7);
        let asset = asset_for(&compressed);
        let url = serve(Reply::Body(compressed.clone())).await;
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        let record = move |event: InstallEvent| seen.lock().unwrap().push(event);
        for v in ["0.157.1", "0.158.0", "0.158.1"] {
            let installed = install(
                &runtime,
                v,
                &asset,
                &url,
                &CancellationToken::new(),
                &record,
            )
            .await
            .unwrap();
            assert_eq!(installed.version, version(v));
            assert_eq!(std::fs::read(&installed.binary).unwrap(), binary);
        }
        // The newest two remain, newest first; nothing else is left behind.
        let versions: Vec<String> = runtime
            .installed()
            .iter()
            .map(|i| i.version.to_string())
            .collect();
        assert_eq!(versions, ["0.158.1", "0.158.0"]);
        assert_eq!(entries(runtime.root()), ["0.158.0", "0.158.1"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&runtime.installed()[0].binary)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        let events = events.lock().unwrap();
        let first_run: Vec<&InstallEvent> = events
            .iter()
            .take_while(|e| !matches!(e, InstallEvent::Done { .. }))
            .collect();
        assert_eq!(
            first_run[0],
            &InstallEvent::DownloadStarted {
                total_bytes: asset.size
            }
        );
        assert!(first_run.contains(&&InstallEvent::Progress {
            downloaded_bytes: asset.size,
            total_bytes: asset.size
        }));
        assert!(first_run.ends_with(&[&InstallEvent::Verifying, &InstallEvent::Installing]));
        assert!(events.contains(&InstallEvent::Done {
            version: "0.158.1".into()
        }));
        runtime.remove_all().unwrap();
        assert!(!runtime.root().exists());
    }

    #[tokio::test]
    async fn a_wrong_hash_size_or_format_installs_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(temp.path().join("codex"));
        let (_, compressed) = fake_binary(1 << 20, 1);
        let url = serve(Reply::Body(compressed.clone())).await;
        let quiet = |_: InstallEvent| {};
        let cancel = CancellationToken::new();

        let mut wrong_hash = asset_for(&compressed);
        wrong_hash.sha256 = "0".repeat(64);
        let err = install(&runtime, "0.158.0", &wrong_hash, &url, &cancel, &quiet)
            .await
            .unwrap_err();
        assert!(
            matches!(err, InstallError::Verify(ref m) if m.contains("SHA-256")),
            "{err}"
        );

        let mut too_small = asset_for(&compressed);
        too_small.size -= 1;
        let err = install(&runtime, "0.158.0", &too_small, &url, &cancel, &quiet)
            .await
            .unwrap_err();
        assert!(matches!(err, InstallError::Verify(_)), "{err}");

        let mut too_large = asset_for(&compressed);
        too_large.size += 1;
        let err = install(&runtime, "0.158.0", &too_large, &url, &cancel, &quiet)
            .await
            .unwrap_err();
        assert!(matches!(err, InstallError::Verify(_)), "{err}");

        let not_zstd = b"#!/bin/sh\necho not codex\n".to_vec();
        let url = serve(Reply::Body(not_zstd.clone())).await;
        let err = install(
            &runtime,
            "0.158.0",
            &asset_for(&not_zstd),
            &url,
            &cancel,
            &quiet,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, InstallError::Verify(ref m) if m.contains("zstd")),
            "{err}"
        );

        assert!(runtime.installed().is_empty());
        assert!(
            entries(runtime.root()).is_empty(),
            "{:?}",
            entries(runtime.root())
        );
    }

    #[tokio::test]
    async fn cancelling_mid_download_leaves_no_partial_file() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(temp.path().join("codex"));
        let (_, compressed) = fake_binary(4 << 20, 3);
        let asset = asset_for(&compressed);
        let url = serve(Reply::Stall {
            first: compressed[..compressed.len() / 2].to_vec(),
            total: compressed.len(),
        })
        .await;
        let cancel = CancellationToken::new();
        let on_progress = cancel.clone();
        let cancel_on_progress = move |event: InstallEvent| {
            if matches!(event, InstallEvent::Progress { .. }) {
                on_progress.cancel();
            }
        };
        let err = install(
            &runtime,
            "0.158.0",
            &asset,
            &url,
            &cancel,
            &cancel_on_progress,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, InstallError::Cancelled), "{err}");
        assert!(
            entries(runtime.root()).is_empty(),
            "{:?}",
            entries(runtime.root())
        );
    }

    #[tokio::test]
    async fn one_install_at_a_time_and_leftovers_are_cleaned() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(temp.path().join("codex"));
        let (_, compressed) = fake_binary(1 << 16, 5);
        let asset = asset_for(&compressed);
        let url = serve(Reply::Body(compressed)).await;
        let quiet = |_: InstallEvent| {};
        std::fs::create_dir_all(runtime.root()).unwrap();
        let held = runtime.lock().unwrap();
        let err = install(
            &runtime,
            "0.158.0",
            &asset,
            &url,
            &CancellationToken::new(),
            &quiet,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, InstallError::Busy), "{err}");
        drop(held);

        // An interrupted install's partial file and staging folder.
        std::fs::write(runtime.root().join(".partial-old"), b"half").unwrap();
        std::fs::create_dir_all(runtime.root().join("0.157.0.tmp")).unwrap();
        install(
            &runtime,
            "0.158.0",
            &asset,
            &url,
            &CancellationToken::new(),
            &quiet,
        )
        .await
        .unwrap();
        assert_eq!(entries(runtime.root()), ["0.158.0"]);

        let err = runtime
            .install_from(
                &version("0.158.0"),
                &asset,
                &url,
                SignatureRule::Skip,
                "../x",
                &CancellationToken::new(),
                &quiet,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, InstallError::Io(_)), "{err}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_unsigned_binary_fails_the_signature_check() {
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("codex");
        std::fs::write(&binary, fake_binary(1 << 12, 2).0).unwrap();
        let err = check_signature(&binary, SignatureRule::OpenAi).unwrap_err();
        assert!(matches!(err, InstallError::Verify(_)), "{err}");
    }

    /// For pin bumps, on macOS or Windows (the real binary isn't in the repository):
    /// `CODEX_SIGNED_BINARY=<unpacked codex> cargo test -p pagelamp-llm --lib -- --ignored signed`
    #[test]
    #[ignore = "needs a real Codex binary in CODEX_SIGNED_BINARY"]
    fn a_real_codex_is_signed_by_openai() {
        let binary = std::env::var_os("CODEX_SIGNED_BINARY").expect("CODEX_SIGNED_BINARY");
        check_signature(Path::new(&binary), SignatureRule::OpenAi).unwrap();
    }

    /// For pin bumps: the pure-Rust decoder unpacks a real asset.
    /// `CODEX_ZST=<asset.zst> cargo test -p pagelamp-llm --lib -- --ignored real_asset`
    #[test]
    #[ignore = "needs a real Codex asset in CODEX_ZST"]
    fn a_real_asset_unpacks() {
        let asset = std::env::var_os("CODEX_ZST").expect("CODEX_ZST");
        let temp = tempfile::tempdir().unwrap();
        unpack(Path::new(&asset), &temp.path().join("out"), "codex").unwrap();
        let unpacked = std::fs::read(temp.path().join("out/codex")).unwrap();
        println!(
            "unpacked {} bytes, sha256 {}",
            unpacked.len(),
            Sha256::digest(&unpacked)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
    }

    #[tokio::test]
    async fn redirects_go_only_to_github_over_https() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(temp.path().join("codex"));
        let (_, compressed) = fake_binary(1 << 16, 9);
        let elsewhere = serve(Reply::Body(compressed.clone())).await;
        let url = serve(Reply::Redirect(elsewhere)).await;
        let err = install(
            &runtime,
            "0.158.0",
            &asset_for(&compressed),
            &url,
            &CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap_err();
        assert!(matches!(err, InstallError::Network(_)), "{err}");
        assert!(entries(runtime.root()).is_empty());
    }
}
