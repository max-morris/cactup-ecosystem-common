//! The Cactarium HTTP API's JSON shapes, shared by the server and cactup's client. Serde only: no
//! HTTP client here. `docs/api.md` describes each endpoint with examples.
//!
//! Every response carries an `api-generation` header ([`API_GENERATION_HEADER`]); a client that
//! sees a generation it does not know should ask the user to upgrade rather than guess. Errors are
//! [`ErrorBody`] with a non-2xx status.
//!
//! | Request | Response |
//! |---|---|
//! | `GET /api/v1/resolve?req=owner/name[/sim]@[track:]range` | [`Resolved`] |
//! | `GET /api/v1/entries/{owner}/{name}` | [`EntryInfo`] |
//! | `GET /api/v1/packages/{owner}/{name}/versions` | [`PackageVersions`] |
//! | `GET /api/v1/stacks/{owner}/{name}/releases` | [`StackReleases`] |
//! | `GET /api/v1/stacks/{owner}/{name}/releases/{version\|head\|master}` | [`crate::stack::FrozenStack`] |
//! | `GET /api/v1/sims/{owner}/{name}/{X.Y.Z}/download[?sim=<sim>]` | the `.sim` bytes, with [`SHA256_HEADER`] |
//! | `POST /api/v1/sims/{owner}/{name}` (bearer token, `.sim` body) | [`Published`] |
//! | `POST /api/v1/stacks/{owner}/{name}/releases` (bearer token, [`ReleaseRequest`]) | [`Published`] |
//! | `POST /api/v1/packages/{owner}/{name}/refresh` (bearer token) | [`Refreshed`] |
//! | `POST /api/v1/lookup` ([`LookupRequest`]) | [`LookedUp`] |
//! | `GET /api/v1/providers?implements=…\|capability=…\|function=…\|thorn=…` ([`ProvidersQuery`]) | [`Providers`] |
//! | `POST /api/v1/repos` (bearer token, [`RegisterRequest`]) | [`Registered`]; 409 `conflict` when there is nothing to add |
//! | `GET /api/v1/whoami` (bearer token) | [`WhoAmI`] |
//! | `POST /api/v1/downloads` (bearer token, [`DownloadSubmission`]) | [`DownloadReview`]; 409 [`DownloadConflict`] when it is known |
//!
//! For `releases/head` and `releases/master` the [`FrozenStack`](crate::stack::FrozenStack) has
//! repos without a `commit` (fetch their branch, or for `master` the default branch), its `stack`
//! is `owner/name@head` or `@master`, and [`UNPINNED_HEADER`] is set.
//!
//! **Multi-sim entries.** A sim entry whose manifest lists `[sim.files]` holds several sims per
//! version, and each is always named: `owner/name/sim[@range]`. `/resolve` takes that form and
//! answers with the newest version in range that contains the sim ([`Resolved::sim`], and
//! [`Resolved::newer_without_sim`] when a newer version in range dropped it); the download takes
//! `?sim=<sim>` ([`SIM_QUERY`]), which such an entry requires. A bare `owner/name` for one is
//! refused with the list of its sims, and `owner/name/sim` for a one-sim entry is refused too:
//! the manifest's form, never the count, decides the address. [`EntryInfo::sims`] lists the sims
//! of the newest version.
//!
//! **What awaits review.** A request that says [`INCLUDE_UNREVIEWED_HEADER`]` = true` (CactUp's
//! `--risky-allow-untrusted-artifacts`) may also be served versions awaiting review, as the
//! server's policy allows: an admin's token always, anyone when the site serves unreviewed code.
//! An answer that serves something awaiting review says [`UNREVIEWED_HEADER`]` = true`, and each
//! such version says `review = "pending"` ([`VersionInfo::review`]).

use serde::{Deserialize, Serialize};

use crate::ccl::{Need, ThornFacts};
use crate::slug::{is_commit_id, valid_thorn};
use crate::{Error, Result};

/// The API generation this crate describes.
pub const API_GENERATION: u32 = 1;
pub const API_GENERATION_HEADER: &str = "api-generation";
/// The sha256 of a downloaded `.sim`, hex.
pub const SHA256_HEADER: &str = "x-sha256";
/// Set (to `true`) on a stack response that is not a release: `@head` or `@master`.
pub const UNPINNED_HEADER: &str = "x-cactarium-unpinned";
/// On every `@head`/`@master` response: `true` when the stack is not trusted (neither the entry
/// nor its owner), whose branch tips are never reviewed, else `false`. Releases do not carry it.
pub const UNTRUSTED_HEADER: &str = "x-cactarium-untrusted";
/// On a stack response: where to go for help with the stack (its declared contact, else its repo).
pub const CONTACT_HEADER: &str = "x-cactarium-contact";
/// The path prefix of every endpoint.
pub const API_PREFIX: &str = "/api/v1";
/// The download's query parameter naming one sim of a multi-sim entry.
pub const SIM_QUERY: &str = "sim";
/// The request header asking to be served what awaits review too, set to `true` (CactUp sends it
/// for `--risky-allow-untrusted-artifacts`). Who gets what is the server's policy.
pub const INCLUDE_UNREVIEWED_HEADER: &str = "x-cactarium-include-unreviewed";
/// On an answer that serves something awaiting review (only ever to a request that asked with
/// [`INCLUDE_UNREVIEWED_HEADER`]): `true`.
pub const UNREVIEWED_HEADER: &str = "x-cactarium-unreviewed";
/// The most pins and downloads, together, one [`LookupRequest`] may ask about.
pub const MAX_LOOKUP_PINS: usize = 1000;

/// What went wrong, with a message meant to be shown to the user as is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    /// `not-found`, `unauthorized`, `forbidden`, `bad-request`, `conflict`, `rate-limited` or
    /// `internal`.
    pub code: String,
    pub message: String,
}

/// One version of a package or a stack release, as `/resolve` and the version lists give it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct VersionInfo {
    /// `1.2.3`, `mixed-precision:1.2.3`, or a sim's `3`.
    pub version: String,
    /// `None` on the default track.
    pub track: Option<String>,
    /// The git tag the version was frozen from (`None` for curated and uploaded versions).
    pub tag: Option<String>,
    /// The frozen commit (packages and repo-backed stacks).
    pub commit: Option<String>,
    pub tree: Option<String>,
    pub yanked: bool,
    pub yank_reason: Option<String>,
    /// `ok`, `invalid`, `tag-moved` (the tag points elsewhere upstream now; the frozen commit is
    /// still what counts) or `upstream-missing`.
    pub state: String,
    /// Named by Cactarium admins for a repo whose authors have not published it.
    pub curated: bool,
    /// Unix seconds.
    pub published: i64,
    /// `pending` while the version awaits an admin's review (served only to a request that asked
    /// with [`INCLUDE_UNREVIEWED_HEADER`]); absent once approved, and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
}

/// `/resolve`: the best usable version matching the request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Resolved {
    /// The exact version: `owner/name@[track:]X.Y.Z` (sims included), or
    /// `owner/name/sim@X.Y.Z` for one sim of a multi-sim entry.
    pub slug: String,
    /// `package`, `stack` or `sim`.
    pub kind: String,
    /// The repo to fetch from (packages and repo-backed stacks).
    pub url: Option<String>,
    /// Where to go for help or to report problems with this version: its declared contact, else
    /// the repo.
    pub contact: Option<String>,
    /// The sim chosen, for a multi-sim entry: pass it to the download as `?sim=`. Absent otherwise
    /// (and from servers before multi-sim entries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sim: Option<String>,
    /// For a multi-sim entry: the newest version in the requested range when it is newer than the
    /// one chosen and no longer contains the sim (it was dropped), so a client can warn. Absent
    /// when the chosen version is the newest in range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newer_without_sim: Option<String>,
    #[serde(flatten)]
    pub version: VersionInfo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct EntryInfo {
    pub slug: String,
    pub kind: String,
    pub summary: Option<String>,
    pub verified: bool,
    pub curated: bool,
    pub repo: Option<String>,
    /// Where to go for help or to report problems: the declared contact, else the repo.
    pub contact: Option<String>,
    /// How many users saved it.
    pub hearts: i64,
    /// Track names, the default track as "".
    pub tracks: Vec<String>,
    pub versions: Vec<String>,
    /// For a multi-sim entry: the sims of its newest version, each addressed `owner/name/sim`.
    /// Empty otherwise (and from servers before multi-sim entries).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sims: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ThornInfo {
    pub arrangement: String,
    pub thorn: String,
    /// The `!REPO_PATH` to write for this thorn's checkout (`Arrangement/Thorn`) in a section that
    /// clones the package's repo, derived by the Cactarium from where the thorn sits in the repo:
    /// `$1/$2` for `Arrangement/Thorn/`, `$2` for a thorn at the root (`path/$2` for
    /// `path/Thorn/`), and `../$1-$2` for a repo that is itself the thorn, whose clone must then be
    /// named `Arrangement-Thorn` (the Einstein Toolkit's convention for ExternalLibraries).
    pub repo_path: String,
    pub facts: ThornFacts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ClaimInfo {
    /// `dependency`, `works-with`, `incompatible-with` or `fork-of`.
    pub kind: String,
    /// `owner/name`.
    pub target: String,
    /// What followed `@` in the target: a version range (or, failing that, a commit).
    pub range: Option<String>,
    /// What followed `#` in the target: a commit, full or abbreviated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    /// A dependency's `needs`: thorns of the target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// A dependency on a library: where its checkout goes, relative to this package's repo root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// A dependency's `needed-by`: thorns of this package; empty means every thorn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needed_by: Vec<String>,
    /// An `any-of` dependency's alternatives, in the author's order (`target`, `range`, `sha` and
    /// `needs` above then repeat the first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<AlternativeInfo>,
    pub note: Option<String>,
}

/// One alternative of an `any-of` dependency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AlternativeInfo {
    /// `owner/name`.
    pub target: String,
    pub range: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    pub needs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PackageVersion {
    #[serde(flatten)]
    pub version: VersionInfo,
    pub url: String,
    /// A Cactus flesh (et/cactus or a fork of it) rather than thorns: `thorns` is empty, and the
    /// repo is checked out as the Cactus root.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flesh: bool,
    /// A library that thorns build against rather than thorns (`thorns` is empty): a dependency
    /// on it says where the checkout goes (`into` in [`ClaimInfo`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub library: bool,
    pub thorns: Vec<ThornInfo>,
    pub claims: Vec<ClaimInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageVersions {
    pub slug: String,
    /// Newest first within each track.
    pub versions: Vec<PackageVersion>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackReleases {
    pub slug: String,
    pub releases: Vec<VersionInfo>,
}

/// `POST …/stacks/{owner}/{name}/releases`: cut a snapshot release of a stack defined on the site.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseRequest {
    /// The `cactarium-stack.toml` text.
    pub definition: String,
    /// The stack's own thornlist, if it has one.
    pub thornlist: Option<String>,
    /// `1.2.3` or `track:1.2.3`.
    pub version: String,
}

/// What a publish created.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Published {
    /// `owner/name@[track:]X.Y.Z`.
    pub slug: String,
    /// The page on the site.
    pub url: String,
    /// `public`, or `pending` while it waits for review.
    pub visibility: String,
    /// Things a reviewer will look at; none blocked publishing.
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Refreshed {
    pub slug: String,
    /// Tags found and stored by this refresh, whether published at once or awaiting review.
    pub new_versions: usize,
    /// The entry's versions waiting for an admin's review (this refresh's and any earlier ones);
    /// not served until approved. Absent from older servers, so it defaults to 0.
    #[serde(default)]
    pub awaiting_review: usize,
}

/// `POST /lookup`: which package, if any, each pin belongs to. No token needed. At most
/// [`MAX_LOOKUP_PINS`] pins and downloads together.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookupRequest {
    /// Repos pinned to a commit, as a `.sim` or a thornlist pins them.
    #[serde(default)]
    pub pins: Vec<PinQuery>,
    /// Files pinned to a sha256 (a tarball component, a `.sim` input's download). Absent from
    /// requests that ask about none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downloads: Vec<DownloadQuery>,
}

/// A repo pinned to a commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinQuery {
    /// The repo, spelled as the pin spells it; matched with [`crate::url::normalize_repo_url`].
    pub url: String,
    /// A full commit id: 40 or 64 lowercase hex digits.
    pub commit: String,
}

/// A file pinned to its content hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadQuery {
    pub url: String,
    /// 64 lowercase hex digits.
    pub sha256: String,
}

impl LookupRequest {
    /// The checks a server makes before looking anything up: at most [`MAX_LOOKUP_PINS`] entries,
    /// each with a URL, full commit ids and sha256s.
    pub fn validate(&self) -> Result<()> {
        let n = self.pins.len() + self.downloads.len();
        if n > MAX_LOOKUP_PINS {
            return Err(Error::invalid(format!("{n} pins in one lookup; at most {MAX_LOOKUP_PINS}")));
        }
        for p in &self.pins {
            if p.url.trim().is_empty() {
                return Err(Error::invalid("a pin without a url"));
            }
            if !is_commit_id(&p.commit) {
                return Err(Error::invalid(format!(
                    "{}: '{}' is not a full lowercase commit id (40 or 64 hex digits)",
                    p.url, p.commit
                )));
            }
        }
        for d in &self.downloads {
            if d.url.trim().is_empty() {
                return Err(Error::invalid("a download without a url"));
            }
            if !crate::sim::is_hex(&d.sha256, &[64]) {
                return Err(Error::invalid(format!("{}: '{}' is not a lowercase sha256", d.url, d.sha256)));
            }
        }
        Ok(())
    }
}

/// `POST /lookup`'s answer: one entry per pin and per download, each list in request order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookedUp {
    #[serde(default)]
    pub pins: Vec<PinInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downloads: Vec<DownloadInfo>,
}

/// What the Cactarium knows of one pinned commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PinInfo {
    /// As asked.
    pub url: String,
    /// As asked.
    pub commit: String,
    /// The package whose repo this is (`owner/name`); `None` when no package's is.
    pub package: Option<String>,
    /// The version that froze this commit (`2026.5.1`, `mixed-precision:1.2.0`); `None` when none
    /// did (the pin is then `package#commit`) or there is no package.
    pub version: Option<String>,
    /// The package is a Cactus flesh (`et/cactus` or a fork of it).
    #[serde(default)]
    pub flesh: bool,
    /// The `[[fork-of]]` targets the package declares at this commit, as `owner/name@<range>`
    /// (`et/carpet@=2026.5.1`). Empty when it declares none or there is no package.
    #[serde(default)]
    pub fork_of: Vec<String>,
    /// Whether the package's repo has the commit: `true` (a version froze it, or the repo's
    /// history has it), `false` (it does not), `None` when there is no package or the history
    /// could not be read just now.
    pub commit_known: Option<bool>,
}

/// What the Cactarium knows of one pinned download.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadInfo {
    /// As asked.
    pub url: String,
    /// As asked.
    pub sha256: String,
    /// `true` when an admin approved this URL with this content ([`DownloadSubmission`]),
    /// `false` while it awaits review or after it was rejected, `None` when it was never
    /// submitted.
    pub reviewed: Option<bool>,
}

/// `GET /providers`: the thorns of published packages that meet one CCL requirement. Exactly one
/// of the four is given. Each maps to a [`Need`] ([`ProvidersQuery::need`], [`ProvidersQuery::of`]),
/// so the server answers with [`crate::ccl::meets`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvidersQuery {
    /// Thorns that `IMPLEMENTS` it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implements: Option<String>,
    /// Thorns that `PROVIDES` the capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    /// Thorns that `PROVIDES FUNCTION` it (an aliased function).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    /// Thorns of that name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thorn: Option<String>,
}

/// A CCL name: `[A-Za-z_][A-Za-z0-9_]*`, which never needs escaping in a query string.
fn ccl_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl ProvidersQuery {
    /// The query that finds what meets `need`.
    pub fn of(need: &Need) -> ProvidersQuery {
        let name = Some(need.name().to_owned());
        match need {
            Need::Implementation(_) => ProvidersQuery { implements: name, ..Default::default() },
            Need::Capability(_) => ProvidersQuery { capability: name, ..Default::default() },
            Need::Function(_) => ProvidersQuery { function: name, ..Default::default() },
            Need::Thorn(_) => ProvidersQuery { thorn: name, ..Default::default() },
        }
    }

    /// The requirement asked about: exactly one parameter, a CCL name (a thorn name for `thorn`).
    pub fn need(&self) -> Result<Need> {
        let given: Vec<Need> = [
            self.implements.clone().map(Need::Implementation),
            self.capability.clone().map(Need::Capability),
            self.function.clone().map(Need::Function),
            self.thorn.clone().map(Need::Thorn),
        ]
        .into_iter()
        .flatten()
        .collect();
        let [need] = <[Need; 1]>::try_from(given)
            .map_err(|_| Error::invalid("give exactly one of implements, capability, function or thorn"))?;
        let ok = match &need {
            Need::Thorn(t) => valid_thorn(t),
            other => ccl_name(other.name()),
        };
        if !ok {
            return Err(Error::invalid(format!("'{}' is not a CCL name", need.name())));
        }
        Ok(need)
    }

    /// The query string, without the `?`: `implements=Driver`. Errors as [`Self::need`] does.
    pub fn to_query_string(&self) -> Result<String> {
        let need = self.need()?;
        let key = match need {
            Need::Implementation(_) => "implements",
            Need::Capability(_) => "capability",
            Need::Function(_) => "function",
            Need::Thorn(_) => "thorn",
        };
        Ok(format!("{key}={}", need.name()))
    }
}

/// `GET /providers`' answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Providers {
    pub providers: Vec<Provider>,
}

/// A thorn of a package version that meets the requirement asked about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    /// `owner/name`.
    pub package: String,
    /// `2026.5.1` or `mixed-precision:1.2.0`.
    pub version: String,
    pub arrangement: String,
    pub thorn: String,
}

/// `POST /repos`: register the manifests a repo carries as entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RegisterRequest {
    pub url: String,
    /// Read the manifests at this tag rather than each entry's newest own tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// The namespace to register under (an organization the user belongs to); else the user's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Register only the entries of these names (each manifest's `name`); the others are skipped.
    /// Empty means every one the repo carries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
    /// Say what registering would do, writing nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
}

/// What `POST /repos` registered (or, for a dry run, would register): every entry on the repo.
/// When one `fails`, nothing is registered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registered {
    pub entries: Vec<RegisteredEntry>,
    /// Things to know that blocked nothing: a version that waits for a dependency, a manifest
    /// left out at the tag named.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// One entry of a registration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredEntry {
    /// `owner/name`.
    pub slug: String,
    /// `package`, `stack` or `sim`.
    pub kind: String,
    /// The manifest it is read from: `cactarium.toml`, `cactarium-stack.toml` or
    /// `cactarium-sim.toml`.
    pub file: String,
    /// The tag its first version is read at.
    pub tag: Option<String>,
    /// That version: `1.2.0` or `mixed-precision:1.2.0`.
    pub version: Option<String>,
    /// Which tags are its versions, in words: `tags vX.Y.Z from 1.0.0`.
    pub rule: Option<String>,
    /// `pending` when its first version awaits an admin's review, `approved` when it is served
    /// at once.
    pub review: Option<String>,
    /// `new` (registered now, or would be by a dry run), `existing` (on the repo already, left as
    /// it is), `skipped` (not registered now: `only` leaves it out, or the tag named is not one of
    /// its versions) or `fails` (reading it fails, so nothing is registered).
    pub status: String,
    /// Why it was skipped or fails, or what else to know about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `GET /whoami`: who the bearer token belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhoAmI {
    /// The username, which is also the user's own namespace.
    pub user: String,
    /// The organizations (namespaces) the user may publish under.
    #[serde(default)]
    pub orgs: Vec<String>,
    /// A Cactarium admin.
    #[serde(default)]
    pub admin: bool,
}

/// `POST /downloads`: submit a download (a `.sim` input, a tarball component) for review, so pins
/// to it can be trusted ([`DownloadInfo::reviewed`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadSubmission {
    pub url: String,
    /// The content's sha256, 64 lowercase hex digits.
    pub sha256: String,
    /// For the reviewer: what it is and why it can be trusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A submitted download's review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadReview {
    pub id: i64,
    /// `pending`, `approved` or `rejected`.
    pub review: String,
}

/// The 409 answer to a download submitted before: an [`ErrorBody`] (code `conflict`) that also
/// carries the known [`DownloadReview`], so it reads as either.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadConflict {
    pub error: ErrorDetail,
    #[serde(flatten)]
    pub known: DownloadReview,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_is_flat_json() {
        let r = Resolved {
            slug: "max/carpetx@mixed-precision:1.4.0".into(),
            kind: "package".into(),
            url: Some("https://github.com/max-morris/CarpetX.git".into()),
            contact: Some("https://github.com/max-morris/CarpetX/issues".into()),
            sim: None,
            newer_without_sim: None,
            version: VersionInfo {
                version: "mixed-precision:1.4.0".into(),
                track: Some("mixed-precision".into()),
                tag: Some("mixed-precision/v1.4.0".into()),
                commit: Some("3f2c9e1a00112233445566778899aabbccddeeff".into()),
                tree: None,
                yanked: false,
                yank_reason: None,
                state: "ok".into(),
                curated: false,
                published: 0,
                review: None,
            },
        };
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["commit"], "3f2c9e1a00112233445566778899aabbccddeeff");
        assert_eq!(j["yank-reason"], serde_json::Value::Null);
        assert!(j.get("sim").is_none() && j.get("newer-without-sim").is_none(), "absent unless a multi-sim entry");
        assert!(j.get("review").is_none(), "absent once approved");
        assert_eq!(serde_json::from_value::<Resolved>(j).unwrap(), r);
    }

    /// One sim of a multi-sim entry, chosen below a newer version that dropped it.
    #[test]
    fn resolved_names_the_sim() {
        let j = serde_json::json!({
            "slug": "et/cottonmouth-sims/qc0-z4c@1.0.0", "kind": "sim", "url": null, "contact": null,
            "sim": "qc0-z4c", "newer-without-sim": "1.1.0",
            "version": "1.0.0", "track": null, "tag": "sim-v1.0.0", "commit": null, "tree": null,
            "yanked": false, "yank-reason": null, "state": "ok", "curated": false, "published": 0
        });
        let r: Resolved = serde_json::from_value(j).unwrap();
        assert_eq!(r.sim.as_deref(), Some("qc0-z4c"));
        assert_eq!(r.newer_without_sim.as_deref(), Some("1.1.0"));
        assert_eq!(r.version.tag.as_deref(), Some("sim-v1.0.0"));
    }

    const SHA: &str = "3f2c9e1a00112233445566778899aabbccddeeff";

    /// A version awaiting review says so; an approved one leaves `review` out.
    #[test]
    fn version_info_review() {
        let j = serde_json::json!({
            "version": "1.0.0", "track": null, "tag": "v1.0.0", "commit": null, "tree": null,
            "yanked": false, "yank-reason": null, "state": "ok", "curated": false, "published": 0
        });
        let v: VersionInfo = serde_json::from_value(j.clone()).unwrap();
        assert_eq!(v.review, None, "older servers send none");
        let mut pending = j;
        pending["review"] = "pending".into();
        let v: VersionInfo = serde_json::from_value(pending.clone()).unwrap();
        assert_eq!(v.review.as_deref(), Some("pending"));
        assert_eq!(serde_json::to_value(&v).unwrap(), pending);
        assert_eq!(INCLUDE_UNREVIEWED_HEADER, "x-cactarium-include-unreviewed");
        assert_eq!(UNREVIEWED_HEADER, "x-cactarium-unreviewed");
    }

    #[test]
    fn lookup_shapes() {
        let req: LookupRequest = serde_json::from_value(serde_json::json!({
            "pins": [{ "url": "https://github.com/x/y.git", "commit": SHA }]
        }))
        .unwrap();
        assert!(req.downloads.is_empty());
        req.validate().unwrap();
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            serde_json::json!({ "pins": [{ "url": "https://github.com/x/y.git", "commit": SHA }] }),
            "a request about no downloads leaves them out"
        );

        let answer = serde_json::json!({
            "pins": [
                { "url": "https://github.com/x/y.git", "commit": SHA, "package": "et/carpet",
                  "version": "2026.5.1", "flesh": false, "fork-of": ["et/carpet@=2026.5.1"], "commit-known": true },
                { "url": "https://example.org/z", "commit": SHA, "package": null, "version": null,
                  "flesh": false, "fork-of": [], "commit-known": null }
            ],
            "downloads": [{ "url": "https://example.org/id.tar.gz", "sha256": "ab".repeat(32), "reviewed": null }]
        });
        let got: LookedUp = serde_json::from_value(answer.clone()).unwrap();
        assert_eq!(got.pins[0].package.as_deref(), Some("et/carpet"));
        assert_eq!(got.pins[0].fork_of, ["et/carpet@=2026.5.1"]);
        assert_eq!(got.pins[0].commit_known, Some(true));
        assert_eq!(got.pins[1].commit_known, None);
        assert_eq!(got.downloads[0].reviewed, None);
        assert_eq!(serde_json::to_value(&got).unwrap(), answer);
        // A newer server's extra field is no error; an older one's missing downloads are none.
        let lean: LookedUp = serde_json::from_value(serde_json::json!({ "pins": [], "extra": 1 })).unwrap();
        assert!(lean.downloads.is_empty());
    }

    #[test]
    fn lookup_limits() {
        let pin = |commit: &str| PinQuery { url: "https://github.com/x/y".into(), commit: commit.into() };
        let mut req = LookupRequest { pins: vec![pin(SHA); MAX_LOOKUP_PINS], downloads: vec![] };
        req.validate().unwrap();
        req.downloads.push(DownloadQuery { url: "https://x.org/a.tar".into(), sha256: "0".repeat(64) });
        assert!(req.validate().unwrap_err().to_string().contains("at most 1000"));
        for bad in ["3f2c9e1", &SHA.to_uppercase(), &format!("{SHA}00")] {
            let req = LookupRequest { pins: vec![pin(bad)], downloads: vec![] };
            assert!(req.validate().is_err(), "{bad}");
        }
        assert!(LookupRequest { pins: vec![pin(&"a".repeat(64))], downloads: vec![] }.validate().is_ok(), "sha-256 repos");
        let bad_sha = LookupRequest { pins: vec![], downloads: vec![DownloadQuery { url: "u".into(), sha256: "x".into() }] };
        assert!(bad_sha.validate().is_err());
    }

    #[test]
    fn providers_query() {
        let q = ProvidersQuery::of(&Need::Implementation("Driver".into()));
        assert_eq!(q.to_query_string().unwrap(), "implements=Driver");
        assert_eq!(q.need().unwrap(), Need::Implementation("Driver".into()));
        assert_eq!(serde_json::to_value(&q).unwrap(), serde_json::json!({ "implements": "Driver" }));
        for need in [Need::Capability("HDF5".into()), Need::Function("GetRefinementLevel".into()), Need::Thorn("CarpetX".into())] {
            assert_eq!(ProvidersQuery::of(&need).need().unwrap(), need);
        }
        assert_eq!(ProvidersQuery::of(&Need::Capability("HDF5".into())).to_query_string().unwrap(), "capability=HDF5");
        assert!(ProvidersQuery::default().need().is_err(), "none");
        let two = ProvidersQuery { implements: Some("A".into()), thorn: Some("B".into()), ..Default::default() };
        assert!(two.need().is_err(), "more than one");
        let odd = ProvidersQuery { capability: Some("a&b=c".into()), ..Default::default() };
        assert!(odd.need().is_err() && odd.to_query_string().is_err());
        let answer = serde_json::json!({ "providers": [
            { "package": "et/carpet", "version": "2026.5.1", "arrangement": "Carpet", "thorn": "Carpet" }
        ]});
        let p: Providers = serde_json::from_value(answer.clone()).unwrap();
        assert_eq!(p.providers[0].thorn, "Carpet");
        assert_eq!(serde_json::to_value(&p).unwrap(), answer);
    }

    #[test]
    fn register_shapes() {
        let bare: RegisterRequest = serde_json::from_value(serde_json::json!({ "url": "https://github.com/x/y" })).unwrap();
        assert!(bare.tag.is_none() && bare.owner.is_none() && bare.only.is_empty() && !bare.dry_run);
        assert_eq!(serde_json::to_value(&bare).unwrap(), serde_json::json!({ "url": "https://github.com/x/y" }));
        let full = serde_json::json!({ "url": "u", "tag": "v1.0.0", "owner": "et", "only": ["cottonmouth"], "dry-run": true });
        let r: RegisterRequest = serde_json::from_value(full.clone()).unwrap();
        assert!(r.dry_run);
        assert_eq!(serde_json::to_value(&r).unwrap(), full);

        let answer = serde_json::json!({
            "entries": [
                { "slug": "et/cottonmouth", "kind": "package", "file": "cactarium.toml", "tag": "v2026.5.1",
                  "version": "2026.5.1", "rule": "tags vX.Y.Z", "review": "pending", "status": "new" },
                { "slug": "et/cottonmouth-sims", "kind": "sim", "file": "cactarium-sim.toml", "tag": null,
                  "version": null, "rule": "tags sim-vX.Y.Z", "review": null, "status": "skipped",
                  "note": "only leaves it out" }
            ],
            "warnings": ["et/cottonmouth@2026.5.1 waits: …"]
        });
        let got: Registered = serde_json::from_value(answer.clone()).unwrap();
        assert_eq!(got.entries[1].note.as_deref(), Some("only leaves it out"));
        assert_eq!(serde_json::to_value(&got).unwrap(), answer);
    }

    #[test]
    fn whoami_and_downloads() {
        let w: WhoAmI = serde_json::from_str(r#"{"user":"max","orgs":["et"],"admin":false}"#).unwrap();
        assert_eq!((w.user.as_str(), w.orgs.as_slice(), w.admin), ("max", ["et".to_owned()].as_slice(), false));

        let sub: DownloadSubmission =
            serde_json::from_value(serde_json::json!({ "url": "https://x.org/id.tar.gz", "sha256": "ab".repeat(32) })).unwrap();
        assert!(sub.note.is_none());
        let r: DownloadReview = serde_json::from_str(r#"{"id":7,"review":"pending"}"#).unwrap();
        assert_eq!(r.review, "pending");

        // A 409 reads as an ErrorBody and as the known review.
        let conflict = r#"{"error":{"code":"conflict","message":"already submitted"},"id":7,"review":"approved"}"#;
        assert_eq!(serde_json::from_str::<ErrorBody>(conflict).unwrap().error.code, "conflict");
        let c: DownloadConflict = serde_json::from_str(conflict).unwrap();
        assert_eq!(c.known, DownloadReview { id: 7, review: "approved".into() });
        assert_eq!(serde_json::from_str::<DownloadReview>(conflict).unwrap().id, 7);
        assert_eq!(serde_json::to_string(&c).unwrap(), conflict);
    }

    #[test]
    fn error_body_shape() {
        let e: ErrorBody = serde_json::from_str(r#"{"error":{"code":"not-found","message":"Not found"}}"#).unwrap();
        assert_eq!(e.error.code, "not-found");
    }
}
