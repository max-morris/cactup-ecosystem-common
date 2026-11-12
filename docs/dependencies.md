# Dependencies, forks and resolution

What a package, stack or sim declares about other packages, what each declaration means, and how
CactUp resolves them. The Cactarium and CactUp both implement this; the types live in this crate
(`manifest.rs`: `Dependency`, `ForkOf`, `Claim`; `version.rs`: `TrackedReq`).

## Principles

1. **CCL says what a thorn needs; a dependency says where it comes from.** A thorn's CCL already
   names the implementations it inherits or befriends, the capabilities and thorns it requires,
   and the functions it requires. A `[[dependency]]` never repeats that. It names the package
   (and versions) that meets it, and, where CCL can't, which thorns of that package.
2. **Identity is a package plus a thorn, never a bare name.** `et/carpetx`'s `ADMBaseX` and a
   fork's `ADMBaseX` are different things. Same-named thorns from different packages are not
   assumed to be related, let alone interchangeable.
3. **Nothing substitutes across packages unless a declaration says so.** The only declarations
   that do are a `[[fork-of]]` the user chooses to install, and, if ever added, an explicit
   `any-of` on a dependency. In particular, two packages that provide the same CCL
   implementation (Carpet and PUGH both implement `Hyperslab`) are never swapped for each other.
4. **Generated declarations crystallize a working setup.** When CactUp writes a manifest from an
   installation, every dependency names the package that actually met the requirement there, so
   a crystallized setup never says "any provider".
5. **Escape hatches are first-class.** The Cactus community's version and dependency information
   is often wrong or missing; every resolver refusal names the exact way past it.

## Declarations

### `[[dependency]]` (packages and stacks)

```toml
[[dependency]]
target = "et/carpetx@^2026.5"   # owner/name, owner/name@<range>, or owner/name#<sha>
needs = ["CarpetX"]              # thorns of the target needed whatever the CCL says (optional)
needed-by = ["Z4c"]              # thorns of this package the dependency is for (optional; default all)
note = "CarpetX as the driver"
```

- `target` without `@` means any version on the target's default track. Ranges use the npm
  syntax of `version.rs` (`^`, `~`, comparators, spaces for AND, hyphen ranges, `||` for OR; a
  track prefix on every alternative or on none). `#<sha>` pins one commit.
- `needs` empty: whichever of the target's thorns the `needed-by` thorns' CCL requirements
  resolve to. Given: those thorns are needed regardless, which covers what CCL can't express (a
  header included by path) and chooses among providers (both Carpet and CarpetX implement
  `Driver`).
- `needed-by` names thorns this package publishes; a version naming one it doesn't is invalid.
  Only a package has thorns, so a stack can't use it.
- `optional = true` is reserved for a later version.

### `[[fork-of]]` (packages)

```toml
[[fork-of]]
target = "et/carpet@~2026.5"     # owner/name, owner/name@<range>, or owner/name#<sha>
note = "subcycling in time"
```

The package is a drop-in replacement for the target at the versions `target` names (a bare
`owner/name`: every version on the default track).

- **When a fork satisfies a dependency on its upstream** (`ForkOf::stands_in_for`): when some
  published version of the upstream is both one the dependency accepts and one the fork
  replaces. A sim's exact pin (`et/carpet@2026.5.1`) is satisfied when the fork's range contains
  it. A dependency on one commit (`#sha`) is satisfied only by a fork of that commit.
- A fork and its upstream are never installed together (they share thorn names).
- A fork is used only when the user chooses it (`cactup add steve/carpet`, which then replaces
  `et/carpet`), never on the resolver's own initiative. Its dependencies then stand in for the
  upstream's.
- The Cactarium warns reviewers when a fork lacks thorns that the newest upstream version in its
  range has: it isn't a full drop-in then.

### `[[works-with]]`, `[[incompatible-with]]`

Displayed claims with the same `target` forms. The resolver treats `incompatible-with` as a hard
conflict and `works-with` as information.

### Sims: `[[uses]]` (planned, sim format 4)

A sim pins every repo already; `[[uses]]` records which registry package and stack versions its
repos are (`et/carpet@2026.5.0`), so the Cactarium can show what a sim is built on and the
resolver can swap in a fork for one of them.

## Resolution (CactUp)

Runs on `cactup add`, `cactup install`, and upgrades. Client-side: the Cactarium supplies data
(versions, their dependencies and fork-of declarations, and the per-thorn CCL facts it already
indexes).

1. **What is installed stays fixed** unless `--upgrade` is given.
2. **Transitive closure** over `[[dependency]]`: each dependency's range is intersected with what
   is already chosen; tracks are honored; `#sha` pins are exact.
3. **CCL requirements no dependency covers** are met by what is installed. If nothing installed
   meets one and several packages could, that is an error naming the candidates and the claim or
   flag that picks one. There is no built-in table of default providers: manifests are expected to
   claim their ExternalLibraries packages explicitly (`et/mpi`), and generated ones always do.
4. **Hard conflicts:** two packages with a thorn of the same name (Cactus can't build both,
   related or not), and `incompatible-with`. Run-time exclusivity (one active thorn per
   implementation) is the parfile's concern, not the resolver's.
5. **Forks:** a chosen fork replaces its upstream in the installation; dependencies on the
   upstream are checked with `stands_in_for`.
6. **The plan is shown before anything changes:** what is added, upgraded, or replaced, and why.

### Escape hatches

- `--no-deps`: add exactly what was named.
- `--ignore-claim owner/pkg[:dependency:target]`: skip one declaration for this command.
- Durable overrides recorded in the installation (`cactup claims override …`): drop a wrong
  declaration, add a missing one, or say that one package satisfies another. Exported sims and
  generated manifests carry them, so they are visible to others.
- Every refusal ends with the command that gets past it.

## Generation (CactUp)

Best-effort declarations from a live installation, so a human writes little more than a review.

- `cactup package init [repo]` writes `cactarium.toml`; `cactup stack init` writes
  `cactarium-stack.toml` (`extends` the stack the installation came from, pins for repos that
  changed); `cactup sim export`/`pack` write `[[uses]]`.
- For each requirement a package's thorns don't meet themselves: the installed repo that meets it,
  mapped to its registry package and version (the Cactarium maps repo URL plus commit to a
  version), becomes a `[[dependency]]` with `needed-by` listing exactly the thorns that need it.
- **Ranges** default to the release series built with (`~2026.5`); a flag widens them.
- **Forks are detected:** a repo whose history contains an installed package's frozen commit and
  that has that package's thorns gets `[[fork-of]] target = "<package>@~<series>"`, and keeps the
  upstream's license unless told otherwise.
- Requirements nothing on the registry meets are written as commented TODO lines, never guessed.
- Generated files are written for review; the user commits them. `--publish` registers.

## Curated manifests

The curated packages' dependencies are generated the same way, with the official stacks as the
working setups: a requirement of a thorn is met by the curated provider that each official stack
building the thorn builds (matched by arrangement and thorn, not by name alone). When those stacks
agree, that is the dependency; when they differ or several providers remain, the requirement is
listed as unclaimed with the reason. Curated manifests give no range, since one overlay manifest
covers every version.
