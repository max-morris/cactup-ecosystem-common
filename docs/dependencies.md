# Dependencies, forks and resolution

What a package, stack or sim declares about other packages, what each declaration means, how the
Cactarium checks it, and how CactUp resolves it. The types live in this crate (`manifest.rs`:
`Dependency`, `Alternative`, `ForkOf`, `Claim`; `version.rs`: `TrackedReq`, `TagPattern`).

## Principles

1. **The Cactarium hosts only what works as delivered.** A version whose declarations don't add
   up is refused, and a thorn that can't build isn't published.
2. **CCL says what a thorn needs; a dependency says which thorn of which package supplies it.**
   A thorn's CCL names the implementations it inherits or befriends, the capabilities and thorns
   it requires, and the functions it requires. A `[[dependency]]` names the package, its versions,
   and the thorns that meet those requirements (`needs`). The CCL is the checker, not the source:
   every hard requirement must be met by a declared thorn, and a declaration may add what the CCL
   can't express (a header included by path).
3. **Identity is a package plus a thorn, never a bare name.** Same-named thorns from different
   packages are not assumed to be related, let alone interchangeable.
4. **Nothing substitutes across packages unless a declaration says so:** an `any-of` the author
   wrote, or a `[[fork-of]]` the user chooses to install. Two packages that provide the same CCL
   implementation (Carpet and PUGH both implement `Hyperslab`) are never swapped otherwise.
5. **Generated declarations crystallize a working setup; tools flag, people confirm.** CactUp
   writes what an installation actually uses. Heuristics never write claims: what looks suspicious
   is shown to the author (as a TODO line) and to reviewers (as a smell).
6. **Escape hatches are first-class** on the CactUp side: every refusal names the way past it.

## Declarations

### `[[dependency]]`

```toml
[[dependency]]
target = "et/carpetx@^2026.5"   # owner/name, owner/name@<range>, or owner/name#<sha>
needs = ["CarpetX", "Loop"]      # thorns of the target this package uses
needed-by = ["Z4c"]              # thorns of this package it is for (optional; default all)
note = "CarpetX as the driver; the sources include its timer.hxx by path"

[[dependency]]                   # interchangeable packages, in order of preference
any-of = [
  { target = "et/carpet",     needs = ["CarpetSlab"] },
  { target = "et/cactuspugh", needs = ["PUGHSlab"] },
]
needed-by = ["IOASCII"]
```

- `target` without `@` means any version on the target's default track. Ranges use npm syntax
  (`^`, `~`, comparators, spaces for AND, hyphen ranges, `||` for OR; a track prefix on every
  alternative or on none). `#<sha>` pins one commit.
- **`needs` is required in a `[package]` when the target has thorns,** and left out when it has
  none (a flesh or a library). Only the Cactarium knows which a target is, so it enforces this at
  publish; `needs = []` is never accepted. In a `[stack]` (whose own thornlist names its thorns)
  `needs` is optional.
- **`into`**, for a library target only: where the library's checkout goes, relative to this
  package's repo root. Not with `needs` or `any-of`; only in a `[package]`.
- **`any-of`** replaces `target`: at least two packages, each with its own `needs`.
- `needed-by` names thorns this package publishes; it can't name an excluded thorn.
- A package's dependencies target packages; a stack is named with `[[works-with]]`.

### `[package] exclude-thorns`

Thorns the repo has but that can't build aren't published: `exclude-thorns = ["CarpetIOF5"]`
(CarpetIOF5 needs an F5 library nothing provides). Not together with `thorns`, which lists what is
published instead. A name the commit doesn't have is refused.

### `[package] flesh = true`

The package is a Cactus flesh (`et/cactus`, or a fork such as a mixed-precision flesh) rather than
thorns: checked for `src/main/flesh.cc` and `lib/sbin/CST`, no thorns, and `flesh: true` in the
API. A flesh fork declares `[[fork-of]] target = "et/cactus@…"`.

### `[package] library = true`

The package is a library that thorns build against rather than thorns or a flesh: FUKA, which
KadathThorn builds from source. It has no thorns and nothing in it is checked for CCL. Being a
package gives the library an identity, so a sim or stack that checks it out pins a registered
package like any other repo.

Where the library goes is a fact about the package that uses it (KadathThorn's `src/build.sh`
builds FUKA from `src/fuka`), so that package says it:

```toml
[[dependency]]
target = "et/fuka"
into = "src/fuka"
needed-by = ["KadathThorn"]
```

CactUp, composing a thornlist from packages (`cactup add et/kadaththorn`), writes the library's
checkout with `!TARGET` at that folder of the dependent package's checkout. A sim or stack carries
its own thornlist, whose `!TARGET` already places the library; there `into` is not consulted.

### `[[fork-of]]`

```toml
[[fork-of]]
target = "et/carpet@~2026.5"     # owner/name, owner/name@<range>, or owner/name#<sha>
note = "subcycling in time"
```

The package is a drop-in replacement for the target at the versions `target` names (a bare
`owner/name`: every version on the default track).

- **When a fork satisfies a dependency on its upstream** (`ForkOf::stands_in_for`): some published
  version of the upstream is both one the dependency accepts and one the fork replaces. A sim's
  pin (`et/carpet@2026.5.1`) is satisfied when the fork's range contains it. A dependency on one
  commit is satisfied only by a fork of that commit.
- A fork and its upstream are never installed together (they share thorn names). A fork is used
  only when the user chooses it; its dependencies then stand in for the upstream's.
- Reviewers are warned when a fork lacks thorns that the newest upstream version in its range has.

### `[[works-with]]`, `[[incompatible-with]]`

Displayed claims with the same `target` forms. The resolver treats `incompatible-with` as a hard
conflict and `works-with` as information.

### Sims

A sim pins every repo to a commit already; it declares no dependencies. The Cactarium derives what
a sim is built on from its pins (repo URL plus commit → `owner/pkg@version`, or `owner/pkg#sha` for
a commit no version froze) and shows it. **Every pin must belong to a registered package**, at a
commit that package's repo has, or the sim is refused: a downstream user must be able to rebuild it
from the Cactarium alone. The flesh pin is `et/cactus` (or a fork of it).

## The check at publish (the Cactarium)

A package version is refused unless:

- each dependency's target (every alternative of an `any-of`) has a version on the Cactarium in
  range, or at the commit, that has the `needs` thorns;
- `needs` is given exactly when the target has thorns, and `into` exactly when it is a library;
- **every hard CCL requirement of every published thorn** is met by the package's own thorns or by
  the `needs` thorns of a dependency that applies to that thorn (its `needed-by` includes the thorn,
  or is empty); for an `any-of`, by every alternative.

The refusal lists every problem and, where it can tell, the line that fixes it ("add `CarpetSlab`
to the needs of the dependency on et/carpet").

**Smells** don't block; they are stored with the version and shown to reviewers:

- a `needs` thorn no CCL requirement points to (legitimate for a header included by path, but
  worth a look);
- an include that climbs out of the thorn's own directory by path, into another thorn's sources;
- a generic `inherits: Driver` next to such an include (it may need one driver in particular);
- the thorns `exclude-thorns` leaves out.

## Resolution (CactUp)

Runs on `cactup add`, `cactup install`, and upgrades, in CactUp; the Cactarium supplies the data
(versions, their dependencies and fork-of declarations, per-thorn CCL facts).

1. **What is installed stays fixed** unless `--upgrade` is given.
2. **Transitive closure** over `[[dependency]]`: install the `needs` thorns of each dependency (a
   library: its checkout, at `into`), then those thorns' own package's dependencies, and so on. Ranges are intersected with what is
   already chosen; tracks are honored; `#sha` pins are exact.
3. **`any-of`**, in this order: an alternative already installed; else one the rest of the plan
   already brings in; else, among those that conflict with nothing installed or planned, the first
   listed. The choice is printed and recorded in the installation.
4. **Hard conflicts:** two packages with a thorn of the same name (Cactus can't build both, related
   or not), and `incompatible-with`. Run-time exclusivity (one active thorn per implementation) is
   the parfile's concern.
5. **Forks:** a chosen fork replaces its upstream; dependencies on the upstream are checked with
   `stands_in_for`.
6. **The plan is shown before anything changes:** what is added, upgraded, or replaced, and why.

### Escape hatches

- `--no-deps`: add exactly what was named.
- `--ignore-claim owner/pkg[:dependency:target]`: skip one declaration for this command.
- Durable overrides recorded in the installation (`cactup claims override …`): drop a wrong
  declaration, add a missing one or a missing `needs` thorn, or say that one package satisfies
  another. Exported sims and generated manifests carry them.
- `--risky-allow-untrusted-artifacts`, with an admin token, also reaches versions awaiting review,
  for reviewing them.
- Every refusal ends with the command that gets past it.

## Generation (CactUp)

- `cactup package init [repo]` writes `cactarium.toml`; `cactup stack init` writes
  `cactarium-stack.toml` (`extends` the stack the installation came from, pins for repos that
  changed).
- For each requirement a package's thorns don't meet themselves: the installed thorn that meets it,
  mapped to its registry package and version, becomes a `[[dependency]]` with that thorn in `needs`
  and `needed-by` listing exactly the thorns that need it.
- **Ranges** are patch ranges floored at the version built with: built with et/carpetx 2026.5.1,
  the claim is `et/carpetx@~2026.5.1` (2026.5.1 or a later 2026.5 patch, never older). Patch
  releases are trusted to be compatible; a patch that breaks something is excluded with the range
  syntax (`>=2026.5.1 <2026.5.3 || >2026.5.3 <2026.6.0`) or worked around with a durable override.
  `--exact` pins the version built with instead, for an upstream not to be trusted. Exactness for
  reproducing a run lives in sims and stack releases, which pin commits.
- **Forks are detected:** a repo whose history contains an installed package's frozen commit and
  that has that package's thorns gets `[[fork-of]] target = "<package>@~<series>"`.
- **What looks suspicious becomes a commented TODO line,** never a claim: an include that reaches
  another package's thorn by path, a generic `Driver` inherit next to one driver's headers.
- Generated files are written for review; the user commits them. `--publish` registers.
- **`cactup sim export` refuses** thorns with uncommitted changes or unpushed commits, or whose repo
  isn't a registered package's, naming the repo. `--local` writes a sim for personal use that the
  Cactarium will never accept.

## Curated manifests

Generated the same way, with the official stacks as the working setups: a requirement of a thorn is
met by the curated provider each official stack building the thorn builds (matched by arrangement
and thorn, not by name alone). Requirements several packages meet are settled by hand (an `any-of`
or one package), and thorns nothing can build are excluded. Curated manifests give no range; one
overlay covers every version from its `since` on, and an admin re-applies a corrected overlay to
published versions.
