//! Stacks: versioned thornlists.
//!
//! One format covers three shapes, and none is preferred over the others:
//! - **flat**: just a CRL thornlist (`thornlist = "carpetx.th"`, no `extends`). A standard
//!   CarpetX stack is typically this.
//! - **recipe**: `extends` other stacks (in priority order), then redirects repos with
//!   overrides and enables or disables thorns.
//! - **both**: bases plus extra CRL sections from `thornlist`.
//!
//! A definition names repos by branch (or tag), like any thornlist, and may pin some of them.
//! `[[stack.pin]]` pins one repo to a commit, a sha256 (downloads) or a tag; `pin-tag` pins every
//! git repo that has a tag of that name (an Einstein Toolkit release tags each of its repos
//! `ET_YYYY_MM_vN`), and a repo's own `[[stack.pin]]` wins over it. Pins are optional and may be
//! partial: a pinned repo stays put, the rest follow their branches.
//!
//! Whatever the shape, every stack version is a **release**: a [`FrozenStack`] holding one
//! verbatim CRL plus a pin for every repo. Releases are addressed exactly like package versions,
//! with track-namespaced semver (`owner/stack@mixed-precision:1.2.0`). Consumers only ever see the
//! frozen form. The exceptions are `owner/stack@head` and `@master` (see [`crate::slug::Tip`]): unpinned
//! forms for developers and power users.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::name::{Name, Owner};
use crate::sim::{RepoPin, ThornlistText, validate_repos};
use crate::slug::Slug;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct StackSection {
    pub name: Name,
    /// Cactarium users or orgs allowed to publish this stack from its repo: the proof of repo
    /// control, as for packages. Required in a repo's stack file; unused for uploaded stacks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<Owner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Where to go for help or to report problems: an `https://` URL (an issue tracker, a forum) or
    /// a `mailto:` address. Unset means the repo the artifact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    /// Bases, highest priority first: where two bases disagree on a repo's source, the earlier one
    /// wins. Each is an exact release, `owner/stack@1.2.0` or `owner/stack@track:1.2.0`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<Slug>,
    /// A CRL file, relative to this manifest. For a flat stack this is the whole list; with
    /// `extends` its sections are added on top of the bases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thornlist: Option<String>,
    /// Repo redirections applied to the bases.
    #[serde(default, rename = "override", skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<Override>,
    /// Thorns to enable (added as checkouts, or un-`#DISABLED`), as `Arrangement/Thorn`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enable: Vec<String>,
    /// Thorns to disable (`#DISABLED`), as `Arrangement/Thorn`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disable: Vec<String>,
    /// How release tags become versions when they aren't `vX.Y.Z`, e.g.
    /// `ET_{major}_{minor}_v{patch}` (see [`crate::version::TagPattern`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_pattern: Option<String>,
    /// Release tags below this version aren't releases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<semver::Version>,
    /// Pin every git repo that has a tag of this name to that tag's commit (e.g. `ET_2026_05_v0`).
    /// A repo without the tag falls through to its base release's pin or its branch. A repo's own
    /// `[[stack.pin]]` and a `!REPO_COMMIT` in the thornlist both win over this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_tag: Option<String>,
    /// Pins the author fixes in the definition itself, one repo each. Optional and possibly
    /// partial; a release pins every repo regardless.
    #[serde(default, rename = "pin", skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<StackPin>,
}

/// One repo pinned by a stack definition: to a commit, to a sha256 (a download), or to a tag
/// (resolved to its commit when a release is cut; a missing tag is an error).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct StackPin {
    /// The repo's name as the thornlist derives it (`!NAME`, else the URL's basename).
    pub name: String,
    /// Informational: the repo's URL, as a `.sim`'s pin records it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Informational: the branch the thornlist names.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// A tag name as a pin may give it: non-empty, and nothing git refuses in a ref name.
fn validate_tag(t: &str) -> Result<()> {
    let bad = t.is_empty()
        || t.starts_with(['-', '/', '.'])
        || t.ends_with(['/', '.'])
        || t.ends_with(".lock")
        || t.contains("..")
        || t.contains("@{")
        || t.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c));
    if bad {
        return Err(Error::invalid(format!("'{t}' is not a valid tag name")));
    }
    Ok(())
}

/// Points one repo of the bases somewhere else: a fork, another branch or tag, or a Cactarium
/// package version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Override {
    /// The repo name as the base thornlist derives it.
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// A Cactarium package (`owner/pkg@track:range`) instead of url + ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<Slug>,
}

fn arr_thorn(s: &str) -> bool {
    let Some((a, t)) = s.split_once('/') else { return false };
    let ident = |x: &str| {
        x.starts_with(|c: char| c.is_ascii_alphabetic())
            && x.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    ident(a) && ident(t)
}

impl StackSection {
    pub fn is_flat(&self) -> bool {
        self.extends.is_empty()
    }

    pub fn validate(&self) -> Result<()> {
        let n = &self.name;
        if let Some(c) = &self.contact {
            crate::manifest::validate_contact(c)?;
        }
        if self.extends.is_empty() && self.thornlist.is_none() {
            return Err(Error::invalid(format!("stack '{n}' needs a thornlist, extends, or both")));
        }
        if self.extends.is_empty() && !self.overrides.is_empty() {
            return Err(Error::invalid(format!("stack '{n}' has overrides but extends nothing")));
        }
        for b in &self.extends {
            if b.thorn.is_some() || b.exact().is_err() {
                return Err(Error::invalid(format!(
                    "base '{b}' must be an exact release, owner/stack@1.2.3"
                )));
            }
        }
        let mut repos = BTreeSet::new();
        for o in &self.overrides {
            if !repos.insert(o.repo.as_str()) {
                return Err(Error::invalid(format!("repo '{}' is overridden twice", o.repo)));
            }
            let git = o.url.is_some() || o.git_ref.is_some();
            if o.package.is_some() && git {
                return Err(Error::invalid(format!(
                    "override of '{}' gives both a package and url/ref",
                    o.repo
                )));
            }
            if o.package.is_none() && !git {
                return Err(Error::invalid(format!("override of '{}' changes nothing", o.repo)));
            }
            // A package reference: a third segment, if any, is a thorn, as before slugs widened.
            if let Some(p) = &o.package {
                p.as_thorn()?;
            }
        }
        for t in self.enable.iter().chain(&self.disable) {
            if !arr_thorn(t) {
                return Err(Error::invalid(format!("'{t}' is not Arrangement/Thorn")));
            }
        }
        if let Some(t) = self.enable.iter().find(|t| self.disable.contains(t)) {
            return Err(Error::invalid(format!("'{t}' is both enabled and disabled")));
        }
        if let Some(t) = &self.pin_tag {
            validate_tag(t)?;
        }
        if let Some(t) = &self.tag_pattern {
            t.parse::<crate::version::TagPattern>()?;
        }
        let mut pinned = BTreeSet::new();
        for p in &self.pins {
            if !pinned.insert(p.name.as_str()) {
                return Err(Error::invalid(format!("repo '{}' is pinned twice", p.name)));
            }
            let given = [p.commit.is_some(), p.sha256.is_some(), p.tag.is_some()]
                .iter()
                .filter(|g| **g)
                .count();
            if given != 1 {
                return Err(Error::invalid(format!(
                    "pin for '{}' needs exactly one of commit, sha256 or tag",
                    p.name
                )));
            }
            if let Some(c) = &p.commit
                && !((c.len() == 40 || c.len() == 64) && c.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(Error::invalid(format!(
                    "pin for '{}': '{c}' is not a full commit id",
                    p.name
                )));
            }
            if let Some(h) = &p.sha256
                && !(h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(Error::invalid(format!("pin for '{}': '{h}' is not a sha256", p.name)));
            }
            if let Some(t) = &p.tag {
                validate_tag(t)?;
            }
        }
        Ok(())
    }
}

/// A stack release: what `cactup install owner/stack@v` consumes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct FrozenStack {
    pub format: u32,
    /// `owner/stack@[track:]X.Y.Z`.
    pub stack: Slug,
    pub root: String,
    pub thornlist: ThornlistText,
    #[serde(rename = "repo")]
    pub repos: Vec<RepoPin>,
    /// The recipe it was resolved from, verbatim, for display and diffing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
}

impl FrozenStack {
    pub fn validate(&self) -> Result<()> {
        if self.stack.thorn.is_some() || self.stack.exact().is_err() {
            return Err(Error::invalid(format!(
                "stack release '{}' needs an exact version",
                self.stack
            )));
        }
        validate_repos(&self.repos)?;
        if let Some(r) = self
            .repos
            .iter()
            .find(|r| (r.commit.is_none() && r.sha256.is_none()) || r.dirty)
        {
            return Err(Error::invalid(format!("frozen stack repo '{}' is not pinned", r.name)));
        }
        Ok(())
    }
}
