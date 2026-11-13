# The Cactarium API

The JSON a Cactarium server and its clients (CactUp) exchange. The types live in this crate
(`api.rs`), serde only; field names are kebab-case. All paths are under `/api/v1` (`API_PREFIX`).

## Conventions

- Every response carries `api-generation` (`API_GENERATION_HEADER`, now 1). A client that sees a
  generation it doesn't know asks the user to upgrade. Adding a field doesn't bump it: new fields
  are optional (`#[serde(default)]`), and response types never refuse unknown fields, so an older
  client reads a newer server's answer and a newer client an older server's.
- Errors are `ErrorBody`, `{"error":{"code":"…","message":"…"}}`, with a non-2xx status. `code` is
  `not-found`, `unauthorized`, `forbidden`, `bad-request`, `conflict`, `rate-limited` or
  `internal`; `message` is meant to be shown to the user as is.
- "Bearer" means `Authorization: Bearer <token>`.
- Repo URLs are matched with `url::normalize_repo_url`: scheme, login, port, `.git` and slashes
  don't count, nor does the host's case; the path's case does.

## Endpoints

| Request | Response |
|---|---|
| `GET /resolve?req=owner/name[/sim]@[track:]range` | `Resolved` |
| `GET /entries/{owner}/{name}` | `EntryInfo` |
| `GET /packages/{owner}/{name}/versions` | `PackageVersions` |
| `GET /stacks/{owner}/{name}/releases` | `StackReleases` |
| `GET /stacks/{owner}/{name}/releases/{version\|head\|master}` | `stack::FrozenStack` |
| `GET /sims/{owner}/{name}/{X.Y.Z}/download[?sim=<sim>]` | the `.sim` bytes, with `x-sha256` |
| `POST /sims/{owner}/{name}` (bearer, `.sim` body) | `Published` |
| `POST /stacks/{owner}/{name}/releases` (bearer, `ReleaseRequest`) | `Published` |
| `POST /packages/{owner}/{name}/refresh` (bearer) | `Refreshed` |
| `POST /lookup` (`LookupRequest`) | `LookedUp` |
| `GET /providers?implements=…` (or `capability=`, `function=`, `thorn=`) | `Providers` |
| `POST /repos` (bearer, `RegisterRequest`) | `Registered`; 409 `conflict` when there is nothing to add |
| `GET /whoami` (bearer) | `WhoAmI` |
| `POST /downloads` (bearer, `DownloadSubmission`) | `DownloadReview`; 409 `DownloadConflict` when known |

Multi-sim entries (`/sim` in `req`, `?sim=` on the download) are described in
[multi-sim.md](multi-sim.md).

## Headers

| Header | Direction | Meaning |
|---|---|---|
| `api-generation` | response | the API generation (`API_GENERATION_HEADER`) |
| `x-sha256` | response | a downloaded `.sim`'s sha256 (`SHA256_HEADER`) |
| `x-cactarium-unpinned` | response | `true` on `@head`/`@master` stacks, which aren't releases (`UNPINNED_HEADER`) |
| `x-cactarium-untrusted` | response | on `@head`/`@master`: `true` when the stack isn't trusted (`UNTRUSTED_HEADER`) |
| `x-cactarium-contact` | response | where to go for help with a stack (`CONTACT_HEADER`) |
| `x-cactarium-include-unreviewed` | request | `true`: serve what awaits review too, as policy allows (`INCLUDE_UNREVIEWED_HEADER`) |
| `x-cactarium-unreviewed` | response | `true` when the answer serves something awaiting review (`UNREVIEWED_HEADER`) |

### What awaits review

CactUp's `--risky-allow-untrusted-artifacts` sends `x-cactarium-include-unreviewed: true`. The
server then may serve versions awaiting review too: to an admin's token always, to anyone when the
site serves unreviewed code. The answer says `x-cactarium-unreviewed: true`, and each version
awaiting review says so in `VersionInfo`:

```json
{ "version": "2026.5.1", "…": "…", "review": "pending" }
```

`review` is absent once a version is approved (and from older servers).

## `POST /lookup`: which package each pin is

No token. At most 1000 pins and downloads together (`MAX_LOOKUP_PINS`); commits are full ids
(40 or 64 lowercase hex digits), sha256s 64 lowercase hex digits (`LookupRequest::validate`).

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
  "downloads": [{ "url": "https://example.org/id.tar.gz", "sha256": "…", "reviewed": null }]
}
```

- `package`: the package whose repo the URL is, or `null`.
- `version`: the version that froze the commit (`2026.5.1`, `mixed-precision:1.2.0`), or `null`
  when none did (the pin is then `package#commit`, which a `.sim` stamps with format 4).
- `flesh`: the package is a Cactus flesh.
- `fork-of`: the package's `[[fork-of]]` targets at this commit.
- `commit-known`: `true` when the package's repo has the commit (a version froze it, or its
  history has it), `false` when it doesn't, `null` when there is no package or its history can't
  be read just now.
- `reviewed`: `true` when an admin approved the download at that sha256, `false` while it awaits
  review or after it was rejected, `null` when it was never submitted (`POST /downloads`).

Pins and downloads are separate lists, rather than one list of either, so each entry has one
unambiguous shape.

## `GET /providers`: what meets a requirement

Exactly one of `implements`, `capability`, `function` or `thorn`, a CCL name. Each is one
`ccl::Need` (`ProvidersQuery::of`, `ProvidersQuery::need`), and the server answers with
`ccl::meets`, so a client asks about exactly what `ccl::unmet` left unmet.

```json
{ "providers": [{ "package": "et/carpet", "version": "2026.5.1", "arrangement": "Carpet", "thorn": "Carpet" }] }
```

## `POST /repos`: register a repo

Bearer. Registers the manifests the repo carries as entries, as the site's "register a repo"
does.

```json
{ "url": "https://github.com/EinsteinToolkit/Cottonmouth", "tag": "v2026.5.1", "owner": "et", "only": ["cottonmouth"], "dry-run": true }
```

- `tag`: read every manifest at this tag, instead of each entry's newest own tag.
- `owner`: the namespace (an organization of the user's); else the user's own.
- `only`: register just the entries of these names; the others are `skipped`.
- `dry-run`: say what registering would do, writing nothing.

```json
{
  "entries": [
    { "slug": "et/cottonmouth", "kind": "package", "file": "cactarium.toml", "tag": "v2026.5.1",
      "version": "2026.5.1", "rule": "tags vX.Y.Z", "review": "pending", "status": "new" },
    { "slug": "et/cottonmouth-sims", "kind": "sim", "file": "cactarium-sim.toml", "tag": null,
      "version": null, "rule": "tags sim-vX.Y.Z", "review": null, "status": "skipped",
      "note": "only leaves it out" }
  ],
  "warnings": []
}
```

`status` is `new` (registered, or would be by a dry run), `existing` (already on the repo),
`skipped` (left out by `only`, or the tag isn't one of its versions) or `fails` (reading it fails,
so nothing is registered). `review` is `pending` or `approved` for the first version read.
`warnings` blocked nothing. A repo that already has every entry its manifests give is answered
409 `conflict`.

## `GET /whoami`

Bearer. `{"user":"max","orgs":["et"],"admin":false}`: the username (also the user's own
namespace), the organizations the user may publish under, and whether the user is an admin.

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
