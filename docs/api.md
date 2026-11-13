# The Cactarium API

The JSON a Cactarium server and its clients (CactUp) exchange. The types live in this crate
(`api.rs`), serde only; field names are kebab-case. All paths are under `/api/v1` (`API_PREFIX`).

## Conventions

- Every response carries `api-generation` (`API_GENERATION_HEADER`, now 1). A client that sees a
  generation it doesn't know asks the user to upgrade. Adding a field doesn't bump it: new fields
  are optional (`#[serde(default)]`), and response types never refuse unknown fields, so an older
  client reads a newer server's answer and a newer client an older server's. A list a server may
  write as `null` reads as empty.
- Errors are `ErrorBody`, `{"error":{"code":"…","message":"…"}}`, with a non-2xx status. `code` is
  `not-found`, `unauthorized`, `forbidden`, `bad-request`, `conflict`, `rate-limited` or
  `internal`; `message` is meant to be shown to the user as is.
- "Bearer" means `Authorization: Bearer <token>`.
- Repo URLs are matched with `url::normalize_repo_url`: scheme, login, default ports, a query,
  `.git` and slashes don't count, nor does the host's case; the path's case does, and a local
  path never matches a remote URL. The module docs say exactly what is equal.

## Endpoints

| Request | Response |
|---|---|
| `GET /resolve?req=owner/name[/sim]@[track:]range` | `Resolved` |
| `GET /entries/{owner}/{name}` | `EntryInfo` |
| `GET /packages/{owner}/{name}/versions` | `PackageVersions` (with the entry's `contact`) |
| `GET /packages/{owner}/{name}/versions/{commit\|head\|master}` | `PackageVersion` (see below) |
| `GET /stacks/{owner}/{name}/releases` | `StackReleases` |
| `GET /stacks/{owner}/{name}/releases/{version\|head\|master}` | `stack::FrozenStack` |
| `GET /sims/{owner}/{name}/{X.Y.Z}/download[?sim=<sim>]` | the `.sim` bytes, with `x-sha256` |
| `POST /sims/{owner}/{name}` (bearer, `.sim` body) | `Published` |
| `POST /stacks/{owner}/{name}/releases` (bearer, `ReleaseRequest`) | `Published` |
| `POST /packages/{owner}/{name}/refresh` (bearer) | `Refreshed` |
| `POST /lookup` (`LookupRequest`) | `LookedUp` |
| `GET /providers?implements=…` (or `capability=`, `function=`, `thorn=`) | `Providers` |
| `POST /repos` (bearer, `RegisterRequest`) | `Registered`; 409 `conflict` when `expect` no longer matches |
| `GET /whoami` (bearer) | `WhoAmI` |
| `POST /downloads` (bearer, `DownloadSubmission`) | `DownloadReview`; 409 `DownloadConflict` when known |

Multi-sim entries (`/sim` in `req`, `?sim=` on the download) are described in
[multi-sim.md](multi-sim.md).

`GET /packages/{owner}/{name}/versions/{commit|head|master}` answers one `PackageVersion`. At a
commit (full, or at least 7 hex digits), a commit some version froze answers with that version;
any other is read now, with `version` set to the full commit id. At `head` or `master` it is the
default branch's tip read now, with `version: "head"` or `"master"`. What is read now was never
reviewed: the answer says `x-cactarium-untrusted`.

`PackageVersions.contact` is where to go for help with the package (its declared contact, else
its repo), as `Resolved.contact` gives it, so a client resolving dependencies needn't ask
`/resolve` for each. Older servers leave it out.

## Headers

| Header | Direction | Meaning |
|---|---|---|
| `api-generation` | response | the API generation (`API_GENERATION_HEADER`) |
| `x-sha256` | response | a downloaded `.sim`'s sha256 (`SHA256_HEADER`) |
| `x-cactarium-unpinned` | response | `true` on `@head`/`@master` stacks, which aren't releases (`UNPINNED_HEADER`) |
| `x-cactarium-untrusted` | response | on what is read now: `true` when it isn't trusted (`UNTRUSTED_HEADER`) |
| `x-cactarium-contact` | response | where to go for help with a stack (`CONTACT_HEADER`) |
| `x-cactarium-include-unreviewed` | request | `true`: serve what awaits review too, as policy allows (`INCLUDE_UNREVIEWED_HEADER`) |
| `x-cactarium-unreviewed` | response | `true` when the answer serves something awaiting review (`UNREVIEWED_HEADER`) |

### What awaits review

CactUp's `--risky-allow-untrusted-artifacts` sends `x-cactarium-include-unreviewed: true`. The
server then may serve versions awaiting review too: to an admin's token always, to anyone when the
site serves unreviewed code. The answer says `x-cactarium-unreviewed: true`, and each version
awaiting review says `"review": "pending"`: in `VersionInfo`, `PinInfo` and `Provider`. `review` is
absent once a version is approved (and from older servers).

## `POST /lookup`: which package each pin is

No token. At most 1000 pins plus downloads (`MAX_LOOKUP_PINS`), each URL at most 2048 bytes
(`MAX_LOOKUP_URL_BYTES`); commits are full ids (40 or 64 lowercase hex digits), sha256s 64
lowercase hex digits (`LookupRequest::validate`).

```json
{
  "pins": [{ "url": "https://github.com/x/y.git", "commit": "3f2c9e1a00112233445566778899aabbccddeeff" }],
  "downloads": [{ "url": "https://example.org/id.tar.gz", "sha256": "…64 hex…" }]
}
```

The answer has one entry per pin and per download, each list in request order. `downloads` is
left out of a request (and its answer) that asks about none.

```json
{
  "pins": [{
    "url": "https://github.com/x/y.git",
    "commit": "3f2c9e1a00112233445566778899aabbccddeeff",
    "package": "et/carpet",
    "version": "2026.5.1",
    "flesh": false,
    "fork-of": ["et/carpet@=2026.5.1"],
    "commit-known": true
  }],
  "downloads": [{ "url": "https://example.org/id.tar.gz", "sha256": "…", "review": null }]
}
```

- `package`: the package whose repo the URL is, or `null`.
- `version`: the version that froze the commit, or `null` when none did (or there is no
  package). Among the versions this request may be served (valid; approved, or awaiting review
  for a request that asked; yanked ones count), when several froze the commit: the newest on the
  default track; with none there, the newest version on any track, ties going to the track name
  first in byte order. A `.sim` labels the pin `owner/pkg@<version>`, or a bare `owner/pkg` when
  `version` is `null`.
- `flesh`: the package is a Cactus flesh.
- `fork-of`: the package's `[[fork-of]]` targets at this commit.
- `commit-known`: `true` when the package's repo has the commit (a version froze it, or its
  history has it), `false` when it doesn't, `null` when there is no package or its history can't
  be read just now.
- `review`: `pending` when `version` awaits review; absent otherwise.
- A download's `review`: `pending`, `approved` or `rejected`, or `null` when it was never
  submitted (`POST /downloads`).

Pins and downloads are separate lists, rather than one list of either, so each entry has one
unambiguous shape.

## `GET /providers`: what meets a requirement

Exactly one of `implements`, `capability`, `function` or `thorn`, a CCL name
(`[A-Za-z0-9_]+`, so it never needs escaping). Each is one `ccl::Need` (`ProvidersQuery::of`,
`ProvidersQuery::need`), and the server answers with `ccl::meets`, so a client asks about exactly
what `ccl::unmet` left unmet; every need `ccl::needs_of` gives round-trips.

The answer covers, for each package and track, the newest version the request may be served
(valid and not yanked; approved, or awaiting review for a request that asked), listing that
version's thorns that meet the need. Older versions aren't searched; a client that needs one reads
`/packages/{owner}/{name}/versions`.

```json
{ "providers": [{ "package": "et/carpet", "version": "2026.5.1", "arrangement": "Carpet", "thorn": "Carpet" }] }
```

## `POST /repos`: register a repo

Bearer. Registers the manifests the repo carries as entries, as the site's "register a repo"
does: preview first (`dry-run`), then confirm what the preview showed (`expect`).

```json
{ "url": "https://github.com/EinsteinToolkit/Cottonmouth", "tag": "v2026.5.1", "owner": "et", "only": ["cottonmouth"], "dry-run": true }
```

- `tag`: the tag to start from, instead of each entry's newest own tag.
- `owner`: the namespace (an organization of the user's); else the user's own.
- `only`: register just the entries of these names; the others are `skipped`.
- `dry-run`: say what registering would do, writing nothing. The answer carries `expect`.
- `expect`: the fingerprint a dry run returned. Registering with it registers exactly what the
  preview showed, or nothing: when the repo or the entries already on it changed since, the
  answer is 409 `conflict` (preview again).

```json
{
  "entries": [
    { "slug": "et/cottonmouth", "kind": "package", "file": "cactarium.toml", "tag": "v2026.5.1",
      "version": "2026.5.1", "rule": "tags vX.Y.Z", "review": "reviewed", "status": "new",
      "new-versions": 0, "awaiting-review": 0 },
    { "slug": "et/cottonmouth-sims", "kind": "sim", "file": "cactarium-sim.toml", "tag": null,
      "version": null, "rule": "tags sim-vX.Y.Z", "review": null, "status": "skipped",
      "note": "only leaves it out", "new-versions": 0, "awaiting-review": 0 }
  ],
  "warnings": [],
  "expect": "3b1f9a…"
}
```

- `status`: `new` (registered, or would be by a dry run), `existing` (already on the repo),
  `skipped` (left out by `only`, or the tag isn't one of its versions), `needs-tag` (not every tag
  could be read: name the tag to start from) or `fails` (it can't be registered as it is, so
  nothing is). Why, in `note`.
- `review`, for a new entry: `reviewed` (its versions wait for an admin's review) or
  `auto-approved`.
- Once registered: `url` (the entry's page), `new-versions` (the tags its first check stored),
  `awaiting-review` (those waiting for review) and the entry's own `warnings` (what its first
  check reported). They default to absent, 0 and empty.
- `warnings` at the top blocked nothing.
- **A repo with nothing to add is answered 200** with every entry `existing`: the repo is
  registered, as asked, and a client tells "nothing to add" from the statuses. 409 `conflict` is
  kept for a stale `expect`.

## `GET /whoami`

Bearer. `{"user":"max","orgs":["et"],"admin":false}`: the username (also the user's own
namespace), the organizations the user may publish under (`null` reads as none), and whether the
user is an admin.

## `POST /downloads`: submit a download for review

Bearer. A `.sim` input or tarball component submitted for review, so pins to it can be trusted:

```json
{ "url": "https://example.org/id.tar.gz", "sha256": "…64 hex…", "note": "initial data from the paper's supplement" }
```

The answer is `{"id":7,"review":"pending"}` (`pending`, `approved` or `rejected`). A download
submitted before is answered 409 with an `ErrorBody` that also carries the known review, so it
reads as either `ErrorBody` or `DownloadReview` (`DownloadConflict`):

```json
{ "error": { "code": "conflict", "message": "already submitted" }, "id": 7, "review": "approved" }
```
