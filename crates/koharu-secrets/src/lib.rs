mod platform;
mod store;

pub use store::SecretStore;

#[cfg(all(target_os = "macos", debug_assertions))]
pub const DEFAULT_SECRET_SERVICE: &str = "koharu-dev";

#[cfg(not(all(target_os = "macos", debug_assertions)))]
pub const DEFAULT_SECRET_SERVICE: &str = "koharu";

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(all(target_os = "macos", debug_assertions))]
    fn macos_debug_uses_isolated_secret_service() {
        assert_eq!(super::DEFAULT_SECRET_SERVICE, "koharu-dev");
    }
}
