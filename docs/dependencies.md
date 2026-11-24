# Dependencies, forks and resolution

What a package, stack or sim declares about other packages, what each declaration means, how
Cactarium checks it, and how Cactup resolves it. The types live in this crate (`manifest.rs`:
`Dependency`, `Alternative`, `ForkOf`, `Claim`; `version.rs`: `TrackedReq`, `TagPattern`;
`ccl.rs`: `Need`, `needs_of`, `meets`, `unmet`; `url.rs`: `normalize_repo_url`), and the API
shapes the two exchange in `api.rs` ([api.md](api.md)).

## Principles

1. **Cactarium hosts only what works as delivered.** A version whose declarations don't add
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
5. **Generated declarations crystallize a working setup; tools flag, people confirm.** Cactup
   writes what an installation actually uses. Heuristics never write claims: what looks suspicious
   is shown to the author (as a TODO line) and to reviewers (as a smell).
6. **Escape hatches are first-class** on the Cactup side: every refusal names the way past it.

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
  none (a flesh or a library). Only Cactarium knows which a target is, so it enforces this at
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

Cactup, composing a thornlist from packages (`cactup add et/kadaththorn`), writes the library's
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

A sim pins every repo to a commit already; it declares no dependencies. Cactarium derives what
a sim is built on from its pins (repo URL plus commit → `owner/pkg@version`, or `owner/pkg#sha` for
a commit no version froze) and shows it. A repo URL matches a registered package's by
`url::normalize_repo_url` (scheme, login, default ports, `.git` and slashes don't count; host
case doesn't either; path case does, with `url::same_repo_lenient` as the shared fallback on
forges that ignore it). `POST /api/v1/lookup` makes the same derivation for a
client before it publishes. **Every pin must belong to a registered package**, at a commit that
package's repo has, or the sim is refused: a downstream user must be able to rebuild it from
Cactarium alone.

A pin may carry that derivation as a label, `cactarium` in its `[[repo]]` (a frozen stack's pins
too): `owner/pkg@<exact version>` when a version froze its commit, else a bare `owner/pkg`, whose
`commit` says which commit. A label never names a commit (`#<sha>`), a range, a tip or a thorn,
and a download (`sha256` pin) has none. Both forms are `.sim` format 1. (`owner/pkg#<sha>` stays a
reference a user types, `cactup add et/carpet#3f2c9e1`, and a target a declaration may give; it is
never a pin's label.) The flesh pin is `et/cactus` (or a fork of it).

## CCL requirements (`ccl::Need`)

Cactarium's check and Cactup's composition and trim read a thorn's requirements the same way,
with this crate's `needs_of` and `meets`, so they cannot disagree about what a thorn needs:

| `Need` | From the thorn's facts | Met by a thorn that |
|---|---|---|
| `Implementation(i)` | `INHERITS: i`, `FRIEND(S): i` | `IMPLEMENTS: i` |
| `Capability(c)` | `REQUIRES c` (configuration.ccl) | `PROVIDES c` |
| `Thorn(t)` | `REQUIRES THORNS: t` | is named `t` |
| `Function(f)` | `REQUIRES FUNCTION f` | `PROVIDES FUNCTION f` (the alias, not the providing routine) |
| `Include(h)` | `USES INCLUDE: h`, and a source `#include`s `h` outside any conditional | `INCLUDE[S] ...: x IN h` |

- Names compare case-insensitively, as Cactus compares them, and so do `Need`s themselves
  (`Need::key`); each need is listed once. Only CCL names (`[A-Za-z0-9_]+`, `ccl::is_ccl_name`)
  are read as names, so every need fits a `/providers` query.
- A thorn without an `IMPLEMENTS` line implements nothing (Cactus refuses to build one), so it
  never meets an implementation need, even one spelled like its own name.
- Not needs: `OPTIONAL` capabilities, `USES FUNCTION`, and a `USES INCLUDE` header the sources
  include only under `#if`/`#ifdef` (Carpet's `nompi.h` under `#ifndef CCTK_MPI`), or not at all.
  A capability's version constraint (`HDF5(>=1.10)`) isn't read.
- `Include` is the one need read from the sources, not the CCL (`ThornFacts::scan_sources`, which
  `scan_tree` runs): Cactus makes the header a thorn includes from every provider in the
  configuration, and from none it is empty, so an unconditional `#include` of a header nothing
  provides fails to compile (EinsteinInitialData/TwoPuncturesX without TwoPunctures). An include
  guard (`#ifndef X` then `#define X`) is no conditional. It counts against a set only where some
  thorn the checker knows of provides the header, and none of those is optional to the thorn
  (`ccl::counts`, `ccl::unmet_among`): a header nothing provides compiles empty, which is how hooks
  work (ADMMacros' `CalcTmunu.inc`, Carpet's `Requirements.hh`), or reaches the thorn through a
  capability's include directory; a provider that provides or implements something the thorn
  declares `OPTIONAL`/`OPTIONAL_IFACTIVE` is one it builds without (Slab's `loopcontrol.h`,
  GRHydro's `carpet.hh`). A header whose own guard macro the sources test (`#include
  "http_Content.h"`, then `#ifdef __HTTP_CONTENT_H__`, Formaline) is not needed either. Checked
  against every config built in an ET 2026_05 tree, this flags nothing; against the
  et/cottonmouth-sims 2026.5.1 thornlist it flags exactly TwoPuncturesX. Facts stored before
  `provides-includes`/`needs-includes` existed read as providing and needing no header: rescan
  them before checking a set against them.
- `unmet(facts, thorns)` lists the needs a set of thorns leaves unmet. `GET /api/v1/providers`
  answers "which published thorns meet this need" (`api::ProvidersQuery::of(&need)`).

How a requirement is met is common ground; what to do about one is not. Cactarium refuses a
version with an unmet need. Cactup's trim keeps every thorn that could meet a required capability
or thorn, and for an implementation or function adds providers only when no kept thorn already
meets it (the parfile chose that one).

## The check at publish (Cactarium)

A package version is refused unless:

- each dependency's target (every alternative of an `any-of`) has a version on Cactarium in
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

## Resolution (Cactup)

Only `cactup add` resolves, and `cactup remove` for what depends on the packages it removes.
Installing a stack and `cactup inst refetch --release` resolve nothing: the stack or release fixes
every repo already. Afterward they run the claims check (as `cactup claims check` does) and print
what it finds as warnings, waiting at most 15 s for each request to Cactarium and 20 s in all.
Cactarium supplies the data (versions, their dependencies and fork-of declarations, per-thorn CCL
facts).

1. **What is installed stays fixed**, except:
   - `--upgrade` moves the installed packages the plan touches to the versions it needs (without
     it, a plan that needs an installed package at another version is refused); `--upgrade-all`
     moves every installed package to its newest fitting version.
   - An installed package named on the command line moves to its newest release that fits.
   - A package the base (the release or stack installed) brings never moves without `--upgrade`,
     `--upgrade-all` or `--replace`.
   - Approved releases are preferred over pending ones, and a release already present moves to an
     older one only when the plan says so ("downgrade").
2. **Transitive closure** over `[[dependency]]`: install the `needs` thorns of each dependency, then
   those thorns' own package's dependencies, and so on. Ranges are intersected with what is
   already chosen; tracks are honored; `#sha` pins are exact. A library dependency is placement,
   not a need: it puts the library's checkout at `into` and meets no CCL requirement (`--no-deps`
   still places libraries).
3. **`any-of`**, in this order: an alternative already installed; else one the rest of the plan
   already brings in; else, among those that conflict with nothing installed or planned, the first
   listed. The choice is printed and recorded in the installation.
4. **Hard conflicts:** two enabled thorns of one name from different packages (Cactus can't build
   both, related or not); a thorn of that name that isn't enabled conflicts with nothing. Also
   `incompatible-with`, and a fork and its upstream, which always conflict, whatever is enabled.
   A base repo that is no registered package takes part as `local:<name>` (its repo name in the
   thornlist). Run-time exclusivity (one active thorn per implementation) is the parfile's concern.
5. **Forks:** a chosen fork replaces its upstream; dependencies on the upstream are checked with
   `stands_in_for`.
6. **The CCL check on the delta:** every hard requirement of a thorn the plan adds must be met by a
   thorn installed or planned. An unmet requirement always refuses the plan, however many
   providers could meet it (none, one or several), and Cactup never plans a provider no
   declaration names. The refusal names what could meet it (`GET /providers`) and the override that
   adds the missing declaration.
7. **One flesh at a time:** a flesh package replaces the installed flesh, and never displaces one
   that something installed needs.
8. **Stand-ins:** a base repo of a registered package at a commit Cactup hasn't read (no version
   froze it, `--no-deps`, or the read went unanswered) stands in for that package. Its thorns are
   those of its clone at that commit, its fork-of is what the lookup said, and its dependencies are
   read only when the plan depends on them.
9. **Batch reads:** the releases a plan needs are read with `POST /packages/read` ([api.md](api.md)),
   many at once, in rounds, for at most 5 minutes per command. From an older Cactarium without it,
   or for an item answered in a way Cactup can't use, they are read one at a time.
10. **The plan is shown before anything changes:** what is added, upgraded, downgraded, replaced or
    removed, and why. Cactup asks for confirmation only when the plan goes beyond what was named;
    with no terminal to ask on, such a plan needs `--yes`. `--dry-run` shows the plan and changes
    nothing.

`cactup remove` is refused while another installed package depends on one named. Packages brought
in only for those named (orphans) are removed too, after the plan is shown; `--no-deps` removes only
what was named. A repo a removed package replaced is restored.

### Escape hatches

- `--no-deps`: add exactly what was named (libraries are still placed), or remove only that.
- `--ignore-claim OWNER/PKG[:KIND:TARGET]`: skip a package's claims, or one of them, for this
  command. KIND is `dependency`, `incompatible-with` or `fork-of`.
- Durable overrides recorded in the installation (`cactup claims override drop | add | needs |
  satisfies`, and `--remove KEY`): drop a wrong claim, add a missing dependency or a missing
  `needs` thorn, or let one package stand in for another (`satisfies`, with no version check;
  `satisfies local:NAME` names a base repo that is no registered package). They apply to every add
  and remove in the installation. Exported sims carry them only as information, in
  `provenance.extra["claims-overrides"]`; generated manifests carry them as comments.
- `cactup claims list` shows the overrides and the `any-of` choices. `cactup claims check` checks
  the installed packages against each other's claims (clashing thorns, incompatibilities, forks
  beside their upstream, unmet dependency claims, CCL requirements no installed thorn meets,
  `satisfies` overrides naming a missing repo), changes nothing, and fails when it finds a problem.
- `--yes` skips the confirmation; `--dry-run` shows the plan and changes nothing.
- `--risky-allow-untrusted-artifacts` also asks Cactarium for releases awaiting review
  (`x-cactarium-include-unreviewed`, [api.md](api.md)), sending the user's token, which commands
  otherwise don't send. It isn't for admins only: anyone gets the pending releases of public
  entries when the site allows untrusted tips, and an admin's token also reaches entries awaiting
  their first review. Cactup warns when an answer says `x-cactarium-unreviewed: true`.
- Every refusal ends with the command that gets past it.

## Generation (Cactup)

- `cactup package init [repo]` writes `cactarium.toml`; `cactup stack init` writes
  `cactarium-stack.toml`.
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
  that has that package's thorns gets a `[[fork-of]]` on exactly the newest upstream version whose
  frozen commit the fork's history contains, never a `~<series>` range: `<package>@=<X.Y.Z>` on
  the default track (`et/carpet@=2026.5.1`), `<package>@<track>:=<X.Y.Z>` on a track
  (`et/carpet@mixed-precision:=1.2.0`; the track comes before the `=`). `ForkOf::exactly`
  writes it (`TrackedReq::exactly` the range). That is all the history proves; an author who knows
  the fork keeps up with later patches widens the range by hand. `POST /api/v1/lookup` answers
  which package and version a commit is, and what a package forks (`fork-of` in its answer).
  Cactup's clones are shallow (depth 1), and a shallow clone's history can't show a fork: the
  manifest gets a TODO line saying to run `git fetch --unshallow` in the clone, then
  `cactup package init <repo> -f`.
- **What looks suspicious becomes a commented TODO line,** never a claim: an include that reaches
  another package's thorn by path, a generic `Driver` inherit next to one driver's headers.
- Without `--publish`, generated files are written for review, and the user commits them.
- **`cactup package init --publish` writes nothing.** It registers or refreshes the `cactarium.toml`
  committed at the repo's newest pushed release tag, through `POST /repos`: a dry run with `only`
  naming the package, then, when its row is `new`, registering with the dry run's `expect`, or,
  when it is `existing` (an entry already on the repo), refreshing that entry.

### `cactup stack init`

- The definition `extends` the stack release the installation is on. An installation on no
  release, or on a stack at a branch tip or a bare commit, is refused: no stack can extend one.
- What installs is pinned exactly. A base repo that moved since the release is a `[[stack.pin]]`.
  A package that takes every checkout of one base repo is a `[[stack.override]]` of that repo (at
  its exact version, or at the commit fetched for a tip or a commit no version froze); any other
  added package is a pinned section of a generated `cactarium-stack.th`.
- The installed packages' dependencies follow as `[[dependency]]` entries, one per declaration:
  information on what could be swapped in, never resolved at install.
- `--publish --version` writes nothing and publishes the definition found there as that release.

### `cactup sim export`

- Each pin's `cactarium` label is `owner/pkg@X.Y.Z`, or a bare `owner/pkg` for a commit no version
  froze, never `#<sha>` ([Sims](#sims)).
- Export refuses, naming each: thorns with uncommitted changes or unpushed commits, or whose repo
  isn't a registered package's; a download that isn't approved, has no `sha256`, or is at a private
  address; a URL with credentials in it, or one that no one can fetch anonymously; an svn, cvs, hg
  or darcs section that isn't pinned, and a section of several files. `--local` is the only way
  past: it writes a sim for personal use that Cactarium will never accept. (There is no
  `--allow-dirty` or `--allow-unpushed`.)

## Curated manifests

Generated the same way, with the official stacks as the working setups: a requirement of a thorn is
met by the curated provider each official stack building the thorn builds (matched by arrangement
and thorn, not by name alone). Requirements several packages meet are settled by hand (an `any-of`
or one package), and thorns nothing can build are excluded. Curated manifests give no range; one
overlay covers every version from its `since` on, and an admin re-applies a corrected overlay to
published versions.
