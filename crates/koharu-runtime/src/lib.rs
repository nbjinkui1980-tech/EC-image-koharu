mod archive;
mod cuda;
pub mod downloads;
mod install;
mod llama;
mod loader;
pub mod packages;
mod runtime;
mod zluda;

pub use cuda::{
    CudaDriverVersion, check_cuda_driver_support, compute_capability,
    driver_version as nvidia_driver_version,
};
pub use hf_hub;
pub use inventory;
pub use loader::{load_library_by_name, load_library_by_path};
pub use packages::{PackageCatalog as Catalog, PackageFuture, PackageKind, PackageRegistration};
pub use runtime::{
    ComputePolicy, Runtime, RuntimeHttpClient, RuntimeHttpConfig, RuntimeManager,
    default_app_data_root,
};
pub use zluda::zluda_active;

/// Number of worker threads suitable for host-bound parallel work.
///
/// Sandboxed environments may not expose their parallelism. Keep the old
/// `num_cpus` contract of always returning at least one worker.
pub fn host_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_parallelism_is_never_zero() {
        assert!(super::host_parallelism() >= 1);
    }
}
