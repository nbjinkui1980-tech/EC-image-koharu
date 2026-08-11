# AR01 R03B Host Origin CORS Scope Card

**Status:** NOT EXECUTABLE — starts only after the R03A commit.

**Goal:** Enforce structural Host/Origin validation and exact CORS behavior outside the complete API/MCP/static router.

## Frozen boundary

- Files: `crates/koharu-rpc/src/security.rs`, `crates/koharu-rpc/src/server.rs`, `crates/koharu-rpc/tests/origin_host.rs`.
- One real-listener RED suite covering Host, URI authority, Origin, preflight, `Vary`, MCP, static assets, and the Desktop session-assets serving constructor.
- Only HTTP(S) request schemes; malformed, duplicate, absent, conflicting, userinfo, invalid port, and invalid bracketed IPv6 authority fail closed.
- Order: Host → Origin/CORS → auth → readiness → handler.
- R03A behavior remains unchanged.
- Lifecycle: RED → minimal GREEN → one independent review → one commit.

No R03B implementation or detailed snippet is authorized by this scope card. Draft its short execution card only after R03A commits.
