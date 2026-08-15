//! Provider base-URL authority parsing and comparison.
//!
//! Single entry point for deciding whether two provider URLs share an
//! authority (scheme + host + effective port). Paths are intentionally
//! excluded so a path-only change does not invalidate provider secrets.

use url::Url;

/// Parsed authority of a provider base URL: scheme + host + effective port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    scheme: String,
    host: String,
    port: u16,
}

/// Parse a provider base URL into its authority.
///
/// Rejects non-HTTP(S) schemes, userinfo, and fragments.
pub fn provider_authority(raw: &str) -> anyhow::Result<Authority> {
    let url = Url::parse(raw.trim())
        .map_err(|err| anyhow::anyhow!("invalid provider base URL: {err}"))?;

    match url.scheme() {
        "http" | "https" => {}
        scheme => anyhow::bail!("provider base URL must use http or https, got `{scheme}`"),
    }
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("provider base URL must not contain userinfo");
    }
    if url.fragment().is_some() {
        anyhow::bail!("provider base URL must not contain a fragment");
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("provider base URL must include a host"))?
        .to_string();
    let port = url
        .port_or_known_default()
        .expect("http and https always have a known default port");

    Ok(Authority {
        scheme: url.scheme().to_string(),
        host,
        port,
    })
}

#[cfg(test)]
mod tests {
    use super::provider_authority;

    #[test]
    fn authority_rejects_non_http_scheme() {
        assert!(provider_authority("ftp://example.com").is_err());
    }

    #[test]
    fn authority_rejects_userinfo() {
        assert!(provider_authority("http://user:pass@host/v1").is_err());
    }

    #[test]
    fn authority_rejects_fragment() {
        assert!(provider_authority("http://host/v1#frag").is_err());
    }

    #[test]
    fn authority_effective_port() {
        assert_eq!(
            provider_authority("https://h").unwrap(),
            provider_authority("https://h:443").unwrap()
        );
        assert_eq!(
            provider_authority("http://h:80").unwrap(),
            provider_authority("http://h").unwrap()
        );
        assert_ne!(
            provider_authority("http://h").unwrap(),
            provider_authority("https://h").unwrap()
        );
    }

    #[test]
    fn authority_same_path_insensitive() {
        assert_eq!(
            provider_authority("http://h:8080/v1").unwrap(),
            provider_authority("http://h:8080/v2/api").unwrap()
        );
    }
}
