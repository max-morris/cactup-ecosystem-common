# cactup-ecosystem-common

Code shared across the Cactup ecosystem: the `cactup` CLI and the Cactarium registry.

- `sim`: `.sim` files, a parfile plus the thornlist it ran on (both verbatim) and per-repo pins
- `manifest`: `cactarium.toml` (packages), `cactarium-stack.toml` (stacks) and `cactarium-sim.toml`
  (sims, one or several per repo), read from repos at each tag
- `stack`: stack definitions (flat thornlist, recipe, or both) and frozen releases
- `name`, `slug`, `version`: `owner/name[/Thorn|/sim][@...]` references and track-namespaced semver
- `crl`: the CRL 1.0 thornlist parser (GetComponents-compatible, plus `!REPO_COMMIT`)
- `ccl`: dependency facts from a thorn's `interface.ccl` and `configuration.ccl`, and the hard
  requirements they make (`Need`, `needs_of`, `meets`), read the same way by the Cactarium and CactUp
- `api`: the Cactarium HTTP API's JSON shapes and headers
- `url`: repo URLs compared as one repo whatever their spelling (`normalize_repo_url`)

Pure Rust with no network or system-library dependencies, so `cactup` can link it into a static
MUSL binary. Depend on it by git revision:

```toml
cactup-ecosystem-common = { git = "https://github.com/max-morris/cactup-ecosystem-common", rev = "..." }
```

Design notes: [docs/dependencies.md](docs/dependencies.md) (dependencies, forks and resolution),
[docs/multi-sim.md](docs/multi-sim.md) (sims from a repo, one or several, and how each is addressed)
and [docs/api.md](docs/api.md) (the Cactarium API).
