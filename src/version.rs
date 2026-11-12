//! Track-namespaced semantic versions.
//!
//! A repo can publish several independent version lines ("tracks"). In git the track is a tag
//! prefix: `v1.2.3` is on the default track, `mixed-precision/v1.2.3` is version 1.2.3 on track
//! `mixed-precision`. Git refs cannot contain ':', so Cactarium and cactup write the same version
//! as `1.2.3` and `mixed-precision:1.2.3`, and ranges as `^1.2` and `mixed-precision:^1.2`.
//!
//! Semver ordering and ranges apply within one track only. Versions on different tracks are never
//! compared and never match each other's ranges.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Track names: a lowercase letter followed by up to 63 of `[a-z0-9._-]`. They start with a letter
/// so `pkg@mixed-precision` (a track) can never be confused with `pkg@1` (a range). `default` names
/// the unprefixed track in `cactarium.toml`, `x` is a semver wildcard, and `head` and `master`
/// name a stack's unpinned forms (see [`crate::slug::Tip`]), so none of them can be used.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TrackName(String);

/// What `default-track = "default"` means: the unprefixed track.
pub const DEFAULT_TRACK_KEYWORD: &str = "default";

impl TrackName {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(s: &str) -> Result<()> {
        let bad = |why: &str| Err(Error::invalid(format!("'{s}' is not a valid track name: {why}")));
        let Some(first) = s.chars().next() else { return bad("it is empty") };
        if s.len() > 64 {
            return bad("it is longer than 64 characters");
        }
        if !first.is_ascii_lowercase() {
            return bad("it must start with a lowercase letter");
        }
        if let Some(c) = s.chars().find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(*c))) {
            return bad(&format!("'{c}' is not allowed"));
        }
        if s == DEFAULT_TRACK_KEYWORD || s == "x" || s == crate::slug::HEAD || s == crate::slug::MASTER {
            return bad("it is reserved");
        }
        Ok(())
    }
}

impl FromStr for TrackName {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        TrackName::validate(s)?;
        Ok(TrackName(s.to_owned()))
    }
}

impl TryFrom<String> for TrackName {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        TrackName::validate(&s)?;
        Ok(TrackName(s))
    }
}

impl From<TrackName> for String {
    fn from(t: TrackName) -> String {
        t.0
    }
}

impl fmt::Display for TrackName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A version on a track: `1.2.3` (default track) or `mixed-precision:1.2.3`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TrackedVersion {
    pub track: Option<TrackName>,
    pub version: semver::Version,
}

impl TrackedVersion {
    /// Reads a git tag. Returns `None` for tags that are not versions: anything other than
    /// `[<track>/][v]X.Y.Z[-pre][+build]` with a single-segment, valid track name. Such tags are
    /// simply ignored by the registry, not errors.
    pub fn from_tag(tag: &str) -> Option<TrackedVersion> {
        let tag = tag.strip_prefix("refs/tags/").unwrap_or(tag);
        let (track, rest) = match tag.split_once('/') {
            Some((prefix, rest)) => (Some(prefix.parse::<TrackName>().ok()?), rest),
            None => (None, tag),
        };
        let rest = rest.strip_prefix('v').unwrap_or(rest);
        let version = semver::Version::parse(rest).ok()?;
        Some(TrackedVersion { track, version })
    }

    /// The conventional git tag for this version: `v1.2.3` or `mixed-precision/v1.2.3`.
    pub fn conventional_tag(&self) -> String {
        match &self.track {
            Some(t) => format!("{t}/v{}", self.version),
            None => format!("v{}", self.version),
        }
    }

    /// Orders two versions on the same track. Versions on different tracks are not comparable.
    pub fn cmp_within_track(&self, other: &TrackedVersion) -> Option<Ordering> {
        (self.track == other.track).then(|| self.version.cmp(&other.version))
    }
}

/// How release tags that aren't `vX.Y.Z` become versions: literal text around `{major}` and,
/// optionally, `{minor}` and `{patch}`, each at most once; one left out reads as 0.
/// `ET_{major}_{minor}_v{patch}` reads the Einstein Toolkit's `ET_2026_05_v0` as 2026.5.0, and
/// `v{major}.{minor}` reads AsterX's `v1.0` as 1.0.0 (on the default track). A manifest gives it
/// as `tag-pattern`; with one, only tags it matches are versions.
#[derive(Clone, Debug)]
pub struct TagPattern {
    text: String,
    re: regex::Regex,
}

impl TagPattern {
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The version `tag` names, if it matches.
    pub fn version(&self, tag: &str) -> Option<TrackedVersion> {
        let tag = tag.strip_prefix("refs/tags/").unwrap_or(tag);
        let c = self.re.captures(tag)?;
        // A part the pattern leaves out reads as 0.
        let n = |k: &str| c.name(k).map_or(Some(0), |m| m.as_str().parse::<u64>().ok());
        Some(TrackedVersion { track: None, version: semver::Version::new(n("major")?, n("minor")?, n("patch")?) })
    }
}

impl FromStr for TagPattern {
    type Err = Error;

    fn from_str(pattern: &str) -> Result<Self> {
        let bad = |why: &str| Error::invalid(format!("tag-pattern '{pattern}': {why}"));
        if pattern.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad("no spaces"));
        }
        let mut re = String::from("^");
        let mut rest = pattern;
        let mut seen: Vec<&str> = Vec::new();
        while let Some(open) = rest.find('{') {
            re.push_str(&regex::escape(&rest[..open]));
            let close = rest[open..].find('}').map(|c| open + c).ok_or_else(|| bad("an unclosed {"))?;
            let name = &rest[open + 1..close];
            if !matches!(name, "major" | "minor" | "patch") || seen.contains(&name) {
                return Err(bad("only {major}, {minor} and {patch}, each once"));
            }
            seen.push(name);
            re.push_str(&format!("(?P<{name}>[0-9]{{1,9}})"));
            rest = &rest[close + 1..];
        }
        if rest.contains('}') {
            return Err(bad("a } without its {"));
        }
        re.push_str(&regex::escape(rest));
        re.push('$');
        if !seen.contains(&"major") {
            return Err(bad("needs {major} ({minor} and {patch} may be left out, reading as 0)"));
        }
        if seen.contains(&"patch") && !seen.contains(&"minor") {
            return Err(bad("{patch} needs {minor}"));
        }
        let re = regex::Regex::new(&re).map_err(|e| bad(&e.to_string()))?;
        Ok(TagPattern { text: pattern.to_owned(), re })
    }
}

/// Splits `track:rest`. A leading lowercase letter means a track; anything else is plain semver.
fn split_track(s: &str) -> Result<(Option<TrackName>, &str)> {
    match s.split_once(':') {
        Some((track, rest)) => Ok((Some(track.parse()?), rest)),
        None => Ok((None, s)),
    }
}

impl FromStr for TrackedVersion {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        let (track, rest) = split_track(s)?;
        let version = semver::Version::parse(rest)
            .map_err(|e| Error::invalid(format!("'{s}' is not a version: {e}")))?;
        Ok(TrackedVersion { track, version })
    }
}

impl TryFrom<String> for TrackedVersion {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<TrackedVersion> for String {
    fn from(v: TrackedVersion) -> String {
        v.to_string()
    }
}

impl fmt::Display for TrackedVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.track {
            Some(t) => write!(f, "{t}:{}", self.version),
            None => write!(f, "{}", self.version),
        }
    }
}

/// A version range, read as npm reads ranges (see [`npm_alternative`]): `^1.2`, `>=2, <3`,
/// `>=1.0.0 <3.2.4`, `1.2.3 - 2.3.4`, `*`, or `||` alternatives such as `^1.2 || >=2.5.0 <2.6`.
///
/// An alternative may open with a track, `mixed-precision:^1.2`, or be a bare track name,
/// `mixed-precision` (any version on that track); `:^1.2` names the default track. If one
/// alternative names a track, every alternative must (`mp:^1 || :^2`, not `mp:^1 || ^2`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TrackedReq {
    /// The `||` alternatives: a version matches if it matches any of them. Never empty.
    pub alts: Vec<TrackedAlt>,
}

/// One `||` alternative of a [`TrackedReq`]: a range on one track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedAlt {
    /// `None`: the default track.
    pub track: Option<TrackName>,
    pub req: semver::VersionReq,
}

impl TrackedReq {
    /// Any version on the default track.
    pub fn any() -> TrackedReq {
        TrackedReq { alts: vec![TrackedAlt { track: None, req: semver::VersionReq::STAR }] }
    }

    pub fn matches(&self, v: &TrackedVersion) -> bool {
        self.alts.iter().any(|a| a.track == v.track && a.req.matches(&v.version))
    }

    /// One exact version, `=1.2.3` (or a bare `1.2.3`), rather than a range.
    pub fn is_exact(&self) -> bool {
        matches!(self.alts.as_slice(), [a] if matches!(a.req.comparators.as_slice(),
            [c] if c.op == semver::Op::Exact && c.minor.is_some() && c.patch.is_some()))
    }

    /// The highest version in `candidates` that this range matches.
    pub fn best_match<'a>(&self, candidates: impl IntoIterator<Item = &'a TrackedVersion>) -> Option<&'a TrackedVersion> {
        candidates.into_iter().filter(|v| self.matches(v)).max_by(|a, b| a.version.cmp(&b.version))
    }
}

impl FromStr for TrackedReq {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        let bad = |why: String| Error::invalid(format!("'{s}' is not a version range: {why}"));
        let mut alts = Vec::new();
        let mut named = Vec::new();
        for alt in s.split("||") {
            let alt = alt.trim();
            let (track, rest, names_track) = match alt.split_once(':') {
                Some(("", rest)) => (None, rest, true),
                Some((t, rest)) => (Some(t.parse()?), rest, true),
                // A bare track name: any version on it.
                None if alt.starts_with(|c: char| c.is_ascii_lowercase()) && alt != "x" => {
                    alts.push(TrackedAlt { track: Some(alt.parse()?), req: semver::VersionReq::STAR });
                    named.push(true);
                    continue;
                }
                None => (None, alt, false),
            };
            let req = semver::VersionReq::parse(&npm_alternative(rest).map_err(bad)?)
                .map_err(|e| bad(e.to_string()))?;
            alts.push(TrackedAlt { track, req });
            named.push(names_track);
        }
        if named.contains(&true) && named.contains(&false) {
            return Err(bad(
                "one alternative names a track, so every alternative must (':' for the default track)".to_owned(),
            ));
        }
        Ok(TrackedReq { alts })
    }
}

/// Reads one `||` alternative of a range (without its track) as npm does, into a Cargo-style
/// requirement:
/// - Comparators are separated by commas, spaces or both (`>=1.2, <2`,
///   `>=1.2 <2`), and an operator may stand apart from its version (`< 3.2.4`).
/// - `A - B` is a hyphen range: at least `A`, at most `B`. A partial `B` covers what it names
///   (`1.2 - 2.3` is `>=1.2.0, <2.4.0`).
/// - Bare versions read as npm reads them, not as Cargo does: `1.2.3` is exactly 1.2.3, `1.2` is
///   `~1.2` (any 1.2.x) and `1` is `^1` (any 1.x.x). A part with an operator (`^`, `~`, `=`, `>`,
///   `<`) or a wildcard (`*`, `x`) keeps its meaning.
fn npm_alternative(alt: &str) -> std::result::Result<String, String> {
    // Tokens, with a lone operator joined to the version after it.
    let mut tokens: Vec<String> = Vec::new();
    let mut pending_op: Option<&str> = None;
    for t in alt.split([',', ' ', '\t']).filter(|t| !t.is_empty()) {
        if matches!(t, "^" | "~" | "=" | ">" | "<" | ">=" | "<=") {
            if pending_op.replace(t).is_some() {
                return Err(format!("two operators in a row in '{}'", alt.trim()));
            }
            continue;
        }
        tokens.push(match pending_op.take() {
            Some(op) => format!("{op}{t}"),
            None => t.to_owned(),
        });
    }
    if pending_op.is_some() {
        return Err(format!("an operator with no version after it in '{}'", alt.trim()));
    }
    if tokens.is_empty() {
        return Err("an empty alternative".to_owned());
    }
    let mut parts = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens.get(i + 1).is_some_and(|t| t == "-") {
            let (Some(lo), Some(hi)) = (tokens.get(i), tokens.get(i + 2)) else {
                return Err(format!("'{} -' needs an upper bound", tokens[i]));
            };
            parts.push(format!(">={}", hyphen_low(lo)?));
            parts.push(hyphen_high(hi)?);
            i += 3;
        } else if tokens[i] == "-" {
            return Err("'-' needs a version on each side".to_owned());
        } else {
            parts.push(bare_npm(&tokens[i]));
            i += 1;
        }
    }
    Ok(parts.join(", "))
}

fn bare_npm(p: &str) -> String {
    let bare = p.starts_with(|c: char| c.is_ascii_digit())
        && !p.split(['.', '-', '+']).take(3).any(|c| matches!(c, "*" | "x" | "X"));
    if !bare {
        return p.to_owned();
    }
    let core = p.split(['-', '+']).next().unwrap_or(p);
    match core.split('.').count() {
        1 => format!("^{p}"),
        2 => format!("~{p}"),
        _ => format!("={p}"),
    }
}

/// The numeric parts a hyphen-range bound names (up to the first wildcard), and whether it is a
/// full `X.Y.Z` (possibly with a pre-release), which is then used as written.
fn hyphen_parts(b: &str) -> std::result::Result<(Vec<u64>, bool), String> {
    let core = b.split(['-', '+']).next().unwrap_or(b);
    let mut nums = Vec::new();
    for c in core.split('.') {
        if matches!(c, "*" | "x" | "X") {
            break;
        }
        nums.push(c.parse::<u64>().map_err(|_| format!("'{b}' in a hyphen range is not a version"))?);
    }
    if nums.len() > 3 || (nums.len() < 3 && core != b) {
        return Err(format!("'{b}' in a hyphen range is not a version"));
    }
    Ok((nums.clone(), nums.len() == 3))
}

fn hyphen_low(b: &str) -> std::result::Result<String, String> {
    let (n, full) = hyphen_parts(b)?;
    Ok(if full { b.to_owned() } else { format!("{}.{}.{}", n.first().unwrap_or(&0), n.get(1).unwrap_or(&0), 0) })
}

fn hyphen_high(b: &str) -> std::result::Result<String, String> {
    let (n, full) = hyphen_parts(b)?;
    Ok(match n.as_slice() {
        _ if full => format!("<={b}"),
        [] => "*".to_owned(),
        [major] => format!("<{}.0.0", major + 1),
        [major, minor] => format!("<{major}.{}.0", minor + 1),
        _ => unreachable!(),
    })
}

impl TryFrom<String> for TrackedReq {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<TrackedReq> for String {
    fn from(r: TrackedReq) -> String {
        r.to_string()
    }
}

impl fmt::Display for TrackedReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Normalized: with a track anywhere, every alternative names one (`:` the default track).
        let tracked = self.alts.iter().any(|a| a.track.is_some());
        for (i, a) in self.alts.iter().enumerate() {
            if i > 0 {
                write!(f, " || ")?;
            }
            match &a.track {
                Some(t) => write!(f, "{t}:")?,
                None if tracked => write!(f, ":")?,
                None => {}
            }
            write!(f, "{}", a.req)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Bare versions read as npm reads them; operators and wildcards keep their meaning.
    #[test]
    fn bare_versions_are_npm_style() {
        let m = |req: &str, v: &str| req.parse::<TrackedReq>().unwrap().matches(&TrackedVersion { track: None, version: semver::Version::parse(v).unwrap() });
        assert!(m("1.2.3", "1.2.3") && !m("1.2.3", "1.2.4") && !m("1.2.3", "1.3.0"));
        assert!(m("1.2", "1.2.9") && !m("1.2", "1.3.0") && !m("1.2", "1.1.9"));
        assert!(m("1", "1.9.0") && !m("1", "2.0.0"));
        assert!(m("^1.2.3", "1.9.0") && !m("^1.2.3", "2.0.0") && !m("^1.2.3", "1.2.2"));
        assert!(m("~1.2.3", "1.2.9") && !m("~1.2.3", "1.3.0"));
        assert!(m("=1.2.3", "1.2.3") && !m("=1.2.3", "1.2.4"));
        assert!(m(">=1.2, <2", "1.5.0") && !m(">=1.2, <2", "2.0.0"));
        assert!(m("1.*", "1.7.0") && m("*", "9.0.0") && m("1.2.x", "1.2.5"));
        assert!(m("2026.5", "2026.5.3") && !m("2026.5", "2026.6.0") && m("2026", "2026.11.0"));
        assert!(m("1.2.3-rc.1", "1.2.3-rc.1") && !m("1.2.3-rc.1", "1.2.3"));
        let t: TrackedReq = "mixed-precision:1.2".parse().unwrap();
        assert_eq!(t.alts[0].track.as_ref().map(ToString::to_string).as_deref(), Some("mixed-precision"));
        assert!(t.matches(&v("mixed-precision:1.2.4")) && !t.matches(&v("mixed-precision:1.3.0")));
    }

    /// Full npm range syntax, for forks that only work together in very specific ways.
    #[test]
    fn npm_ranges() {
        let m = |req: &str, ver: &str| req.parse::<TrackedReq>().unwrap().matches(&v(ver));
        for r in [">=1.0.0 <3.2.4", ">=1.0.0 < 3.2.4", ">= 1.0.0, < 3.2.4", ">=1.0.0,<3.2.4"] {
            assert!(m(r, "1.0.0") && m(r, "3.2.3") && !m(r, "3.2.4") && !m(r, "0.9.9"), "{r}");
        }
        // Hyphen ranges: a partial upper bound covers what it names.
        assert!(m("1.2.3 - 2.3.4", "2.3.4") && !m("1.2.3 - 2.3.4", "2.3.5") && !m("1.2.3 - 2.3.4", "1.2.2"));
        assert!(m("1.2 - 2.3", "1.2.0") && m("1.2 - 2.3", "2.3.9") && !m("1.2 - 2.3", "2.4.0"));
        assert!(m("1 - 2", "2.9.9") && !m("1 - 2", "3.0.0"));
        assert!(m("1.x - 2.x", "2.5.0") && !m("1.x - 2.x", "3.0.0"));
        // Alternatives.
        let r = "^1.2 || >=2.5.0 <2.6 || 3.0.1";
        assert!(m(r, "1.9.0") && m(r, "2.5.7") && m(r, "3.0.1"));
        assert!(!m(r, "2.0.0") && !m(r, "2.6.0") && !m(r, "3.0.2"));
        // Each alternative has its own track; naming one anywhere means naming one everywhere.
        let t: TrackedReq = "mixed-precision:>=1.0.0 <1.4.2 || :2.1 || other".parse().unwrap();
        assert!(t.matches(&v("mixed-precision:1.4.1")) && t.matches(&v("2.1.7")) && t.matches(&v("other:9.0.0")));
        assert!(!t.matches(&v("1.4.1")) && !t.matches(&v("mixed-precision:2.1.7")) && !t.matches(&v("mixed-precision:1.4.2")));
        assert_eq!(t.to_string(), "mixed-precision:>=1.0.0, <1.4.2 || :~2.1 || other:*");
        let d: TrackedReq = ":1.0.0".parse().unwrap();
        assert!(d.matches(&v("1.0.0")) && d.to_string() == "=1.0.0");
        // It prints back, normalized, in a form it reads.
        for r in [">=1.0.0 <3.2.4", "1.2 - 2.3", "^1.2 || >=2.5.0 <2.6", "mixed-precision:1 || :3", "mp || other:^2"] {
            let a: TrackedReq = r.parse().unwrap();
            assert_eq!(a.to_string().parse::<TrackedReq>().unwrap(), a, "{r}");
        }
        assert_eq!(">=1.0.0 <3.2.4".parse::<TrackedReq>().unwrap().to_string(), ">=1.0.0, <3.2.4");
        for bad in [">=1.0.0 <", "1.2.3 -", "- 1.2.3", "^1 ||", "|| ^1", ">= < 1.0.0", "1.2.3 - 2.x.5-rc",
                    "mp:^1 || ^2", "^1 || mp", ">=1 mp:<2", "mp:^1 mp:<2"] {
            assert!(bad.parse::<TrackedReq>().is_err(), "{bad} should be rejected");
        }
        assert!("1.2.3".parse::<TrackedReq>().unwrap().is_exact() && "=1.2.3".parse::<TrackedReq>().unwrap().is_exact());
        assert!(!"1.2".parse::<TrackedReq>().unwrap().is_exact() && !"1.2.3 || 1.2.4".parse::<TrackedReq>().unwrap().is_exact());
    }

    use super::*;

    fn v(s: &str) -> TrackedVersion {
        s.parse().unwrap()
    }

    #[test]
    fn tags_map_to_tracks() {
        assert_eq!(TrackedVersion::from_tag("v1.2.3"), Some(v("1.2.3")));
        assert_eq!(TrackedVersion::from_tag("1.2.3"), Some(v("1.2.3")));
        assert_eq!(TrackedVersion::from_tag("refs/tags/v1.2.3"), Some(v("1.2.3")));
        assert_eq!(TrackedVersion::from_tag("mixed-precision/v1.2.3"), Some(v("mixed-precision:1.2.3")));
        assert_eq!(TrackedVersion::from_tag("mixed-precision/v2.0.0-rc.1"), Some(v("mixed-precision:2.0.0-rc.1")));
        assert_eq!(v("mixed-precision:1.2.3").conventional_tag(), "mixed-precision/v1.2.3");
    }

    #[test]
    fn non_version_tags_are_ignored() {
        for tag in ["ET_2026_05", "mixed-precision-1", "v1.2", "a/b/v1.2.3", "Upper/v1.2.3", "default/v1.0.0"] {
            assert_eq!(TrackedVersion::from_tag(tag), None, "{tag}");
        }
    }

    #[test]
    fn ranges_stay_within_their_track() {
        let r: TrackedReq = "mixed-precision:^1.2".parse().unwrap();
        assert!(r.matches(&v("mixed-precision:1.4.0")));
        assert!(!r.matches(&v("1.4.0")));
        assert!(!r.matches(&v("mixed-precision:2.0.0")));
        let d: TrackedReq = "^1.2".parse().unwrap();
        assert!(d.matches(&v("1.9.0")));
        assert!(!d.matches(&v("mixed-precision:1.9.0")));
    }

    #[test]
    fn bare_track_means_any_version_on_it() {
        let r: TrackedReq = "mixed-precision".parse().unwrap();
        assert!(r.matches(&v("mixed-precision:0.1.0")));
        assert!(!r.matches(&v("0.1.0")));
        let any: TrackedReq = "*".parse().unwrap();
        assert!(any.matches(&v("3.0.0")));
        assert_eq!(r.to_string(), "mixed-precision:*");
    }

    #[test]
    fn best_match_picks_highest_in_track() {
        let all = [v("1.0.0"), v("1.5.0"), v("mixed-precision:1.9.0"), v("2.0.0")];
        let r: TrackedReq = "^1".parse().unwrap();
        assert_eq!(r.best_match(&all), Some(&v("1.5.0")));
        assert_eq!(v("1.0.0").cmp_within_track(&v("mixed-precision:1.0.0")), None);
    }

    #[test]
    fn reserved_track_names() {
        assert!("default".parse::<TrackName>().is_err());
        assert!("x".parse::<TrackName>().is_err());
        assert!("master".parse::<TrackName>().is_err());
        assert!("head".parse::<TrackName>().is_err());
        assert!("1abc".parse::<TrackName>().is_err());
    }
}
