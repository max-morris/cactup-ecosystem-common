//! The Cactarium HTTP API's JSON shapes, shared by the server and cactup's client. Serde only: no
//! HTTP client here.
//!
//! Every response carries an `api-generation` header ([`API_GENERATION_HEADER`]); a client that
//! sees a generation it does not know should ask the user to upgrade rather than guess. Errors are
//! [`ErrorBody`] with a non-2xx status.
//!
//! | Request | Response |
//! |---|---|
//! | `GET /api/v1/resolve?req=owner/name@[track:]range` | [`Resolved`] |
//! | `GET /api/v1/entries/{owner}/{name}` | [`EntryInfo`] |
//! | `GET /api/v1/packages/{owner}/{name}/versions` | [`PackageVersions`] |
//! | `GET /api/v1/stacks/{owner}/{name}/releases` | [`StackReleases`] |
//! | `GET /api/v1/stacks/{owner}/{name}/releases/{version\|head\|master}` | [`crate::stack::FrozenStack`] |
//! | `GET /api/v1/sims/{owner}/{name}/{N}/download` | the `.sim` bytes, with [`SHA256_HEADER`] |
//! | `POST /api/v1/sims/{owner}/{name}` (bearer token, `.sim` body) | [`Published`] |
//! | `POST /api/v1/stacks/{owner}/{name}/releases` (bearer token, [`ReleaseRequest`]) | [`Published`] |
//! | `POST /api/v1/packages/{owner}/{name}/refresh` (bearer token) | [`Refreshed`] |
//!
//! For `releases/head` and `releases/master` the [`FrozenStack`](crate::stack::FrozenStack) has
//! repos without a `commit` (fetch their branch, or for `master` the default branch), its `stack`
//! is `owner/name@head` or `@master`, and [`UNPINNED_HEADER`] is set.

use serde::{Deserialize, Serialize};

use crate::ccl::ThornFacts;

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
}

/// `/resolve`: the best usable version matching the request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Resolved {
    /// The exact version: `owner/name@[track:]X.Y.Z` (sims included).
    pub slug: String,
    /// `package`, `stack` or `sim`.
    pub kind: String,
    /// The repo to fetch from (packages and repo-backed stacks).
    pub url: Option<String>,
    /// Where to go for help or to report problems with this version: its declared contact, else
    /// the repo.
    pub contact: Option<String>,
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
    /// A dependency's `needed-by`: thorns of this package; empty means every thorn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needed_by: Vec<String>,
    pub note: Option<String>,
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
            },
        };
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["commit"], "3f2c9e1a00112233445566778899aabbccddeeff");
        assert_eq!(j["yank-reason"], serde_json::Value::Null);
        assert_eq!(serde_json::from_value::<Resolved>(j).unwrap(), r);
    }

    #[test]
    fn error_body_shape() {
        let e: ErrorBody = serde_json::from_str(r#"{"error":{"code":"not-found","message":"Not found"}}"#).unwrap();
        assert_eq!(e.error.code, "not-found");
    }
}
