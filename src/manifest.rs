//! Manifests committed in a repo and read at every version tag.
//!
//! - [`PACKAGE_FILE`] (`cactarium.toml`) describes a **package** (`[package]`: the repo's thorns).
//! - [`STACK_FILE`] (`cactarium-stack.toml`) describes a **stack** (`[stack]`, see
//!   [`crate::stack`]). A repo holding one is a *manifest repo*; it may hold nothing else, or be a
//!   package repo that also carries a stack.
//! - [`SIM_FILE`] (`cactarium-sim.toml`) publishes a **sim** from the repo (`[sim]`): the `.sim`
//!   file it names, read at each version tag. Uploads remain the other way to publish a sim.
//!
//! A repo can carry any of the three; the registry looks for each at every tag. Dependencies
//! (`[[dependency]]`), claims (`[[fork-of]]`, `[[works-with]]`, `[[incompatible-with]]`) and
//! `[[caveat]]`s may appear in either file.
//!
//! CCL says *what* a thorn needs (an implementation, a capability, a function); a
//! `[[dependency]]` says *where it comes from*: which package, at which versions or commit. It
//! never repeats the CCL. Its `target` is `owner/pkg`, `owner/pkg@<range>` or `owner/pkg#<sha>`;
//! `needs` names thorns of the target that are needed whatever the CCL says (to choose among
//! providers, or for a need the CCL can't express), and `needed-by` names the thorns of this
//! package the dependency is for (default: every thorn).
//!
//! Admins use the same schema for the overlay manifest of a curated package, i.e. one whose
//! upstream repo has no `cactarium.toml` of its own.

use std::str::FromStr;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::name::{Name, Owner};
use crate::slug::Slug;
use crate::stack::StackSection;
use crate::version::{DEFAULT_TRACK_KEYWORD, TrackName, TrackedVersion};
use crate::{Error, MANIFEST_FORMAT, Result, generation};

/// The package manifest's file name, at the repo root.
pub const PACKAGE_FILE: &str = "cactarium.toml";
/// The stack manifest's file name, at the repo root.
pub const STACK_FILE: &str = "cactarium-stack.toml";
/// The sim manifest's file name, at the repo root.
pub const SIM_FILE: &str = "cactarium-sim.toml";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<StackSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sim: Option<SimSection>,
    /// Descriptions of the repo's tracks (tag prefixes), keyed by track name.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub tracks: IndexMap<TrackName, TrackInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fork_of: Vec<ForkOf>,
    #[serde(default, rename = "dependency", skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<Dependency>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub works_with: Vec<Claim>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incompatible_with: Vec<Claim>,
    #[serde(default, rename = "caveat", skip_serializing_if = "Vec::is_empty")]
    pub caveats: Vec<Caveat>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PackageSection {
    pub name: Name,
    /// Cactarium users or orgs allowed to publish this repo. Only someone who can push to the repo
    /// can put their name here, so this is the proof of repo control.
    pub owners: Vec<Owner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// An SPDX license expression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Where to go for help or to report problems: an `https://` URL (an issue tracker, a forum) or
    /// a `mailto:` address. Unset means the repo the artifact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    /// Path of the README to show, relative to the repo root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    /// The arrangement every thorn belongs to, whatever folder it sits in: needed when the thorns
    /// sit at the repo root (`Thorn/`), the repo is one thorn, or the folders don't name the
    /// arrangement (`implementations/GRHayLib`). Repos laid out as `Arrangement/Thorn/` don't
    /// need it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrangement: Option<String>,
    /// Where the thorns are, relative to the repo root, when not at it: the folder holding the
    /// `Arrangement/Thorn/` (or `Thorn/`) directories. Kranc keeps its thorns under
    /// `Auxiliary/Cactus`. Default: the repo root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// The thorns to publish, by directory name. Default: every directory with an interface.ccl.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thorns: Option<Vec<String>>,
    /// The track shown first on the package page: a track name, or "default" for unprefixed tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_track: Option<String>,
    /// How release tags become versions when they aren't `vX.Y.Z`, e.g.
    /// `ET_{major}_{minor}_v{patch}` (see [`crate::version::TagPattern`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_pattern: Option<String>,
    /// Release tags below this version aren't versions (say, releases from before the repo
    /// carried this file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<semver::Version>,
}

/// A sim published from its repo (`cactarium-sim.toml`): at each version tag, the `.sim` file it
/// names becomes that version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SimSection {
    pub name: Name,
    /// Cactarium users or orgs allowed to publish this repo's sim: the proof of repo control.
    pub owners: Vec<Owner>,
    /// The `.sim` file, relative to the repo root.
    pub file: String,
    /// Shown on the sim's page; default: the description inside the `.sim`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Where to go for help or to report problems; default: the `.sim`'s own contact, else the repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    /// How release tags become versions when they aren't `vX.Y.Z` (see
    /// [`crate::version::TagPattern`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_pattern: Option<String>,
    /// Release tags below this version aren't versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<semver::Version>,
}

impl SimSection {
    pub fn validate(&self) -> Result<()> {
        if self.owners.is_empty() {
            return Err(Error::invalid(format!("[sim] in {SIM_FILE} needs owners")));
        }
        let f = &self.file;
        let bad = f.is_empty()
            || f.starts_with('/')
            || f.contains('\\')
            || f.split('/').any(|c| c.is_empty() || c == "." || c == "..");
        if bad {
            return Err(Error::invalid(format!("[sim] file '{f}' must be a relative path inside the repo")));
        }
        if let Some(c) = &self.contact {
            validate_contact(c)?;
        }
        if let Some(t) = &self.tag_pattern {
            crate::version::TagPattern::from_str(t)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct TrackInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ForkOf {
    /// The upstream package, and the versions of it this package is a drop-in replacement for:
    /// `owner/name` (any version on its default track), `owner/name@<range>` or
    /// `owner/name#<sha>` (one commit). A fork and its upstream are never installed together;
    /// see [`ForkOf::stands_in_for`] for when the fork satisfies a dependency on the upstream.
    pub target: Slug,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ForkOf {
    /// Whether this fork satisfies `dep`, a dependency on the same upstream package: some
    /// published version of the upstream (`published`: each version with its commit) is one the
    /// dependency accepts and one the fork is a drop-in for. A dependency on one commit is
    /// satisfied only by a fork of that same commit. A dependency on another package is never
    /// satisfied.
    pub fn stands_in_for(&self, dep: &Slug, published: &[(TrackedVersion, Option<String>)]) -> bool {
        if self.target.owner != dep.owner || self.target.name != dep.name {
            return false;
        }
        let same_commit = |a: &str, b: &str| a.starts_with(b) || b.starts_with(a);
        let dep_sha = dep.sha.as_deref().or(dep.commit_fallback());
        let fork_sha = self.target.sha.as_deref().or(self.target.commit_fallback());
        if let Some(want) = dep_sha {
            return fork_sha.is_some_and(|have| same_commit(have, want));
        }
        let Ok(accepts) = dep.req() else { return false };
        published.iter().any(|(v, commit)| {
            accepts.matches(v)
                && match fork_sha {
                    Some(have) => commit.as_deref().is_some_and(|c| same_commit(c, have)),
                    None => self.target.req().is_ok_and(|covers| covers.matches(v)),
                }
        })
    }
}

/// A package or stack this one needs (`[[dependency]]`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Dependency {
    /// `owner/name`, `owner/name@<range>` or `owner/name#<sha>`. No `@`: every version on the
    /// default track.
    pub target: Slug,
    /// Thorns of the target that are needed whatever the CCL says. Empty: whichever of its thorns
    /// the CCL requirements of the `needed-by` thorns resolve to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// Thorns of this package the dependency is for. Empty: every thorn. Only in a `[package]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needed_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A declared relation to another package or stack (`[[works-with]]`, `[[incompatible-with]]`):
/// claims the registry displays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", try_from = "RawClaim")]
pub struct Claim {
    /// `owner/name`, `owner/name@<range>` or `owner/name#<sha>`. No `@`: every version on the
    /// default track.
    pub target: Slug,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A claim as written. Manifests from before ranges moved into `target` give them as `range`;
/// those are read, and written back with the range in `target`.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RawClaim {
    target: Slug,
    #[serde(default)]
    range: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

impl TryFrom<RawClaim> for Claim {
    type Error = Error;
    fn try_from(r: RawClaim) -> Result<Claim> {
        let mut target = r.target;
        if let Some(range) = r.range {
            if target.at.is_some() || target.sha.is_some() {
                return Err(Error::invalid(format!(
                    "claim target '{target}' already names versions; drop 'range' or put it in 'target'"
                )));
            }
            target.at = Some(range);
        }
        Ok(Claim { target, note: r.note })
    }
}

/// A note attached to one thorn, e.g. a platform requirement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Caveat {
    pub thorn: String,
    pub note: String,
}

/// A `contact` must be an https URL or a mailto: address.
pub fn validate_contact(c: &str) -> Result<()> {
    let ok = (c.starts_with("https://") && c.len() > "https://".len() || c.starts_with("mailto:") && c.contains('@'))
        && !c.chars().any(|ch| ch.is_whitespace() || ch.is_control());
    if ok { Ok(()) } else { Err(Error::invalid(format!("contact '{c}' must be an https:// URL or a mailto: address"))) }
}

/// A fork-of target: the upstream package, at the versions (a range) or the commit forked;
/// never a thorn or a branch tip.
fn fork_target(t: &Slug) -> Result<()> {
    if t.thorn.is_some() {
        return Err(Error::invalid(format!("fork-of target '{t}' names a thorn; name the package")));
    }
    if t.sha.is_some() || t.commit_fallback().is_some() {
        return Ok(());
    }
    t.req().map(|_| ())
}

/// A dependency or claim target: `owner/name`, at a range or a commit; never a thorn (a
/// dependency names thorns in `needs`) or a branch tip.
fn versioned_target(t: &Slug) -> Result<()> {
    if t.thorn.is_some() {
        return Err(Error::invalid(format!(
            "target '{t}' names a thorn; name the package, and its thorns in 'needs'"
        )));
    }
    if t.sha.is_some() || t.commit_fallback().is_some() {
        return Ok(());
    }
    t.req().map(|_| ())
}

fn thorn_list(what: &str, target: &Slug, thorns: &[String]) -> Result<()> {
    for (i, t) in thorns.iter().enumerate() {
        if !crate::slug::valid_thorn(t) {
            return Err(Error::invalid(format!("'{t}' in {what} of the dependency on '{target}' is not a thorn name")));
        }
        if thorns[..i].iter().any(|u| u.eq_ignore_ascii_case(t)) {
            return Err(Error::invalid(format!("'{t}' is listed twice in {what} of the dependency on '{target}'")));
        }
    }
    Ok(())
}

impl Manifest {
    /// Reads a manifest found in a repo, checking that the file holds the kind its name promises
    /// and names the owners allowed to publish it.
    pub fn from_repo_file(filename: &str, text: &str) -> Result<Manifest> {
        let m = Manifest::from_toml(text)?;
        match filename {
            PACKAGE_FILE if m.package.is_none() => {
                Err(Error::invalid(format!("{PACKAGE_FILE} must hold a [package]; a stack goes in {STACK_FILE}")))
            }
            STACK_FILE if m.stack.is_none() => {
                Err(Error::invalid(format!("{STACK_FILE} must hold a [stack]; a package goes in {PACKAGE_FILE}")))
            }
            STACK_FILE if m.stack.as_ref().is_some_and(|s| s.owners.is_empty()) => {
                Err(Error::invalid(format!("[stack] in {STACK_FILE} needs owners")))
            }
            SIM_FILE if m.sim.is_none() => Err(Error::invalid(format!("{SIM_FILE} must hold a [sim]"))),
            PACKAGE_FILE if m.sim.is_some() => {
                Err(Error::invalid(format!("[sim] belongs in {SIM_FILE}, not {PACKAGE_FILE}")))
            }
            STACK_FILE if m.sim.is_some() => {
                Err(Error::invalid(format!("[sim] belongs in {SIM_FILE}, not {STACK_FILE}")))
            }
            PACKAGE_FILE | STACK_FILE | SIM_FILE => Ok(m),
            other => Err(Error::invalid(format!("'{other}' is not a Cactarium manifest file name"))),
        }
    }

    pub fn from_toml(text: &str) -> Result<Manifest> {
        generation::check(text, "cactarium.toml", MANIFEST_FORMAT, MANIFEST_FORMAT)?;
        let m: Manifest = toml::from_str(text).map_err(|source| Error::Toml { what: "cactarium.toml", source })?;
        m.validate()?;
        Ok(m)
    }

    pub fn to_toml(&self) -> Result<String> {
        self.validate()?;
        toml::to_string(self).map_err(|source| Error::Serialize { what: "cactarium.toml", source })
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != MANIFEST_FORMAT {
            return Err(Error::invalid(format!("format must be {MANIFEST_FORMAT}")));
        }
        match (&self.package, &self.stack, &self.sim) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
                return Err(Error::invalid("a manifest holds one of [package], [stack] or [sim]"));
            }
            (None, None, None) => {
                return Err(Error::invalid("a manifest needs a [package], [stack] or [sim] section"));
            }
            (None, None, Some(s)) => s.validate()?,
            (Some(p), None, None) => {
                if p.owners.is_empty() {
                    return Err(Error::invalid("[package] owners is empty"));
                }
                if let Some(c) = &p.contact {
                    validate_contact(c)?;
                }
                if let Some(d) = &p.default_track
                    && d != DEFAULT_TRACK_KEYWORD
                {
                    TrackName::from_str(d)?;
                }
                if let Some(t) = &p.tag_pattern {
                    crate::version::TagPattern::from_str(t)?;
                }
                if let Some(r) = &p.root {
                    let path = std::path::Path::new(r);
                    let plain = path.components().all(|c| matches!(c, std::path::Component::Normal(_)));
                    if r.trim().is_empty() || !plain {
                        return Err(Error::invalid(format!(
                            "[package] root '{r}' must be a folder inside the repo, like \"Auxiliary/Cactus\""
                        )));
                    }
                }
            }
            (None, Some(s), None) => s.validate()?,
        }
        for f in &self.fork_of {
            fork_target(&f.target)?;
        }
        for c in self.works_with.iter().chain(&self.incompatible_with) {
            versioned_target(&c.target)?;
        }
        for d in &self.dependencies {
            versioned_target(&d.target)?;
            thorn_list("needs", &d.target, &d.needs)?;
            thorn_list("needed-by", &d.target, &d.needed_by)?;
            if d.needed_by.is_empty() {
                continue;
            }
            let Some(p) = &self.package else {
                return Err(Error::invalid(format!(
                    "needed-by in the dependency on '{}' names thorns, but only a [package] has thorns",
                    d.target
                )));
            };
            if let Some(own) = &p.thorns
                && let Some(t) = d.needed_by.iter().find(|t| !own.contains(t))
            {
                return Err(Error::invalid(format!(
                    "needed-by in the dependency on '{}' names '{t}', which is not one of this package's thorns",
                    d.target
                )));
            }
        }
        Ok(())
    }
}
