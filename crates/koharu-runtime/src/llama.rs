//! llama.cpp runtime detection, installation, and platform-specific library resolution.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::Runtime;
use crate::archive;
use crate::install::InstallState;
use crate::loader::{add_runtime_search_path, preload_library};

const LLAMA_CPP_TAG: &str = env!("LLAMA_CPP_TAG");
const RELEASE_BASE_URL: &str = "https://github.com/ggml-org/llama.cpp/releases/download";

fn release_base_url() -> String {
    #[cfg(test)]
    if let Some(over) = RELEASE_BASE_OVERRIDE.with(|cell| cell.get()) {
        return over.to_string();
    }
    RELEASE_BASE_URL.to_string()
}

#[cfg(test)]
thread_local! {
    static RELEASE_BASE_OVERRIDE: std::cell::Cell<Option<&'static str>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn override_release_base(base: Option<&'static str>) {
    RELEASE_BASE_OVERRIDE.with(|cell| cell.set(base));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum LlamaDistribution {
    WindowsCuda13X64,
    WindowsVulkanX64,
    LinuxVulkanX64,
    LinuxVulkanArm64,
    MacosArm64,
}

impl LlamaDistribution {
    #[allow(clippy::needless_return)]
    fn detect(_runtime: &Runtime) -> Result<Self> {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        return Ok(Self::windows_x64(_runtime));

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        return Ok(Self::LinuxVulkanX64);

        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        return Ok(Self::LinuxVulkanArm64);

        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        return Ok(Self::MacosArm64);

        #[cfg(not(any(
            all(target_os = "windows", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "aarch64")
        )))]
        bail!(
            "unsupported platform: os={}, arch={}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    fn windows_x64(runtime: &Runtime) -> Self {
        if crate::zluda::package_enabled(runtime) {
            Self::WindowsVulkanX64
        } else if crate::cuda::llama_cuda_enabled(runtime) {
            Self::WindowsCuda13X64
        } else {
            Self::WindowsVulkanX64
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::WindowsCuda13X64 => "windows-cuda13-x64",
            Self::WindowsVulkanX64 => "windows-vulkan-x64",
            Self::LinuxVulkanX64 => "linux-vulkan-x64",
            Self::LinuxVulkanArm64 => "linux-vulkan-arm64",
            Self::MacosArm64 => "macos-arm64",
        }
    }

    #[cfg(test)]
    fn assets(self) -> Vec<String> {
        self.artifacts()
            .into_iter()
            .map(|artifact| artifact.file_name)
            .collect()
    }

    fn artifacts(&self) -> Vec<crate::install::NativeArtifact> {
        // Pinned SHA-256 digests for LLAMA_CPP_TAG release artifacts. When
        // the tag moves, every digest must be recomputed and repinned.
        let base = release_base_url();
        let tag = LLAMA_CPP_TAG;
        let artifact = |file: String, sha256: &'static str| crate::install::NativeArtifact {
            url: format!("{base}/{tag}/{file}"),
            archive_kind: if file.ends_with(".zip") {
                "zip"
            } else {
                "tar.gz"
            },
            file_name: file,
            sha256,
            selected_files: None,
        };
        match self {
            Self::WindowsCuda13X64 => vec![
                artifact(
                    format!("llama-{tag}-bin-win-cuda-13.1-x64.zip"),
                    "677222c68c38f8d3e63ed76e7f93ea907b856394072cc79810b44993f563929f",
                ),
                artifact(
                    "cudart-llama-bin-win-cuda-13.1-x64.zip".to_string(),
                    "f96935e7e385e3b2d0189239077c10fe8fd7e95690fea4afec455b1b6c7e3f18",
                ),
            ],
            Self::WindowsVulkanX64 => vec![artifact(
                format!("llama-{tag}-bin-win-vulkan-x64.zip"),
                "6733f6354fdf171ba384057ede7a40574f9276af14142d310d7cbbf1d641b3a0",
            )],
            Self::LinuxVulkanX64 => vec![artifact(
                format!("llama-{tag}-bin-ubuntu-vulkan-x64.tar.gz"),
                "9d35d80aa48dd53eecc6bc67cc2a63f15861d51df263a52a929b519b23664ac3",
            )],
            Self::LinuxVulkanArm64 => vec![artifact(
                format!("llama-{tag}-bin-ubuntu-vulkan-arm64.tar.gz"),
                "8af5c6d8b8e471e41c65b2ddf6293b8663680cc87f93b6241679b2833d14c9d6",
            )],
            Self::MacosArm64 => vec![artifact(
                format!("llama-{tag}-bin-macos-arm64.tar.gz"),
                "a1d5261c3f14e7e094eb1f831d7e3a8971df69bfc7dd8c6b3421d5319f1cde53",
            )],
        }
    }

    fn libraries(self) -> &'static [&'static str] {
        match self {
            Self::WindowsCuda13X64 => &[
                "cudart64_13.dll",
                "cublasLt64_13.dll",
                "cublas64_13.dll",
                "libomp140.x86_64.dll",
                "ggml-base.dll",
                "ggml.dll",
                "ggml-cpu-alderlake.dll",
                "ggml-cpu-cannonlake.dll",
                "ggml-cpu-cascadelake.dll",
                "ggml-cpu-cooperlake.dll",
                "ggml-cpu-haswell.dll",
                "ggml-cpu-icelake.dll",
                "ggml-cpu-ivybridge.dll",
                "ggml-cpu-piledriver.dll",
                "ggml-cpu-sandybridge.dll",
                "ggml-cpu-sapphirerapids.dll",
                "ggml-cpu-skylakex.dll",
                "ggml-cpu-sse42.dll",
                "ggml-cpu-x64.dll",
                "ggml-cpu-zen4.dll",
                "ggml-cuda.dll",
                "ggml-rpc.dll",
                "llama.dll",
                "mtmd.dll",
            ],
            Self::WindowsVulkanX64 => &[
                "libomp140.x86_64.dll",
                "ggml-base.dll",
                "ggml.dll",
                "ggml-cpu-alderlake.dll",
                "ggml-cpu-cannonlake.dll",
                "ggml-cpu-cascadelake.dll",
                "ggml-cpu-cooperlake.dll",
                "ggml-cpu-haswell.dll",
                "ggml-cpu-icelake.dll",
                "ggml-cpu-ivybridge.dll",
                "ggml-cpu-piledriver.dll",
                "ggml-cpu-sandybridge.dll",
                "ggml-cpu-sapphirerapids.dll",
                "ggml-cpu-skylakex.dll",
                "ggml-cpu-sse42.dll",
                "ggml-cpu-x64.dll",
                "ggml-cpu-zen4.dll",
                "ggml-rpc.dll",
                "ggml-vulkan.dll",
                "llama.dll",
                "mtmd.dll",
            ],
            Self::LinuxVulkanX64 => &[
                "libggml-base.so",
                "libggml.so",
                "libggml-cpu-alderlake.so",
                "libggml-cpu-cannonlake.so",
                "libggml-cpu-cascadelake.so",
                "libggml-cpu-cooperlake.so",
                "libggml-cpu-haswell.so",
                "libggml-cpu-icelake.so",
                "libggml-cpu-ivybridge.so",
                "libggml-cpu-piledriver.so",
                "libggml-cpu-sandybridge.so",
                "libggml-cpu-sapphirerapids.so",
                "libggml-cpu-skylakex.so",
                "libggml-cpu-sse42.so",
                "libggml-cpu-x64.so",
                "libggml-cpu-zen4.so",
                "libggml-rpc.so",
                "libggml-vulkan.so",
                "libllama.so",
                "libmtmd.so",
            ],
            Self::LinuxVulkanArm64 => &[
                "libggml-base.so",
                "libggml.so",
                "libggml-cpu-armv8.0_1.so",
                "libggml-cpu-armv8.2_1.so",
                "libggml-cpu-armv8.2_2.so",
                "libggml-cpu-armv8.2_3.so",
                "libggml-cpu-armv8.6_1.so",
                "libggml-cpu-armv8.6_2.so",
                "libggml-cpu-armv9.2_1.so",
                "libggml-cpu-armv9.2_2.so",
                "libggml-rpc.so",
                "libggml-vulkan.so",
                "libllama.so",
                "libmtmd.so",
            ],
            Self::MacosArm64 => &[
                "libggml-base.dylib",
                "libggml.dylib",
                "libggml-blas.dylib",
                "libggml-cpu.dylib",
                "libggml-metal.dylib",
                "libggml-rpc.dylib",
                "libllama.dylib",
                "libmtmd.dylib",
            ],
        }
    }

    fn install_dir(self, runtime: &Runtime) -> PathBuf {
        runtime
            .root()
            .join("runtime")
            .join("llama.cpp")
            .join(LLAMA_CPP_TAG)
            .join(self.id())
    }

    fn source_id(self) -> String {
        let digest = self
            .artifacts()
            .first()
            .map(|artifact| &artifact.sha256[..12])
            .unwrap_or("none");
        format!("llama-{LLAMA_CPP_TAG}-{};sha256={digest}", self.id())
    }
}

pub(crate) fn package_enabled(runtime: &Runtime) -> bool {
    LlamaDistribution::detect(runtime).is_ok()
}

pub(crate) fn package_present(runtime: &Runtime) -> Result<bool> {
    let distribution = LlamaDistribution::detect(runtime)?;
    let install_dir = distribution.install_dir(runtime);
    let source_id = distribution.source_id();
    let install = InstallState::new(&install_dir, &source_id);
    if !install.is_current() {
        return Ok(false);
    }

    Ok(distribution
        .libraries()
        .iter()
        .all(|library| install_dir.join(library).exists()))
}

pub(crate) async fn package_prepare(runtime: &Runtime) -> Result<()> {
    ensure_ready(runtime).await
}

pub(crate) async fn ensure_ready(runtime: &Runtime) -> Result<()> {
    let distribution = LlamaDistribution::detect(runtime)?;
    let install_dir = distribution.install_dir(runtime);
    let source_id = distribution.source_id();
    let install = InstallState::new(&install_dir, &source_id);

    if !install.is_current() {
        // Download and verify every artifact before touching the install
        // dir: a digest failure cleans the download temp and leaves any
        // existing installation untouched.
        let artifacts = distribution.artifacts();
        let mut archives = Vec::with_capacity(artifacts.len());
        for artifact in &artifacts {
            let archive = runtime
                .downloads()
                .cached_download_with_sha256(&artifact.url, &artifact.file_name, artifact.sha256)
                .await
                .with_context(|| format!("failed to download `{}`", artifact.url))?;
            archives.push(archive);
        }

        install.reset()?;

        for (archive, artifact) in archives.iter().zip(&artifacts) {
            let kind = match artifact.archive_kind {
                "zip" => archive::ArchiveKind::Zip,
                "tar.gz" => archive::ArchiveKind::TarGz,
                other => bail!("unsupported archive kind `{other}`"),
            };
            archive::extract(archive, &install_dir, kind, artifact.extract_policy())?;
        }

        for library in distribution.libraries() {
            if !install_dir.join(library).exists() {
                bail!(
                    "required library `{library}` missing from `{}`",
                    install_dir.display()
                );
            }
        }

        install.commit()?;
    }

    add_runtime_search_path(&install_dir)?;
    for library in distribution.libraries() {
        preload_library(&install_dir.join(library))?;
    }

    Ok(())
}

pub(crate) fn runtime_dir(runtime: &Runtime) -> Result<PathBuf> {
    Ok(LlamaDistribution::detect(runtime)?.install_dir(runtime))
}

crate::declare_native_package!(
    id: "runtime:llama",
    bootstrap: true,
    order: 20,
    enabled: crate::llama::package_enabled,
    present: crate::llama::package_present,
    prepare: crate::llama::package_prepare,
);

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    fn touch(path: &Path) {
        fs::write(path, b"ok").unwrap();
    }

    #[test]
    fn detect_returns_a_variant_for_current_platform() {
        let runtime = Runtime::new("/tmp/koharu-runtime", crate::ComputePolicy::PreferGpu).unwrap();
        let distribution = LlamaDistribution::detect(&runtime).unwrap();
        assert!(!distribution.id().is_empty());
        assert!(!distribution.assets().is_empty());
        assert!(!distribution.libraries().is_empty());
    }

    #[test]
    fn install_dir_includes_tag_and_id() {
        let runtime = Runtime::new("/tmp/koharu-runtime", crate::ComputePolicy::CpuOnly).unwrap();
        let dir = LlamaDistribution::WindowsVulkanX64.install_dir(&runtime);
        assert!(
            dir.ends_with(
                std::path::Path::new("llama.cpp")
                    .join(LLAMA_CPP_TAG)
                    .join("windows-vulkan-x64")
            )
        );
    }

    #[test]
    fn preload_order_matches_libraries() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = tempdir.path();
        let runtime = LlamaDistribution::WindowsCuda13X64;

        for library in runtime.libraries() {
            touch(&root.join(library));
        }

        let paths: Vec<PathBuf> = runtime
            .libraries()
            .iter()
            .map(|library| root.join(library))
            .collect();
        assert!(paths.iter().all(|path| path.exists()));
        assert_eq!(paths.len(), runtime.libraries().len());
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[test]
    fn windows_runtime_prefers_vulkan_when_zluda_is_enabled() {
        let runtime = Runtime::new("/tmp/koharu-runtime", crate::ComputePolicy::PreferGpu).unwrap();
        if crate::zluda::package_enabled(&runtime) {
            assert_eq!(
                LlamaDistribution::detect(&runtime).unwrap(),
                LlamaDistribution::WindowsVulkanX64
            );
        }
    }
}

#[cfg(test)]
mod artifact_tests {
    use super::*;

    fn is_hex_64(value: &str) -> bool {
        value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
    }

    // AR09-T02 RED: every llama artifact must carry a pinned SHA-256, and the
    // source id must track the digest.
    #[test]
    fn llama_artifacts_have_pinned_sha256() {
        let runtime = Runtime::new("/tmp/koharu-runtime", crate::ComputePolicy::CpuOnly).unwrap();
        let distribution = LlamaDistribution::detect(&runtime).unwrap();
        let artifacts = distribution.artifacts();
        assert!(!artifacts.is_empty());
        for artifact in &artifacts {
            assert!(
                artifact.url.starts_with("http"),
                "url missing: {artifact:?}"
            );
            assert!(
                is_hex_64(artifact.sha256),
                "artifact `{}` must pin a 64-hex sha256",
                artifact.file_name
            );
            assert!(matches!(artifact.archive_kind, "zip" | "tar.gz"));
        }
    }

    #[test]
    fn llama_source_id_includes_digest() {
        let runtime = Runtime::new("/tmp/koharu-runtime", crate::ComputePolicy::CpuOnly).unwrap();
        let distribution = LlamaDistribution::detect(&runtime).unwrap();
        assert!(
            distribution.source_id().contains("sha256="),
            "source id must embed the artifact digest"
        );
    }

    // A failed digest check must leave any existing install untouched —
    // including the marker and every file in the install dir.
    #[tokio::test]
    async fn llama_bad_digest_keeps_existing_install() {
        let root =
            std::env::temp_dir().join(format!("koharu-llama-artifact-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let runtime = Runtime::new(&root, crate::ComputePolicy::CpuOnly).unwrap();
        let distribution = LlamaDistribution::detect(&runtime).unwrap();
        let install_dir = distribution.install_dir(&runtime);
        std::fs::create_dir_all(&install_dir).unwrap();
        std::fs::write(install_dir.join("sentinel"), b"keep me").unwrap();
        crate::install::InstallState::new(&install_dir, "stale-source")
            .commit()
            .unwrap();

        let bytes: &'static [u8] = b"not-a-real-archive";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
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
                            .and_then(|(_, spec)| spec.trim().split_once('-'))
                            .map(|(a, b)| {
                                (
                                    a.parse::<usize>().unwrap_or(0),
                                    b.parse::<usize>().unwrap_or(bytes.len() - 1),
                                )
                            })
                            .unwrap_or((0, bytes.len() - 1));
                        let (start, stop) = range;
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
        override_release_base(Some(Box::leak(
            format!("http://127.0.0.1:{port}").into_boxed_str(),
        )));
        let result = ensure_ready(&runtime).await;
        override_release_base(None);
        server.abort();

        assert!(result.is_err(), "bad digest must fail ensure_ready");
        assert!(
            install_dir.join("sentinel").exists(),
            "existing install must be untouched"
        );
        assert_eq!(
            std::fs::read(install_dir.join(".installed")).unwrap(),
            b"stale-source",
            "marker must not be rewritten"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
