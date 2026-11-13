# Sims from a repo: one sim or several

How a repo publishes sims through `cactarium-sim.toml`, what the formats look like, how each sim
is addressed, and what a client (CactUp) sends and receives. The types live in this crate
(`manifest.rs`: `SimSection`, `SimForm`; `name.rs`: `SimName`; `sim.rs`: `SimFile`;
`slug.rs`: `Slug::as_sim`, `Slug::as_thorn`; `api.rs`: `Resolved`, `EntryInfo`, `SIM_QUERY`).

## The two forms

A `[sim]` section names its `.sim` files in exactly one of two ways. **The form, never the
count, decides how the sims are addressed.**

### `file`: one sim, the entry itself (manifest format 1)

```toml
format = 1

[sim]
name = "qc0-demo"
owners = ["max-morris"]
file = "runs/qc0.sim"
```

- The entry is the sim: `max-morris/qc0-demo[@range]`.
- The stored copy of each version is stamped `[sim] cactarium = "max-morris/qc0-demo@1.2.0"`.

### `[sim.files]`: several sims released together (manifest format 2)

```toml
format = 2

[sim]
name = "cottonmouth-sims"
owners = ["et"]
description = "Cottonmouth's example simulations, on et/cottonmouth-stack."
tag-pattern = "sim-v{major}.{minor}.{patch}"

[sim.files]
qc0-bssnok = "examples/sims/qc0-bssnok.sim"
qc0-z4c    = "examples/sims/qc0-z4c.sim"
mag-tov    = "examples/sims/mag-tov.sim"
```

- A table keyed by **sim name**, each value a `.sim` path relative to the repo root. TOML itself
  refuses a key listed twice; two names may not point at the same file.
- Sim names are `SimName`s: the entry-name rules (lowercase `[a-z0-9._-]`, at most 64 characters,
  not ending in `.` or `-`, no `..`) and **starting with a letter**, so `owner/name/sim` can never
  read as a version or a commit.
- `files` and `file` exclude each other; one of them is required.
- Every sim is always named, even while the table lists one: `et/cottonmouth-sims/qc0-z4c[@range]`.
  A bare `owner/name` would otherwise silently change meaning as the list grows or shrinks.
- The stored copy of each sim is stamped `[sim] cactarium = "et/cottonmouth-sims/qc0-z4c@1.0.0"`.
- A sim's own `[sim] name` inside its `.sim` file may differ from its key; the Cactarium shows a
  reviewer warning, nothing more.

`SimSection::form()` tells the two apart (`SimForm::File(path)` or `SimForm::Files(map)`), and
`SimSection::sims()` lists every sim with its file (`None` as the name for a `file` entry).

## Format generations

Every format carries a top-level `format`, and every reader is strict: unknown keys are an
error, and a generation newer than the reader knows is refused with an "upgrade" message
(`Error::NewerFormat`). A feature that adds a key therefore always comes with a new generation,
so an older reader says "upgrade" rather than "unknown key".

| Format | Generation | Added | Required when |
|---|---|---|---|
| Manifest (`MANIFEST_FORMAT`) | 2 | `[sim.files]` | `[sim.files]` is used (`Manifest::required_format`) |
| `.sim` (`SIM_FORMAT`) | 4 | a `[sim] cactarium` stamp of three parts | the stamp names one sim of a multi-sim entry (`SimFile::required_format`) |

- Format 2 manifests without `[sim.files]` are fine: generations only add, and every older
  generation is read forever (tags are immutable).
- `SimFile::to_toml` writes the oldest generation that can hold the file, so a sim from a `file`
  entry stays format 1, 2 or 3 as before and only stamped copies of multi-sim sims are format 4.
  A reader from before format 4 would read `et/x/qc0-z4c` as a thorn, so it is asked to upgrade.
- The stamp's third segment must be a sim name (`Slug::as_sim`).

## References: the slug's third segment

`Slug` keeps the third segment raw in `thorn` and accepts `[A-Za-z][A-Za-z0-9_.-]*` there. The
entry's kind decides what it means:

| Entry | `owner/name` | `owner/name/x` |
|---|---|---|
| package | the package | `x` is a thorn: `Slug::as_thorn` checks thorn-name rules (`[A-Za-z][A-Za-z0-9_]*`) |
| stack | the stack | `x` must be a thorn name, as before (a stack names no sims) |
| sim, `file` form | the sim | refused: a one-sim entry has no sims to name |
| sim, `files` form | refused, listing its sims | `x` is one of its sims: `Slug::as_sim` checks sim-name rules |

Declarations that name packages or stacks keep their old rules: `[[dependency]]`, `[[fork-of]]`,
claims and a stack's `extends` refuse any third segment; a stack override's `package`, a `.sim`
pin's `cactarium` and its provenance `base` accept only a thorn name there (`Slug::as_thorn`), as
before the segment widened. `valid_thorn` is public for clients that read a third segment as a
thorn.

An old CactUp reads `et/cottonmouth-sims/qc0-z4c` as a thorn (and refuses `qc0-z4c`, which is
no thorn name, or ignores a thorn-like one). It stays safe: the Cactarium refuses a bare
`owner/name` for a multi-sim entry, and the download requires `?sim=` for one.

## Versions

A version is one tag holding every sim its manifest lists at that tag. Each is checked as an
upload is; if one fails, the whole version is refused with every reason (a partial version would
silently freeze a version missing a sim). Review, yank and visibility are per version. The list
may change from version to version: a sim can be added, or dropped.

## API (what a client sends and receives)

All under `/api/v1` (`API_PREFIX`).

### Resolve: `GET /resolve?req=owner/name/sim@range`

For a multi-sim entry, the answer is the newest servable version in range **that contains the
sim**. `Resolved` gains two optional fields (absent from older servers, and for other entries):

- `sim`: the sim chosen; pass it to the download as `?sim=`;
- `newer-without-sim`: the newest version in range when it is newer than the one chosen and no
  longer contains the sim (it was dropped), so the client can warn.

```json
{
  "slug": "et/cottonmouth-sims/qc0-z4c@1.0.0",
  "kind": "sim",
  "url": "https://github.com/EinsteinToolkit/Cottonmouth",
  "contact": "https://github.com/EinsteinToolkit/Cottonmouth",
  "sim": "qc0-z4c",
  "newer-without-sim": "1.1.0",
  "version": "1.0.0",
  "track": null,
  "tag": "sim-v1.0.0",
  "commit": "…",
  "tree": "…",
  "yanked": false,
  "yank-reason": null,
  "state": "ok",
  "curated": false,
  "published": 1762905600
}
```

Errors (`ErrorBody`, `bad-request`):

- `owner/name` for a multi-sim entry: the message lists its sims (`owner/name/qc0-z4c`, …);
- `owner/name/sim` for a `file` entry: name it without the sim;
- a sim no version in range contains: says so, naming the sims the newest version has.

### Download: `GET /sims/{owner}/{name}/{X.Y.Z}/download?sim=<sim>`

`?sim=` (`SIM_QUERY`) is required for a multi-sim entry and refused for a `file` entry. The
body is the stamped `.sim`; `x-sha256` (`SHA256_HEADER`) is that sim's own sha256.

### Entry: `GET /entries/{owner}/{name}`

`EntryInfo.sims` lists the sims of the newest version of a multi-sim entry (empty otherwise).

## What a client caches

The `.sim` a version serves never changes, so a cache key of owner, name and version is enough
for a `file` entry. A multi-sim entry serves one file per sim, so the key needs the sim name too
(for example `simfiles/<owner>/<name>/<sim>/<version>.sim`).
