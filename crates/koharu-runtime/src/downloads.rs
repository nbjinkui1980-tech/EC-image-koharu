//! HTTP download manager with retry logic for model and runtime files.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt, TryStreamExt};
use hf_hub::{
    Cache, Repo, RepoType,
    api::tokio::{ApiBuilder, Metadata},
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use koharu_core::events::{DownloadProgress, DownloadStatus};
use reqwest::header::{CONTENT_LENGTH, RANGE};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::broadcast;

use crate::runtime::{RuntimeHttpClient, RuntimeHttpConfig};

/// 10 MiB per ranged GET — same size hf-hub's `.high()` mode uses. Short enough
/// that reqwest's read_timeout catches a stalled connection quickly, and the
/// retry middleware can restart the chunk.
const CHUNK_SIZE: u64 = 10 * 1024 * 1024;

/// hf-hub's internal client has no read timeout, so we cap the metadata call
/// ourselves. The response body is a single byte — a short cap is safe.
const HF_METADATA_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Downloads — unified download manager
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Downloads {
    downloads_root: PathBuf,
    huggingface_cache: Cache,
    client: RuntimeHttpClient,
    tx: broadcast::Sender<DownloadProgress>,
    progress: Arc<MultiProgress>,
}

impl Downloads {
    pub(crate) fn new(
        downloads_root: PathBuf,
        huggingface_root: PathBuf,
        http: &RuntimeHttpConfig,
    ) -> Result<Self> {
        let client = http.build_client()?;

        Ok(Self {
            downloads_root,
            huggingface_cache: Cache::new(huggingface_root),
            client,
            tx: broadcast::channel(256).0,
            progress: Arc::new(MultiProgress::new()),
        })
    }

    pub fn client(&self) -> RuntimeHttpClient {
        Arc::clone(&self.client)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DownloadProgress> {
        self.tx.subscribe()
    }

    /// Download a HuggingFace model file, using the local cache first.
    ///
    /// hf-hub resolves URL + metadata + cache layout; the byte transfer runs
    /// on our retry-configured client so a stalled chunk is retried by the
    /// middleware instead of hanging the future.
    pub async fn huggingface_model(&self, repo: &str, filename: &str) -> Result<PathBuf> {
        let cache_repo = self
            .huggingface_cache
            .repo(Repo::new(repo.to_string(), RepoType::Model));

        if let Some(path) = cache_repo.get(filename) {
            return Ok(path);
        }

        let api = ApiBuilder::from_cache(self.huggingface_cache.clone())
            .with_progress(false)
            .with_user_agent("koharu", env!("CARGO_PKG_VERSION"))
            .build()
            .context("failed to build HF Hub API")?;
        let repo_handle = api.model(repo.to_string());
        let url = repo_handle.url(filename);

        let metadata: Metadata = tokio::time::timeout(HF_METADATA_TIMEOUT, api.metadata(&url))
            .await
            .map_err(|_| anyhow::anyhow!("HF metadata request timed out for `{repo}/{filename}`"))?
            .with_context(|| format!("failed to fetch HF metadata for `{repo}/{filename}`"))?;

        let blob_path = cache_repo.blob_path(metadata.etag());
        if let Some(parent) = blob_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create HF blob directory `{}`", parent.display())
            })?;
        }

        if !blob_path.exists() {
            let reporter = self.begin(filename);
            if let Err(error) = self
                .ranged_download(&url, &blob_path, &reporter, Some(metadata.size() as u64))
                .await
            {
                reporter.fail(&error);
                return Err(error.context(format!(
                    "failed to download HF model file `{repo}/{filename}`"
                )));
            }
            reporter.finish();
        }

        let pointer_dir = cache_repo.pointer_path(metadata.commit_hash());
        let pointer_path = pointer_dir.join(filename);
        if let Some(parent) = pointer_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        if !pointer_path.exists() {
            #[cfg(target_os = "windows")]
            std::os::windows::fs::symlink_file(&blob_path, &pointer_path).ok();
            #[cfg(target_family = "unix")]
            std::os::unix::fs::symlink(&blob_path, &pointer_path).ok();
        }
        cache_repo
            .create_ref(metadata.commit_hash())
            .context("failed to create HF cache ref")?;

        Ok(if pointer_path.exists() {
            pointer_path
        } else {
            blob_path
        })
    }

    /// Download a file to the downloads cache, returning the cached path.
    pub(crate) async fn cached_download(&self, url: &str, file_name: &str) -> Result<PathBuf> {
        let destination = self.downloads_root.join(file_name);
        if destination.exists() {
            return Ok(destination);
        }

        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create `{}`", parent.display()))?;
        }

        let reporter = self.begin(file_name);
        if let Err(error) = self
            .ranged_download(url, &destination, &reporter, None)
            .await
        {
            reporter.fail(&error);
            return Err(error);
        }
        reporter.finish();
        Ok(destination)
    }

    /// Digest-checked variant of `cached_download`: a cache hit must match
    /// `expected_sha256` (a mismatching cache is deleted and refetched), and
    /// a fresh download that fails verification is deleted instead of being
    /// returned — a bad download never clobbers a verified cache.
    #[allow(dead_code)] // wired into llama/zluda/cuda by AR09-T02/T03.
    pub(crate) async fn cached_download_with_sha256(
        &self,
        url: &str,
        file_name: &str,
        expected_sha256: &str,
    ) -> Result<PathBuf> {
        let destination = self.downloads_root.join(file_name);
        if destination.exists() {
            if verify_sha256(&destination, expected_sha256)? {
                return Ok(destination);
            }
            tokio::fs::remove_file(&destination).await.ok();
        }

        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create `{}`", parent.display()))?;
        }

        let reporter = self.begin(file_name);
        if let Err(error) = self
            .ranged_download(url, &destination, &reporter, None)
            .await
        {
            reporter.fail(&error);
            return Err(error);
        }
        reporter.finish();

        if !verify_sha256(&destination, expected_sha256)? {
            tokio::fs::remove_file(&destination).await.ok();
            anyhow::bail!("sha256 mismatch for `{file_name}` after download");
        }
        Ok(destination)
    }

    /// Stream a URL to `destination` as a set of ranged GETs running up to
    /// `chunk_parallelism()` in flight (defaults to the host's CPU core count).
    /// The temp file is pre-allocated to the full size so each worker can
    /// seek-and-write its range independently. Transient failures surface as
    /// `Err`; the retry middleware on `self.client` retries at the request
    /// level, and when retries are exhausted the whole download fails cleanly.
    async fn ranged_download(
        &self,
        url: &str,
        destination: &Path,
        reporter: &TransferReporter,
        total_hint: Option<u64>,
    ) -> Result<()> {
        let total = match total_hint {
            Some(t) => t,
            None => self.probe_content_length(url).await?,
        };
        reporter.start(Some(total));

        let temp = part_path(destination)?;
        tokio::fs::remove_file(&temp).await.ok();
        {
            let file = tokio::fs::File::create(&temp)
                .await
                .with_context(|| format!("failed to create `{}`", temp.display()))?;
            file.set_len(total)
                .await
                .with_context(|| format!("failed to preallocate `{}`", temp.display()))?;
        }

        let mut chunks = Vec::new();
        let mut start: u64 = 0;
        while start < total {
            let stop = (start + CHUNK_SIZE).min(total) - 1;
            chunks.push((start, stop));
            start = stop + 1;
        }

        let temp_ref: &Path = &temp;
        let write_result: Result<()> = stream::iter(chunks)
            .map(|(start, stop)| async move {
                let range = format!("bytes={start}-{stop}");
                let response = self
                    .client
                    .get(url)
                    .header(RANGE, &range)
                    .send()
                    .await
                    .with_context(|| format!("failed to fetch range {range} of `{url}`"))?
                    .error_for_status()
                    .with_context(|| format!("fetch failed for range {range} of `{url}`"))?;
                let bytes = response
                    .bytes()
                    .await
                    .with_context(|| format!("failed to read range {range} of `{url}`"))?;
                let mut file = tokio::fs::OpenOptions::new()
                    .write(true)
                    .open(temp_ref)
                    .await
                    .with_context(|| format!("failed to open `{}`", temp_ref.display()))?;
                file.seek(std::io::SeekFrom::Start(start))
                    .await
                    .with_context(|| format!("failed to seek in `{}`", temp_ref.display()))?;
                file.write_all(&bytes)
                    .await
                    .with_context(|| format!("failed to write `{}`", temp_ref.display()))?;
                file.flush()
                    .await
                    .with_context(|| format!("failed to flush `{}`", temp_ref.display()))?;
                reporter.advance(bytes.len());
                Ok::<_, anyhow::Error>(())
            })
            .buffer_unordered(crate::host_parallelism())
            .try_collect()
            .await;

        if let Err(err) = write_result {
            tokio::fs::remove_file(&temp).await.ok();
            return Err(err);
        }

        tokio::fs::remove_file(destination).await.ok();
        tokio::fs::rename(&temp, destination)
            .await
            .with_context(|| {
                format!(
                    "failed to rename `{}` → `{}`",
                    temp.display(),
                    destination.display()
                )
            })?;
        Ok(())
    }

    async fn probe_content_length(&self, url: &str) -> Result<u64> {
        let response = self
            .client
            .head(url)
            .send()
            .await
            .with_context(|| format!("failed to HEAD `{url}`"))?
            .error_for_status()
            .with_context(|| format!("HEAD failed for `{url}`"))?;

        let content_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .ok_or_else(|| anyhow::anyhow!("missing Content-Length for `{url}`"))?
            .to_str()
            .context("invalid Content-Length header")?;
        content_length
            .trim()
            .parse::<u64>()
            .with_context(|| format!("invalid Content-Length `{content_length}` for `{url}`"))
    }

    fn begin(&self, label: &str) -> TransferReporter {
        let bar = self.progress.add(ProgressBar::new_spinner());
        bar.enable_steady_tick(Duration::from_millis(120));
        bar.set_style(
            ProgressStyle::with_template(
                "{msg} [{elapsed_precise}] [{wide_bar}] {bytes}/{total_bytes} ({eta})",
            )
            .expect("progress style"),
        );
        bar.set_message(label.to_string());
        TransferReporter::new(self.tx.clone(), bar, label)
    }
}

// ---------------------------------------------------------------------------
// Transfer progress reporter
// ---------------------------------------------------------------------------

const UNKNOWN_TOTAL: u64 = u64::MAX;

#[derive(Clone)]
struct TransferReporter {
    tx: broadcast::Sender<DownloadProgress>,
    bar: ProgressBar,
    filename: Arc<str>,
    downloaded: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl TransferReporter {
    fn new(tx: broadcast::Sender<DownloadProgress>, bar: ProgressBar, label: &str) -> Self {
        Self {
            tx,
            bar,
            filename: Arc::<str>::from(label),
            downloaded: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(UNKNOWN_TOTAL)),
        }
    }

    fn start(&self, total: Option<u64>) {
        self.total
            .store(total.unwrap_or(UNKNOWN_TOTAL), Ordering::Relaxed);
        self.downloaded.store(0, Ordering::Relaxed);
        self.bar.set_length(total.unwrap_or(0));
        self.bar.set_position(0);
        self.emit(DownloadStatus::Started);
    }

    fn advance(&self, delta: usize) {
        self.downloaded.fetch_add(delta as u64, Ordering::Relaxed);
        self.bar.inc(delta as u64);
        self.emit(DownloadStatus::Downloading);
    }

    fn finish(&self) {
        self.bar.finish_and_clear();
        self.emit(DownloadStatus::Completed);
    }

    fn fail(&self, error: &anyhow::Error) {
        self.bar.finish_and_clear();
        self.emit(DownloadStatus::Failed {
            reason: error.to_string(),
        });
    }

    fn emit(&self, status: DownloadStatus) {
        let total = self.total.load(Ordering::Relaxed);
        let _ = self.tx.send(DownloadProgress {
            id: self.filename.to_string(),
            filename: self.filename.to_string(),
            downloaded: self.downloaded.load(Ordering::Relaxed),
            total: (total != UNKNOWN_TOTAL).then_some(total),
            status,
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Stream a file and compare its SHA-256 against the expected hex digest.
#[allow(dead_code)] // wired into llama/zluda/cuda by AR09-T02/T03.
pub(crate) fn verify_sha256(path: &Path, expected_hex: &str) -> Result<bool> {
    use sha2::Digest;
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("open `{}` for digest", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()) == expected_hex.to_ascii_lowercase())
}

fn part_path(destination: &Path) -> Result<PathBuf> {
    let file_name = destination.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "destination `{}` does not have a filename",
            destination.display()
        )
    })?;
    Ok(destination.with_file_name(format!("{}.part", file_name.to_string_lossy())))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::part_path;

    #[test]
    fn partial_download_path_appends_suffix() {
        let part = part_path(Path::new("/tmp/models/config.json")).unwrap();
        assert_eq!(part, Path::new("/tmp/models/config.json.part"));
    }
}

#[cfg(test)]
mod digest_tests {
    use std::path::Path;

    use super::{Downloads, verify_sha256};

    const GOOD_BYTES: &[u8] = b"koharu-digest-fixture";
    const BAD_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    fn downloads(root: &Path) -> Downloads {
        Downloads::new(
            root.join("downloads"),
            root.join("hf"),
            &crate::RuntimeHttpConfig::default(),
        )
        .expect("downloads")
    }

    // A minimal HTTP server answering HEAD (Content-Length) and ranged GETs
    // (206) so ranged_download can run against a local fixture.
    async fn spawn_range_server(bytes: &'static [u8]) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0u8; 4096];
                    let read = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..read]);
                    let response = if request.starts_with("HEAD") {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        )
                        .into_bytes()
                    } else {
                        let range = request
                            .lines()
                            .find(|line| line.to_ascii_lowercase().starts_with("range:"))
                            .and_then(|line| line.split_once("bytes="))
                            .map(|(_, spec)| spec.trim().to_string())
                            .unwrap_or_else(|| format!("0-{}", bytes.len() - 1));
                        let (start, stop) = range
                            .split_once('-')
                            .map(|(a, b)| {
                                (
                                    a.parse::<usize>().unwrap_or(0),
                                    b.parse::<usize>().unwrap_or(bytes.len() - 1),
                                )
                            })
                            .unwrap();
                        let body = &bytes[start..=stop.min(bytes.len() - 1)];
                        let mut response = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                            body.len(),
                            start,
                            start + body.len() - 1,
                            bytes.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(body);
                        response
                    };
                    let _ = stream.write_all(&response).await;
                });
            }
        });
        (port, task)
    }

    #[test]
    fn verify_sha256_matches_and_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.bin");
        std::fs::write(&path, GOOD_BYTES).unwrap();
        assert!(verify_sha256(&path, &sha256_hex(GOOD_BYTES)).unwrap());
        assert!(!verify_sha256(&path, BAD_SHA256).unwrap());
    }

    // AR09-T01 RED: a cache hit must be re-verified — a mismatched cache is
    // deleted and refetched, and a failed download never clobbers the cache.
    #[tokio::test]
    async fn cached_hit_with_mismatched_digest_redownloads() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = downloads(dir.path());
        let cache = dir.path().join("downloads");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("fixture.bin"), b"corrupt").unwrap();
        let (port, server) = spawn_range_server(GOOD_BYTES).await;

        let path = downloads
            .cached_download_with_sha256(
                &format!("http://127.0.0.1:{port}/fixture.bin"),
                "fixture.bin",
                &sha256_hex(GOOD_BYTES),
            )
            .await
            .expect("redownload with the correct bytes");
        server.abort();

        assert_eq!(std::fs::read(&path).unwrap(), GOOD_BYTES);
    }

    #[tokio::test]
    async fn failed_download_never_lands_in_cache() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = downloads(dir.path());
        let (port, server) = spawn_range_server(GOOD_BYTES).await;

        let result = downloads
            .cached_download_with_sha256(
                &format!("http://127.0.0.1:{port}/fixture.bin"),
                "fixture.bin",
                BAD_SHA256,
            )
            .await;
        server.abort();

        assert!(result.is_err(), "digest mismatch after download must fail");
        assert!(
            !dir.path().join("downloads").join("fixture.bin").exists(),
            "a failed download must not land in the cache"
        );
    }

    // Lock: a cache hit with a matching digest returns without any network.
    #[tokio::test]
    async fn cached_hit_with_matching_digest_returns_cache() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = downloads(dir.path());
        let cache = dir.path().join("downloads");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("fixture.bin"), GOOD_BYTES).unwrap();

        let path = downloads
            .cached_download_with_sha256(
                "http://127.0.0.1:1/unreachable",
                "fixture.bin",
                &sha256_hex(GOOD_BYTES),
            )
            .await
            .expect("matching cache hit returns without network");
        assert_eq!(std::fs::read(&path).unwrap(), GOOD_BYTES);
    }
}
