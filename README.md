# cactup-ecosystem-common

Code shared across the Cactup ecosystem: the `cactup` CLI and the Cactarium registry.

- `sim`: `.sim` files, a parfile plus the thornlist it ran on (both verbatim) and per-repo pins
- `manifest`: `cactarium.toml` (packages) and `cactarium-stack.toml` (stacks), read from repos at each tag
- `stack`: stack definitions (flat thornlist, recipe, or both) and frozen releases
- `name`, `slug`, `version`: `owner/name[/Thorn][@...]` references and track-namespaced semver
- `crl`: the CRL 1.0 thornlist parser (GetComponents-compatible, plus `!REPO_COMMIT`)
- `ccl`: dependency facts from a thorn's `interface.ccl` and `configuration.ccl`

Pure Rust with no network or system-library dependencies, so `cactup` can link it into a static
MUSL binary. Depend on it by git revision:

```toml
cactup-ecosystem-common = { git = "https://github.com/max-morris/cactup-ecosystem-common", rev = "..." }
```
