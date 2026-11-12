//! References users type: `owner/name`, `owner/name/Thorn`, `owner/name/sim`, `owner/name@...`.
//!
//! The third segment is kept raw ([`Slug::thorn`], `[A-Za-z][A-Za-z0-9_.-]*`); the entry's kind
//! decides what it means. For a package it is a thorn ([`Slug::as_thorn`] checks thorn-name rules);
//! for a multi-sim entry (`[sim.files]`) it is one of its sims ([`Slug::as_sim`] checks sim-name
//! rules), which such an entry always needs. A reader from before multi-sim entries took any third
//! segment for a thorn, and only accepted thorn names there.
//!
//! What follows `@` depends on the entry kind, so a [`Slug`] keeps it raw and offers typed
//! readers: a range ([`Slug::req`]), an exact version ([`Slug::exact`]) or a branch tip
//! ([`Slug::tip`]). A package can also be named at a commit:
//!
//! - `owner/pkg#<sha>` names only a commit, full or abbreviated (at least 7 hex digits);
//! - `owner/pkg@<x>` prefers every other reading: only when `<x>` names no version, range or
//!   track does a registry try it as a commit ([`Slug::commit_fallback`]).
//!
//! An abbreviated id is resolved by the registry against the repo's history, and refused when it
//! matches more than one commit.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::name::{Name, Owner, SimName};
use crate::version::{TrackedReq, TrackedVersion};
use crate::{Error, Result};

/// The two unpinned forms of a stack, for developers and power users:
/// - `owner/stack@head`: the definition as written, each repo at the tip of the branch it names
///   (commits the definition pins on purpose stay pinned);
/// - `owner/stack@master`: every repo at the tip of its default branch, ignoring the branches and
///   pins the definition names, as `cactup install master` does for the ET manifest.
///
/// Neither is a release; neither can be a stack's base. A power-user
/// form (like installing the ET's `master`): never a release and never a stack base. A `.sim` may
/// name it as provenance, since its own `[[repo]]` pins are what make it reproducible.
pub const HEAD: &str = "head";
/// See [`HEAD`].
pub const MASTER: &str = "master";

/// Which unpinned form a slug names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tip {
    /// `@head`: the branches the definition names.
    Head,
    /// `@master`: every repo's default branch.
    Master,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Slug {
    pub owner: Owner,
    pub name: Name,
    /// The third segment, raw: a thorn inside a package (`owner/pkg/Thorn`, keeping Cactus's case;
    /// read it with [`Slug::as_thorn`]) or one sim of a multi-sim entry (`owner/name/qc0-z4c`; read
    /// it with [`Slug::as_sim`]). The entry's kind decides which.
    pub thorn: Option<String>,
    /// Everything after `@`, unparsed.
    pub at: Option<String>,
    /// Everything after `#`: a commit id, full or abbreviated, lowercase hex. Never with `at`.
    pub sha: Option<String>,
}

/// The shortest abbreviated commit id accepted.
pub const MIN_COMMIT_PREFIX: usize = 7;

/// A Cactus thorn name: `[A-Za-z][A-Za-z0-9_]*`.
pub fn valid_thorn(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// What a slug's third segment may hold: a thorn or a sim name, `[A-Za-z][A-Za-z0-9_.-]*`.
fn valid_segment(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

impl Slug {
    /// The third segment read as a thorn (`owner/pkg/Thorn`): `None` without one, an error when it
    /// is not a thorn name. For packages, and anywhere else a third segment means a thorn.
    pub fn as_thorn(&self) -> Result<Option<&str>> {
        match self.thorn.as_deref() {
            Some(t) if !valid_thorn(t) => Err(Error::invalid(format!("'{t}' in '{self}' is not a thorn name"))),
            t => Ok(t),
        }
    }

    /// The third segment read as one sim of a multi-sim entry (`owner/name/qc0-z4c`): `None`
    /// without one, an error when it is not a sim name (lowercase, starting with a letter).
    pub fn as_sim(&self) -> Result<Option<SimName>> {
        self.thorn
            .as_deref()
            .map(|s| s.parse::<SimName>().map_err(|e| Error::invalid(format!("'{s}' in '{self}': {e}"))))
            .transpose()
    }

    /// `@head` or `@master`: one of a stack's unpinned forms.
    pub fn tip(&self) -> Option<Tip> {
        match self.at.as_deref() {
            Some(HEAD) => Some(Tip::Head),
            Some(MASTER) => Some(Tip::Master),
            _ => None,
        }
    }

    /// `#<sha>`: the commit this slug names, full or abbreviated. Nothing else reads `#`.
    pub fn sha(&self) -> Option<&str> {
        self.sha.as_deref()
    }

    /// `@<x>` that could also be a commit (lowercase hex, at least [`MIN_COMMIT_PREFIX`] digits).
    /// Every other reading wins: try this only when `<x>` names no version, range or track.
    pub fn commit_fallback(&self) -> Option<&str> {
        self.at.as_deref().filter(|at| is_commit_prefix(at))
    }

    /// The `@` part as a version range (`^1.2`, `mixed-precision:^1`). No `@` means any version on
    /// the default track. `@head` and `@master` are not ranges (check [`Slug::tip`] first), and a
    /// `#<sha>` slug names a commit, not a version.
    pub fn req(&self) -> Result<TrackedReq> {
        if self.sha.is_some() {
            return Err(Error::invalid(format!("'{self}' names a commit, not a version")));
        }
        match &self.at {
            None => Ok(TrackedReq::any()),
            Some(at) if at == HEAD || at == MASTER => Err(Error::invalid(format!("'{self}' names unpinned branch tips, not a version"))),
            Some(at) => at.parse(),
        }
    }

    /// The `@` part as one exact version (`1.4.0`, `mixed-precision:1.4.0`).
    pub fn exact(&self) -> Result<TrackedVersion> {
        let at = self.at.as_deref().ok_or_else(|| Error::invalid(format!("'{self}' names no version")))?;
        at.parse()
    }

    /// The `@` part as an upload number (sims: `owner/name@3`).
    pub fn number(&self) -> Result<Option<u32>> {
        match &self.at {
            None => Ok(None),
            Some(at) => at
                .parse::<u32>()
                .ok()
                .filter(|n| *n >= 1)
                .map(Some)
                .ok_or_else(|| Error::invalid(format!("'{at}' in '{self}' is not a version number"))),
        }
    }

    /// `owner/name`, without thorn or version.
    pub fn entry(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// A full commit id: 40 (SHA-1) or 64 (SHA-256) lowercase hex digits.
pub fn is_commit_id(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && is_lower_hex(s)
}

/// A commit id, full or abbreviated: [`MIN_COMMIT_PREFIX`] to 64 lowercase hex digits.
pub fn is_commit_prefix(s: &str) -> bool {
    (MIN_COMMIT_PREFIX..=64).contains(&s.len()) && is_lower_hex(s)
}

fn is_lower_hex(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl FromStr for Slug {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        let bad = |why: &str| Error::invalid(format!("'{s}' is not a Cactarium reference: {why}"));
        if s.contains('@') && s.contains('#') {
            return Err(bad("use '@' for a version or '#' for a commit, not both"));
        }
        let (path, at, sha) = if let Some((p, h)) = s.split_once('#') {
            let h = h.to_ascii_lowercase();
            if !is_commit_prefix(&h) {
                return Err(bad(&format!(
                    "'#' takes a commit id of {MIN_COMMIT_PREFIX} to 64 hex digits, not '{h}'"
                )));
            }
            (p, None, Some(h))
        } else {
            match s.split_once('@') {
                Some((_, "")) => return Err(bad("nothing follows '@'")),
                Some((p, a)) => (p, Some(a.to_owned()), None),
                None => (s, None, None),
            }
        };
        let mut parts = path.split('/');
        let (Some(owner), Some(name)) = (parts.next(), parts.next()) else {
            return Err(bad("expected owner/name"));
        };
        let thorn = parts.next().map(str::to_owned);
        if parts.next().is_some() {
            return Err(bad("expected owner/name, owner/name/Thorn or owner/name/sim"));
        }
        if let Some(t) = &thorn
            && !valid_segment(t)
        {
            return Err(bad(&format!("'{t}' is not a thorn or sim name")));
        }
        Ok(Slug { owner: owner.parse()?, name: name.parse()?, thorn, at, sha })
    }
}

impl TryFrom<String> for Slug {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<Slug> for String {
    fn from(s: Slug) -> String {
        s.to_string()
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)?;
        if let Some(t) = &self.thorn {
            write!(f, "/{t}")?;
        }
        if let Some(a) = &self.at {
            write!(f, "@{a}")?;
        }
        if let Some(h) = &self.sha {
            write!(f, "#{h}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_shapes() {
        let s: Slug = "max-morris/carpetx/TestReal4@mixed-precision:^1.4".parse().unwrap();
        assert_eq!(s.entry(), "max-morris/carpetx");
        assert_eq!(s.thorn.as_deref(), Some("TestReal4"));
        assert!(s.req().unwrap().matches(&"mixed-precision:1.5.0".parse().unwrap()));
        assert_eq!(s.to_string(), "max-morris/carpetx/TestReal4@mixed-precision:^1.4");

        let sim: Slug = "someone/qc0-bbh@3".parse().unwrap();
        assert_eq!(sim.number().unwrap(), Some(3));

        let exact: Slug = "max-morris/carpetx@mixed-precision:1.4.0".parse().unwrap();
        assert_eq!(exact.exact().unwrap().to_string(), "mixed-precision:1.4.0");

        let tip: Slug = "max-morris/carpetx@master".parse().unwrap();
        assert_eq!(tip.tip(), Some(Tip::Master));
        assert!(tip.req().is_err() && tip.exact().is_err());
        let head: Slug = "max-morris/carpetx@head".parse().unwrap();
        assert_eq!(head.tip(), Some(Tip::Head));
        assert!(head.req().is_err());

        let bare: Slug = "einsteintoolkit/et".parse().unwrap();
        assert!(bare.req().unwrap().matches(&"2026.5.0".parse().unwrap()));
    }

    #[test]
    fn rejects_malformed() {
        for s in ["carpetx", "a/b/c/d", "a/b@", "A/b", "a/b/1Thorn", "a/b/-x", "a/b/x y", "a/b/x/", "a/b@0"] {
            let r = s.parse::<Slug>().and_then(|x| x.number().map(|_| x));
            assert!(r.is_err(), "{s} should be rejected");
        }
    }

    /// The third segment is a thorn or a sim, by the entry's kind; each reading checks its rules.
    #[test]
    fn third_segment_is_a_thorn_or_a_sim() {
        let sim: Slug = "et/cottonmouth-sims/qc0-z4c@^1".parse().unwrap();
        assert_eq!(sim.entry(), "et/cottonmouth-sims");
        assert_eq!(sim.as_sim().unwrap().unwrap().as_str(), "qc0-z4c");
        assert!(sim.as_thorn().is_err(), "a sim name with '-' is not a thorn name");
        assert_eq!(sim.to_string(), "et/cottonmouth-sims/qc0-z4c@^1");
        assert!(sim.req().unwrap().matches(&"1.2.0".parse().unwrap()));

        let thorn: Slug = "et/carpetx/CarpetX".parse().unwrap();
        assert_eq!(thorn.as_thorn().unwrap(), Some("CarpetX"));
        assert!(thorn.as_sim().is_err(), "a sim name is lowercase");

        let both: Slug = "et/x/qc0".parse().unwrap();
        assert_eq!(both.as_thorn().unwrap(), Some("qc0"));
        assert_eq!(both.as_sim().unwrap().unwrap().as_str(), "qc0");

        let dotted: Slug = "et/x/bhns.gw230529@1.0.0".parse().unwrap();
        assert!(dotted.as_thorn().is_err());
        assert_eq!(dotted.as_sim().unwrap().unwrap().as_str(), "bhns.gw230529");

        let none: Slug = "et/x".parse().unwrap();
        assert_eq!(none.as_thorn().unwrap(), None);
        assert!(none.as_sim().unwrap().is_none());
    }
}
