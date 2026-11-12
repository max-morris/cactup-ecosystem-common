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
//! package the dependency is for (default: every thorn). A dependency on a library (a package
//! with `library = true`, which has no thorns) gives `into` instead of `needs`: where its checkout
//! goes in this package's repo.
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
    /// The package is a Cactus flesh (the repo holds `src/main/flesh.cc` and `lib/sbin/CST`)
    /// rather than thorns: et/cactus, or a fork of it such as a mixed-precision flesh. It has no
    /// thorns, so no `thorns`, `arrangement` or `root`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flesh: bool,
    /// The package is a library that thorns build against (FUKA, which KadathThorn builds from
    /// source) rather than thorns or a flesh: nothing in it is checked for CCL, and it has no
    /// `thorns`, `exclude-thorns`, `arrangement` or `root`. A package that uses it says where its
    /// checkout goes with `into` on the dependency.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub library: bool,
    /// Where the thorns are, relative to the repo root, when not at it: the folder holding the
    /// `Arrangement/Thorn/` (or `Thorn/`) directories. Kranc keeps its thorns under
    /// `Auxiliary/Cactus`. Default: the repo root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Thorns the repo has but that aren't published, because they can't build: the Cactarium
    /// hosts only what works as delivered. CarpetIOF5 needs an F5 library nothing provides, for
    /// example. Not with `thorns`, which lists what is published instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_thorns: Vec<String>,
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
    /// The package depended on: `owner/name`, `owner/name@<range>` or `owner/name#<sha>`. No `@`:
    /// every version on the default track. Exactly one of `target` and `any-of`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Slug>,
    /// The thorns of `target` this package uses. Required in a `[package]` whose target has thorns
    /// (the Cactarium, which knows the target, refuses a version without it): the CCL says what a
    /// thorn needs, this says which thorn of which package supplies it, and the Cactarium checks
    /// that the two agree (every hard CCL requirement of a published thorn is met by the package
    /// itself or a dependency's `needs`). Left out for a target with no thorns: a flesh or a
    /// library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<Vec<String>>,
    /// For a library target: where its checkout goes, relative to this package's repo root
    /// (`src/fuka` for KadathThorn, whose build looks there). Required for a library, refused for
    /// anything else; not with `needs` or `any-of`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// Interchangeable packages, each with the thorns it supplies, in the author's order of
    /// preference: any one of them meets the dependency. A resolver takes one already installed,
    /// else one the rest of the plan brings in, else the first that conflicts with nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_of: Vec<Alternative>,
    /// Thorns of this package the dependency is for. Empty: every thorn. Only in a `[package]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needed_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One of a dependency's interchangeable packages (`any-of`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Alternative {
    /// `owner/name`, `owner/name@<range>` or `owner/name#<sha>`.
    pub target: Slug,
    /// The thorns of this alternative that the package uses (left out only for a flesh).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<Vec<String>>,
}

impl Dependency {
    /// Each package the dependency may be met by, with its `needs`: the target, or every
    /// alternative.
    pub fn options(&self) -> Vec<(&Slug, Option<&[String]>)> {
        match &self.target {
            Some(t) => vec![(t, self.needs.as_deref())],
            None => self.any_of.iter().map(|a| (&a.target, a.needs.as_deref())).collect(),
        }
    }

    /// The dependency as people read it: `et/carpet`, or `any of et/carpet, et/cactuspugh`.
    pub fn label(&self) -> String {
        match &self.target {
            Some(t) => t.to_string(),
            None => format!(
                "any of {}",
                self.any_of.iter().map(|a| a.target.to_string()).collect::<Vec<_>>().join(", ")
            ),
        }
    }
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

/// A `needed-by` list: thorn names, of this package (only a `[package]` has thorns).
fn thorn_names_of_this_package(package: &Option<PackageSection>, what: &str, of: &str, thorns: &[String]) -> Result<()> {
    if thorns.is_empty() {
        return Ok(());
    }
    let Some(p) = package else {
        return Err(Error::invalid(format!("{what} of {of} names thorns, but only a [package] has thorns")));
    };
    for (i, t) in thorns.iter().enumerate() {
        if !crate::slug::valid_thorn(t) {
            return Err(Error::invalid(format!("'{t}' in {what} of {of} is not a thorn name")));
        }
        if thorns[..i].iter().any(|u| u.eq_ignore_ascii_case(t)) {
            return Err(Error::invalid(format!("'{t}' is listed twice in {what} of {of}")));
        }
        if let Some(own) = &p.thorns
            && !own.contains(t)
        {
            return Err(Error::invalid(format!("{what} of {of} names '{t}', which is not one of this package's thorns")));
        }
        if p.exclude_thorns.contains(t) {
            return Err(Error::invalid(format!("{what} of {of} names '{t}', which exclude-thorns leaves out")));
        }
    }
    Ok(())
}

/// A folder inside the repo, given relative to its root: `Auxiliary/Cactus`, `src/fuka`.
fn inside_repo(path: &str) -> bool {
    let plain = std::path::Path::new(path).components().all(|c| matches!(c, std::path::Component::Normal(_)));
    !path.trim().is_empty() && plain
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
        // Every generation back to 1: tags are immutable, so an old one is read forever.
        generation::check(text, "cactarium.toml", 1, MANIFEST_FORMAT)?;
        let m: Manifest = toml::from_str(text).map_err(|source| Error::Toml { what: "cactarium.toml", source })?;
        m.validate()?;
        Ok(m)
    }

    pub fn to_toml(&self) -> Result<String> {
        self.validate()?;
        toml::to_string(self).map_err(|source| Error::Serialize { what: "cactarium.toml", source })
    }

    pub fn validate(&self) -> Result<()> {
        if !(1..=MANIFEST_FORMAT).contains(&self.format) {
            return Err(Error::invalid(format!("format must be {}", generation::readable(MANIFEST_FORMAT))));
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
                if p.flesh && p.library {
                    return Err(Error::invalid("[package] is a flesh or a library, not both"));
                }
                let thornless = p.thorns.is_some() || p.arrangement.is_some() || p.root.is_some() || !p.exclude_thorns.is_empty();
                if p.flesh && thornless {
                    return Err(Error::invalid(
                        "[package] flesh = true: a flesh has no thorns, so no thorns, exclude-thorns, arrangement or root",
                    ));
                }
                if p.library && thornless {
                    return Err(Error::invalid(
                        "[package] library = true: a library has no thorns, so no thorns, exclude-thorns, arrangement or root",
                    ));
                }
                if p.thorns.is_some() && !p.exclude_thorns.is_empty() {
                    return Err(Error::invalid(
                        "[package] gives both thorns (what is published) and exclude-thorns (what isn't); use one",
                    ));
                }
                for (i, t) in p.exclude_thorns.iter().enumerate() {
                    if !crate::slug::valid_thorn(t) || p.exclude_thorns[..i].contains(t) {
                        return Err(Error::invalid(format!("exclude-thorns: '{t}' is not a thorn name, or is listed twice")));
                    }
                }
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
                if let Some(r) = &p.root
                    && !inside_repo(r)
                {
                    return Err(Error::invalid(format!(
                        "[package] root '{r}' must be a folder inside the repo, like \"Auxiliary/Cactus\""
                    )));
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
            match (&d.target, d.any_of.len()) {
                (Some(_), 0) => {}
                (None, 0) => return Err(Error::invalid("a [[dependency]] needs a target or an any-of")),
                (Some(t), _) => {
                    return Err(Error::invalid(format!("the dependency on '{t}' has both a target and an any-of")));
                }
                (None, 1) => {
                    return Err(Error::invalid(format!(
                        "the dependency on {} has one alternative in any-of; give it as target",
                        d.label()
                    )));
                }
                (None, _) if d.needs.is_some() => {
                    return Err(Error::invalid(format!(
                        "the dependency on {}: with any-of, each alternative gives its own needs",
                        d.label()
                    )));
                }
                (None, _) => {}
            }
            if let Some(into) = &d.into {
                let label = d.label();
                if !d.any_of.is_empty() {
                    return Err(Error::invalid(format!(
                        "the dependency on {label}: into (where a library goes) is for a single target, not an any-of"
                    )));
                }
                if self.package.is_none() {
                    return Err(Error::invalid(format!(
                        "the dependency on '{label}' gives into, but only a [package] places a library; \
                         a stack's or sim's thornlist says where each checkout goes"
                    )));
                }
                if d.needs.is_some() {
                    return Err(Error::invalid(format!(
                        "the dependency on '{label}' gives into, so its target is a library, which has no thorns: drop needs"
                    )));
                }
                if !inside_repo(into) {
                    return Err(Error::invalid(format!(
                        "the dependency on '{label}': into '{into}' must be a folder inside this package's repo, like \"src/fuka\""
                    )));
                }
            }
            for (i, (target, needs)) in d.options().into_iter().enumerate() {
                versioned_target(target)?;
                if needs.is_some_and(|n| n.is_empty()) {
                    return Err(Error::invalid(format!(
                        "the dependency on '{target}' has needs = []: name the thorns of {target} this package uses, \
                         or, for a target with none (a flesh or a library), leave needs out"
                    )));
                }
                thorn_list("needs", target, needs.unwrap_or_default())?;
                if d.options()[..i].iter().any(|(t, _)| t.owner == target.owner && t.name == target.name) {
                    return Err(Error::invalid(format!("{} lists '{target}' twice", d.label())));
                }
            }
            thorn_names_of_this_package(&self.package, "needed-by", &d.label(), &d.needed_by)?;
        }
        Ok(())
    }
}
