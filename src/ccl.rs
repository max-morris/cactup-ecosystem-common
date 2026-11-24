//! A conservative scanner for the dependency facts in a thorn's `interface.ccl` and
//! `configuration.ccl`.
//!
//! This is not a full CCL parser (that lives in the flesh's Perl). It extracts what the registry
//! indexes and what a thornlist trim needs:
//! - `interface.ccl`: `IMPLEMENTS`, `INHERITS`, `FRIEND(S)`, `REQUIRES/USES/PROVIDES FUNCTION`,
//!   `USES INCLUDE`, `INCLUDE[S]`.
//! - `configuration.ccl`: `REQUIRES` capabilities, `REQUIRES THORNS`, `OPTIONAL`, `PROVIDES`.
//! - the thorn's sources, for a tree ([`scan_tree`]) or a directory ([`ThornFacts::scan_sources`]):
//!   which of its `USES INCLUDE` headers they `#include` outside any conditional, the ones it
//!   cannot compile without ([`Need::Include`]).
//!
//! Keywords are case-insensitive, `#` starts a comment, and the bodies of `{ ... }` blocks (a
//! `PROVIDES` block's `SCRIPT`/`LANG`, an `OPTIONAL` block's `DEFINE`) are skipped. Names are kept
//! only when they are CCL names ([`is_ccl_name`]), in the case they were written in; Cactus
//! compares implementation and capability names case-insensitively, so callers should too
//! ([`Need`] does).

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

/// What one thorn declares about its dependencies.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ThornFacts {
    pub implements: Option<String>,
    pub inherits: Vec<String>,
    pub friends: Vec<String>,
    pub requires_functions: Vec<String>,
    pub uses_functions: Vec<String>,
    pub provides_functions: Vec<String>,
    pub uses_includes: Vec<String>,
    /// Capabilities from `REQUIRES` in configuration.ccl.
    pub requires: Vec<String>,
    /// Thorns from `REQUIRES THORNS:`.
    pub requires_thorns: Vec<String>,
    /// Capabilities from `OPTIONAL` and `OPTIONAL_IFACTIVE`.
    pub optional: Vec<String>,
    /// Capabilities from `PROVIDES`.
    pub provides: Vec<String>,
    /// The include names it provides for others to `USES INCLUDE`: the `y` of each
    /// `INCLUDE[S] [SOURCE|HEADER]: x IN y` in interface.ccl. Facts stored by a scanner older than
    /// this field have it empty: rescan them before trusting an unmet include.
    #[serde(default)]
    pub provides_includes: Vec<String>,
    /// The `USES INCLUDE` headers its sources `#include` outside any preprocessor conditional
    /// (an include guard is no conditional), so it does not compile without a thorn providing
    /// them ([`Need::Include`]). Read from the sources, not the CCL: [`ThornFacts::scan`] leaves it
    /// empty and [`ThornFacts::scan_sources`] fills it.
    #[serde(default)]
    pub needs_includes: Vec<String>,
}

fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim()
}

/// Splits a list written with spaces and/or commas.
fn words(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).map(str::to_owned)
}

/// A CCL name: an implementation, capability, function or thorn, `[A-Za-z0-9_]+`. The scanner
/// keeps only words of this shape as names (Cactus accepts no others), so every [`Need`] has one
/// and never needs escaping in a query string.
pub fn is_ccl_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The CCL names in a list written with spaces and/or commas; other words are dropped.
fn names(s: &str) -> impl Iterator<Item = String> + '_ {
    words(s).filter(|w| is_ccl_name(w))
}

/// An include name, as `USES INCLUDE` and `INCLUDE ... IN` write it: `TP_Guts.h`, `loop.hxx`.
/// Letters, digits, `_`, `.` and `-`, not starting with `.`, so it never needs escaping in a query
/// string and never climbs out of a directory.
pub fn is_include_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// The first word, if it is a CCL name.
fn first_name(s: &str) -> Option<String> {
    words(s).next().filter(|w| is_ccl_name(w))
}

/// A capability name without a version constraint: `HDF5(>=1.10)` → `HDF5`.
fn bare_capability(w: &str) -> Option<String> {
    let name = w.split('(').next().unwrap_or("").trim();
    is_ccl_name(name).then(|| name.to_owned())
}

/// Splits `KEYWORD rest` case-insensitively; `rest` keeps its case.
fn after_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let head = line.get(..keyword.len())?;
    if !head.eq_ignore_ascii_case(keyword) {
        return None;
    }
    let rest = &line[keyword.len()..];
    // The keyword must end at a word boundary.
    match rest.chars().next() {
        None => Some(""),
        Some(c) if c.is_whitespace() || c == ':' || c == '{' || c == '(' => Some(rest),
        Some(_) => None,
    }
}

fn after_colon(rest: &str) -> &str {
    rest.trim_start().strip_prefix(':').unwrap_or(rest).trim()
}

/// Lines outside `{ ... }` blocks, comments stripped, with any text before an opening `{` kept.
fn top_level_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    for raw in text.lines() {
        let line = strip_comment(raw);
        let mut kept = String::new();
        for c in line.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                _ if depth == 0 => kept.push(c),
                _ => {}
            }
        }
        let kept = kept.trim();
        if !kept.is_empty() {
            out.push(kept.to_owned());
        }
    }
    out
}

impl ThornFacts {
    /// Reads `interface.ccl` and, if present, `configuration.ccl` text.
    pub fn scan(interface: &str, configuration: Option<&str>) -> ThornFacts {
        let mut f = ThornFacts::default();
        for line in interface.lines().map(strip_comment).filter(|l| !l.is_empty()) {
            if let Some(rest) = after_keyword(line, "IMPLEMENTS") {
                f.implements = first_name(after_colon(rest));
            } else if let Some(rest) = after_keyword(line, "INHERITS") {
                f.inherits.extend(names(after_colon(rest)));
            } else if let Some(rest) = after_keyword(line, "FRIENDS").or_else(|| after_keyword(line, "FRIEND")) {
                f.friends.extend(names(after_colon(rest)));
            } else if let Some(rest) = after_keyword(line, "REQUIRES") {
                if let Some(name) = after_keyword(rest.trim_start(), "FUNCTION") {
                    f.requires_functions.extend(first_name(name));
                }
            } else if let Some(rest) = after_keyword(line, "PROVIDES") {
                if let Some(name) = after_keyword(rest.trim_start(), "FUNCTION") {
                    f.provides_functions.extend(first_name(name));
                }
            } else if let Some(rest) = after_keyword(line, "INCLUDES").or_else(|| after_keyword(line, "INCLUDE")) {
                // INCLUDE[S] [SOURCE|HEADER]: file.h IN name.h
                let rest = rest.trim_start();
                let rest = after_keyword(rest, "SOURCE").or_else(|| after_keyword(rest, "HEADER")).unwrap_or(rest);
                let mut w = words(after_colon(rest));
                if let (Some(_file), Some(kw), Some(name)) = (w.next(), w.next(), w.next())
                    && kw.eq_ignore_ascii_case("in")
                    && is_include_name(&name)
                {
                    f.provides_includes.push(name);
                }
            } else if let Some(rest) = after_keyword(line, "USES") {
                let rest = rest.trim_start();
                if let Some(name) = after_keyword(rest, "FUNCTION") {
                    f.uses_functions.extend(first_name(name));
                } else if let Some(inc) = after_keyword(rest, "INCLUDE") {
                    // USES INCLUDE [SOURCE|HEADER]: file.h ...
                    let inc = inc.trim_start();
                    let inc = after_keyword(inc, "SOURCE").or_else(|| after_keyword(inc, "HEADER")).unwrap_or(inc);
                    f.uses_includes.extend(words(after_colon(inc)));
                }
            }
        }
        if let Some(conf) = configuration {
            for line in top_level_lines(conf) {
                if let Some(rest) = after_keyword(&line, "REQUIRES") {
                    let rest = rest.trim_start();
                    if let Some(thorns) = after_keyword(rest, "THORNS") {
                        f.requires_thorns.extend(names(after_colon(thorns)));
                    } else {
                        f.requires.extend(words(rest).filter_map(|w| bare_capability(&w)));
                    }
                } else if let Some(rest) =
                    after_keyword(&line, "OPTIONAL_IFACTIVE").or_else(|| after_keyword(&line, "OPTIONAL"))
                {
                    f.optional.extend(words(rest).filter_map(|w| bare_capability(&w)));
                } else if let Some(rest) = after_keyword(&line, "PROVIDES") {
                    f.provides.extend(words(rest).filter_map(|w| bare_capability(&w)).take(1));
                }
            }
        }
        for v in [
            &mut f.inherits,
            &mut f.friends,
            &mut f.requires_functions,
            &mut f.uses_functions,
            &mut f.provides_functions,
            &mut f.uses_includes,
            &mut f.requires,
            &mut f.requires_thorns,
            &mut f.optional,
            &mut f.provides,
            &mut f.provides_includes,
        ] {
            let mut seen = std::collections::BTreeSet::new();
            v.retain(|x| seen.insert(x.to_ascii_lowercase()));
        }
        f
    }

    /// Reads which of the thorn's `USES INCLUDE` headers its sources (under `thorn_dir/src`)
    /// `#include` outside any preprocessor conditional, into [`ThornFacts::needs_includes`]. Such
    /// a thorn does not compile without a thorn that provides the header: Cactus makes the header
    /// it includes from every provider in the configuration, and from none it is empty. A header
    /// included only under `#if`/`#ifdef` is optional, and not read as needed. An include guard
    /// (`#ifndef X` followed by `#define X`) is not a conditional.
    ///
    /// The tree may be untrusted: symlinks are never followed, hidden directories are skipped,
    /// and at most [`MAX_SOURCE_FILES`] files of at most [`MAX_SOURCE_BYTES`] each are read.
    pub fn scan_sources(&mut self, thorn_dir: &Path) {
        self.scan_sources_until(thorn_dir, &AtomicBool::new(false));
    }

    /// [`ThornFacts::scan_sources`], stopping before the next directory or file once `stop` is
    /// set (a caller's Ctrl-C flag). Returns whether the scan finished; a stopped scan leaves
    /// [`ThornFacts::needs_includes`] empty, never a partial answer that looks complete.
    pub fn scan_sources_until(&mut self, thorn_dir: &Path, stop: &AtomicBool) -> bool {
        self.needs_includes.clear();
        if self.uses_includes.is_empty() {
            return true;
        }
        let mut found: Vec<String> = Vec::new();
        // Macros the sources test (`#ifdef X`, `defined(X)`), normalized.
        let mut tested: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut budget = MAX_SOURCE_FILES;
        let mut stack = vec![thorn_dir.join("src")];
        while let Some(dir) = stack.pop() {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                if stop.load(Ordering::Relaxed) {
                    return false;
                }
                let name = e.file_name().to_string_lossy().into_owned();
                let Ok(ty) = e.file_type() else { continue };
                if name.starts_with('.') || ty.is_symlink() {
                    continue;
                }
                if ty.is_dir() {
                    stack.push(e.path());
                } else if ty.is_file() && is_source(&name) {
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    let Some(text) = read_bounded(&e.path(), MAX_SOURCE_BYTES) else { continue };
                    tested.extend(tested_macros(&text).iter().map(|m| guard_key(m)));
                    for inc in unguarded_includes(&text) {
                        if self.uses_includes.contains(&inc) && !found.contains(&inc) {
                            found.push(inc);
                        }
                    }
                }
            }
        }
        // In the order interface.ccl declares them. A header whose own guard
        // macro the sources test (`#include "http_Content.h"`, then
        // `#ifdef __HTTP_CONTENT_H__`) is used only when a provider filled it:
        // included, but not needed.
        self.needs_includes = (self.uses_includes.iter())
            .filter(|u| found.contains(u) && is_include_name(u) && !tested.contains(&guard_key(u)))
            .cloned()
            .collect();
        true
    }
}

/// Source files read for `#include`s: at most this many per thorn.
pub const MAX_SOURCE_FILES: usize = 2000;
/// Source files larger than this are not read.
pub const MAX_SOURCE_BYTES: u64 = 1024 * 1024;

/// Whether a file name is a source the preprocessor reads: C, C++, CUDA, preprocessed Fortran,
/// and the headers and `.inc` files they include.
fn is_source(name: &str) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else { return false };
    matches!(
        ext,
        "c" | "cc" | "cxx" | "cpp" | "C" | "h" | "hh" | "hxx" | "hpp" | "inc" | "cu" | "cuh" | "F" | "F90" | "F77"
            | "f" | "f90" | "f77"
    )
}

/// The macros `text` tests: `#ifdef X`, `#ifndef X`, and each `defined(X)` / `defined X` of an
/// `#if` or `#elif`.
fn tested_macros(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(d) = line.trim_start().strip_prefix('#') else { continue };
        let d = d.trim_start();
        if let Some(rest) = d.strip_prefix("ifdef").or_else(|| d.strip_prefix("ifndef")) {
            out.extend(rest.split_whitespace().next().map(str::to_owned));
        } else if d.starts_with("if") || d.starts_with("elif") {
            let mut rest = d;
            while let Some(i) = rest.find("defined") {
                rest = &rest[i + "defined".len()..];
                let name: String = rest
                    .trim_start_matches(|c: char| c.is_whitespace() || c == '(')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    out.push(name);
                }
            }
        }
    }
    out
}

/// What a header name and its guard macro share: `http_Content.h` and `__HTTP_CONTENT_H__` both
/// read `HTTPCONTENTH`.
fn guard_key(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect()
}

/// The `#include "x"` and `#include <x>` names in `text` that no preprocessor conditional
/// encloses, an include guard aside. Comments are not parsed: an `#include` must start its line
/// (after blanks), as the preprocessor requires.
fn unguarded_includes(text: &str) -> Vec<String> {
    // One entry per open conditional: whether it is an include guard.
    let mut open: Vec<bool> = Vec::new();
    // An `#ifndef X` just seen, waiting to see whether `#define X` comes next.
    let mut pending: Option<String> = None;
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(directive) = line.trim_start().strip_prefix('#') else { continue };
        let directive = directive.trim_start();
        let (word, rest) = directive.split_once(|c: char| c.is_whitespace() || c == '"' || c == '<').map_or(
            (directive, ""),
            |(w, _)| (w, directive[w.len()..].trim_start()),
        );
        let guard = pending.take();
        match word {
            "ifndef" => {
                open.push(false);
                pending = rest.split_whitespace().next().map(str::to_owned);
            }
            "if" | "ifdef" => open.push(false),
            "endif" => {
                open.pop();
            }
            "define" => {
                if let (Some(g), Some(last)) = (guard, open.last_mut())
                    && rest.split_whitespace().next() == Some(g.as_str())
                {
                    *last = true;
                }
            }
            "include" if open.iter().all(|&g| g) => {
                let name = rest
                    .strip_prefix('"')
                    .and_then(|r| r.split_once('"'))
                    .or_else(|| rest.strip_prefix('<').and_then(|r| r.split_once('>')))
                    .map(|(n, _)| n.trim());
                if let Some(name) = name.filter(|n| !n.is_empty()) {
                    out.push(name.to_owned());
                }
            }
            // `#elif`/`#else` keep the conditional open; anything else changes nothing here.
            _ => {}
        }
    }
    out
}

/// A hard CCL requirement of a thorn, as [`needs_of`] reads it from its [`ThornFacts`]. The
/// Cactarium checks every published thorn's needs against the package and its dependencies'
/// `needs` thorns; CactUp finds the thorns that meet them when it composes or trims a thornlist.
/// Names keep the case they were written in and are compared case-insensitively ([`meets`]).
///
/// Soft declarations are not needs: `OPTIONAL` capabilities, `USES FUNCTION`, and a `USES INCLUDE`
/// header the sources include only under a conditional, or not at all. A capability's version
/// constraint (`HDF5(>=1.10)`) is not read, only its name.
///
/// Equality, hashing and order are case-insensitive, by [`Need::key`], as [`meets`] compares:
/// `Need::Capability("hdf5".into()) == Need::Capability("HDF5".into())`.
#[derive(Clone, Debug)]
pub enum Need {
    /// An implementation it `INHERITS` from or is a `FRIEND` of: met by a thorn that `IMPLEMENTS` it.
    Implementation(String),
    /// A capability it `REQUIRES` (configuration.ccl): met by a thorn that `PROVIDES` it.
    Capability(String),
    /// A thorn named in `REQUIRES THORNS`: met by the thorn of that name.
    Thorn(String),
    /// A function it `REQUIRES FUNCTION` (an aliased function): met by a thorn that
    /// `PROVIDES FUNCTION` it.
    Function(String),
    /// A header it `USES INCLUDE` and its sources `#include` outside any conditional
    /// ([`ThornFacts::needs_includes`]): met by a thorn whose interface.ccl has
    /// `INCLUDE[S] ...: x IN` it.
    Include(String),
}

impl Need {
    /// The name required: an implementation, capability, thorn or function, as written.
    pub fn name(&self) -> &str {
        match self {
            Need::Implementation(n) | Need::Capability(n) | Need::Thorn(n) | Need::Function(n) | Need::Include(n) => {
                n
            }
        }
    }

    /// What two needs are compared by: the kind, and the name folded to lowercase.
    pub fn key(&self) -> (u8, String) {
        let kind = match self {
            Need::Implementation(_) => 0,
            Need::Capability(_) => 1,
            Need::Thorn(_) => 2,
            Need::Function(_) => 3,
            Need::Include(_) => 4,
        };
        (kind, self.name().to_ascii_lowercase())
    }
}

impl PartialEq for Need {
    fn eq(&self, other: &Need) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Need {}

impl std::hash::Hash for Need {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state)
    }
}

impl PartialOrd for Need {
    fn partial_cmp(&self, other: &Need) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Need {
    fn cmp(&self, other: &Need) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

/// What the requiring thorn does, for a message: `{thorn} {need}` reads
/// "Z4c inherits or befriends ADMBase".
impl fmt::Display for Need {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Need::Implementation(i) => write!(f, "inherits or befriends {i}"),
            Need::Capability(c) => write!(f, "requires the capability {c}"),
            Need::Thorn(t) => write!(f, "requires the thorn {t}"),
            Need::Function(x) => write!(f, "requires the function {x}"),
            Need::Include(h) => write!(f, "includes the header {h}"),
        }
    }
}

/// The hard requirements of a thorn with `facts`: the implementations it inherits from or
/// befriends, then the capabilities, thorns and functions it requires, then the headers it cannot
/// compile without, each once (compared case-insensitively, the first spelling kept). Only CCL
/// names ([`is_ccl_name`]) count, and include names ([`is_include_name`]) for a header: facts
/// stored by an older scanner may hold other words, which Cactus would refuse and nothing can
/// meet, so every need has such a name and fits a `/providers` query.
pub fn needs_of(facts: &ThornFacts) -> Vec<Need> {
    let mut v: Vec<Need> = Vec::new();
    v.extend(facts.inherits.iter().chain(&facts.friends).map(|i| Need::Implementation(i.clone())));
    v.extend(facts.requires.iter().map(|c| Need::Capability(c.clone())));
    v.extend(facts.requires_thorns.iter().map(|t| Need::Thorn(t.clone())));
    v.extend(facts.requires_functions.iter().map(|x| Need::Function(x.clone())));
    v.extend(facts.needs_includes.iter().map(|h| Need::Include(h.clone())));
    let mut seen = std::collections::HashSet::new();
    v.retain(|n| {
        let named = match n {
            Need::Include(h) => is_include_name(h),
            _ => is_ccl_name(n.name()),
        };
        named && seen.insert(n.clone())
    });
    v
}

/// Whether the thorn `thorn` (its name, as Cactus knows it) with `facts` meets `need`. Names
/// compare case-insensitively, as Cactus compares them. A thorn without an `IMPLEMENTS` line
/// implements nothing (Cactus refuses to build one), so it never meets an implementation need.
pub fn meets(thorn: &str, facts: &ThornFacts, need: &Need) -> bool {
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match need {
        Need::Implementation(i) => facts.implements.as_deref().is_some_and(|x| eq(x, i)),
        Need::Capability(c) => facts.provides.iter().any(|p| eq(p, c)),
        Need::Thorn(t) => eq(thorn, t),
        Need::Function(x) => facts.provides_functions.iter().any(|p| eq(p, x)),
        Need::Include(h) => facts.provides_includes.iter().any(|p| eq(p, h)),
    }
}

/// The needs of a thorn with `facts` that none of `thorns` (name and facts, the thorn itself
/// included if it should count) meets, in [`needs_of`]'s order. Empty means the set meets them all.
pub fn unmet<'a, I>(facts: &ThornFacts, thorns: I) -> Vec<Need>
where
    I: IntoIterator<Item = (&'a str, &'a ThornFacts)>,
    I::IntoIter: Clone,
{
    let thorns = thorns.into_iter();
    needs_of(facts).into_iter().filter(|need| !thorns.clone().any(|(name, f)| meets(name, f, need))).collect()
}

/// Whether an unmet `need` of the thorn with `facts` counts against a set, given every thorn the
/// checker knows of (`known`: the registry's packages, an installation's repos). An
/// [`Need::Include`] counts only where some thorn of `known` provides the header, and none of
/// those is optional to the thorn:
/// - Cactus compiles a header that nothing provides as an empty one, which is how a hook others
///   may fill works (ADMMacros' `CalcTmunu.inc`, Carpet's `Requirements.hh`), and a header can
///   reach a thorn another way (a capability's include directory).
/// - A provider that `PROVIDES` or `IMPLEMENTS` something the thorn declares `OPTIONAL` (or
///   `OPTIONAL_IFACTIVE`) is one it builds without, using the header only when it is there
///   (Slab's `loopcontrol.h`, GRHydro's `carpet.hh`).
///
/// One a known thorn provides otherwise is that thorn's code, and an empty stand-in for it does
/// not compile (EinsteinInitialData/TwoPuncturesX without TwoPunctures). Every other need always
/// counts.
pub fn counts<'a>(facts: &ThornFacts, need: &Need, known: impl IntoIterator<Item = &'a ThornFacts>) -> bool {
    let Need::Include(h) = need else { return true };
    let optional = |p: &ThornFacts| {
        facts.optional.iter().any(|o| {
            p.provides.iter().any(|c| c.eq_ignore_ascii_case(o))
                || p.implements.as_deref().is_some_and(|i| i.eq_ignore_ascii_case(o))
        })
    };
    let mut provided = false;
    for p in known.into_iter().filter(|p| p.provides_includes.iter().any(|x| x.eq_ignore_ascii_case(h))) {
        if optional(p) {
            return false;
        }
        provided = true;
    }
    provided
}

/// [`unmet`], keeping only the needs that count given the thorns of `known` ([`counts`]): what a
/// closure check of a thornlist (or of a package and its dependencies) refuses.
pub fn unmet_among<'a, 'k, I, K>(facts: &ThornFacts, thorns: I, known: K) -> Vec<Need>
where
    I: IntoIterator<Item = (&'a str, &'a ThornFacts)>,
    I::IntoIter: Clone,
    K: IntoIterator<Item = &'k ThornFacts>,
    K::IntoIter: Clone,
{
    let known = known.into_iter();
    unmet(facts, thorns).into_iter().filter(|need| counts(facts, need, known.clone())).collect()
}

/// A thorn found in a source tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ScannedThorn {
    /// The thorn's directory relative to the scanned root (empty if the root is the thorn).
    pub dir: PathBuf,
    /// The directory's name; `None` when the root itself is the thorn.
    pub name: Option<String>,
    /// The parent directory's name, when the thorn sits at `Arrangement/Thorn`.
    pub arrangement: Option<String>,
    pub facts: ThornFacts,
}

/// CCL files larger than this are not read (real ones are a few kB).
pub const MAX_CCL_BYTES: u64 = 1024 * 1024;

/// Reads a CCL file only if it is a regular file (never through a symlink, which could point
/// outside the tree or at a device) and not absurdly large. The tree may be untrusted.
fn read(path: &Path) -> Option<String> {
    read_bounded(path, MAX_CCL_BYTES)
}

/// A regular file's text (never through a symlink) of at most `max` bytes; invalid UTF-8 is
/// replaced, since sources may carry Latin-1 comments.
fn read_bounded(path: &Path, max: u64) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > max {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn scan_dir(root: &Path, rel: &Path, out: &mut Vec<ScannedThorn>) {
    let dir = root.join(rel);
    if let Some(iface) = read(&dir.join("interface.ccl")) {
        let conf = read(&dir.join("configuration.ccl"));
        let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
        let (first, second) = (parts.next(), parts.next());
        let (arrangement, name) = match (first, second) {
            (Some(a), Some(t)) => (Some(a), Some(t)),
            (Some(t), None) => (None, Some(t)),
            _ => (None, None),
        };
        let mut facts = ThornFacts::scan(&iface, conf.as_deref());
        facts.scan_sources(&dir);
        out.push(ScannedThorn { dir: rel.to_owned(), name, arrangement, facts });
    }
}

/// Finds thorns (directories with an `interface.ccl`) at the root, one level down
/// (`Thorn/`, when the root is an arrangement), and two levels down (`Arrangement/Thorn/`), with
/// their facts from the CCL and their sources ([`ThornFacts::scan_sources`]).
/// Hidden directories, symlinks (to directories or to CCL files) and CCL files over
/// [`MAX_CCL_BYTES`] are skipped, so an untrusted tree cannot make this read outside itself.
/// Sorted by directory.
pub fn scan_tree(root: &Path) -> Vec<ScannedThorn> {
    fn subdirs(dir: &Path) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut v: Vec<PathBuf> = rd
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| PathBuf::from(e.file_name()))
            .collect();
        v.sort();
        v
    }
    let mut out = Vec::new();
    scan_dir(root, Path::new(""), &mut out);
    for a in subdirs(root) {
        scan_dir(root, &a, &mut out);
        for t in subdirs(&root.join(&a)) {
            scan_dir(root, &a.join(&t), &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_facts() {
        let iface = "\
# Interface definition for thorn CarpetX
IMPLEMENTS: CarpetX
INHERITS: grid, IO
friends: ADMBase
USES INCLUDE HEADER: loop_device.hxx
CCTK_INT FUNCTION GetCallFunctionCount()
REQUIRES FUNCTION GetCallFunctionCount  # comment
PROVIDES FUNCTION GetRefinementLevel WITH CarpetX_GetRefinementLevel LANGUAGE C
USES FUNCTION Boundary_SelectGroupForBC
";
        let f = ThornFacts::scan(iface, None);
        assert_eq!(f.implements.as_deref(), Some("CarpetX"));
        assert_eq!(f.inherits, ["grid", "IO"]);
        assert_eq!(f.friends, ["ADMBase"]);
        assert_eq!(f.uses_includes, ["loop_device.hxx"]);
        assert_eq!(f.requires_functions, ["GetCallFunctionCount"]);
        assert_eq!(f.provides_functions, ["GetRefinementLevel"]);
        assert_eq!(f.uses_functions, ["Boundary_SelectGroupForBC"]);
    }

    #[test]
    fn configuration_facts() {
        let conf = "\
# Configuration definitions for thorn CarpetX
REQUIRES AMReX MPI yaml_cpp zlib HDF5(>=1.10)
REQUIRES THORNS: Arith, CarpetXRegrid Loop
OPTIONAL ADIOS2 openPMD_api Silo
{
  DEFINE HAVE_IO
}
PROVIDES CarpetX
{
  SCRIPT configure.sh
  LANG bash
}
";
        let f = ThornFacts::scan("IMPLEMENTS: CarpetX", Some(conf));
        assert_eq!(f.requires, ["AMReX", "MPI", "yaml_cpp", "zlib", "HDF5"]);
        assert_eq!(f.requires_thorns, ["Arith", "CarpetXRegrid", "Loop"]);
        assert_eq!(f.optional, ["ADIOS2", "openPMD_api", "Silo"]);
        assert_eq!(f.provides, ["CarpetX"]);
    }

    /// Representative facts: Z4c on CarpetX, as the ET builds it.
    fn z4c() -> ThornFacts {
        ThornFacts::scan(
            "IMPLEMENTS: Z4c\nINHERITS: ADMBaseX, TmunuBaseX\nFRIENDS: admbasex\n\
             CCTK_INT FUNCTION GetCallFunctionCount()\nREQUIRES FUNCTION GetCallFunctionCount\n\
             USES FUNCTION GetLoopDevice\nUSES INCLUDE HEADER: loop_device.hxx\n",
            Some("REQUIRES Arith Loop\nREQUIRES THORNS: CarpetX\nOPTIONAL Silo\n{\n}\n"),
        )
    }

    #[test]
    fn needs_are_the_hard_requirements() {
        assert_eq!(
            needs_of(&z4c()),
            [
                Need::Implementation("ADMBaseX".into()),
                Need::Implementation("TmunuBaseX".into()),
                Need::Capability("Arith".into()),
                Need::Capability("Loop".into()),
                Need::Thorn("CarpetX".into()),
                Need::Function("GetCallFunctionCount".into()),
            ],
            "a friend that is also inherited once; no optional capability, used function or include"
        );
        assert_eq!(Need::Implementation("ADMBaseX".into()).to_string(), "inherits or befriends ADMBaseX");
        assert_eq!(Need::Function("F".into()).to_string(), "requires the function F");
        // The same name as two kinds of need is two needs.
        let f = ThornFacts::scan("IMPLEMENTS: X\nINHERITS: Loop", Some("REQUIRES Loop"));
        assert_eq!(needs_of(&f), [Need::Implementation("Loop".into()), Need::Capability("Loop".into())]);
    }

    #[test]
    fn what_meets_a_need() {
        let admbasex = ThornFacts::scan("implements: admbasex", None);
        let carpetx = ThornFacts::scan(
            "IMPLEMENTS: CarpetX\nPROVIDES FUNCTION GetCallFunctionCount WITH CarpetX_GetCallFunctionCount LANGUAGE C",
            Some("PROVIDES Loop\n{\n}\n"),
        );
        assert!(meets("ADMBaseX", &admbasex, &Need::Implementation("ADMBaseX".into())), "case-insensitive");
        assert!(meets("CarpetX", &carpetx, &Need::Capability("loop".into())));
        assert!(meets("CarpetX", &carpetx, &Need::Function("getcallfunctioncount".into())), "by its alias");
        assert!(!meets("CarpetX", &carpetx, &Need::Function("CarpetX_GetCallFunctionCount".into())), "not by the provider's own name");
        assert!(meets("carpetx", &carpetx, &Need::Thorn("CarpetX".into())));
        assert!(!meets("CarpetX", &carpetx, &Need::Implementation("Loop".into())), "a capability is not an implementation");
        // A thorn's name is not its implementation: without IMPLEMENTS it implements nothing.
        let anonymous = ThornFacts::scan("INHERITS: grid", None);
        assert!(!meets("ADMBaseX", &anonymous, &Need::Implementation("ADMBaseX".into())));
        assert!(meets("ADMBaseX", &anonymous, &Need::Thorn("ADMBaseX".into())), "but it is still that thorn");
    }

    #[test]
    fn unmet_needs_of_a_set() {
        let z4c = z4c();
        let admbasex = ThornFacts::scan("IMPLEMENTS: ADMBaseX", None);
        let tmunu = ThornFacts::scan("IMPLEMENTS: TmunuBaseX", None);
        let carpetx = ThornFacts::scan(
            "IMPLEMENTS: CarpetX\nPROVIDES FUNCTION GetCallFunctionCount WITH X LANGUAGE C",
            Some("PROVIDES Loop\n{\n}\n"),
        );
        let arith = ThornFacts::scan("IMPLEMENTS: Arith", Some("PROVIDES Arith\n{\n}\n"));
        let mut set = vec![("Z4c", &z4c), ("ADMBaseX", &admbasex), ("CarpetX", &carpetx)];
        assert_eq!(unmet(&z4c, set.iter().copied()), [Need::Implementation("TmunuBaseX".into()), Need::Capability("Arith".into())]);
        set.extend([("TmunuBaseX", &tmunu), ("Arith", &arith)]);
        assert!(unmet(&z4c, set.iter().copied()).is_empty());
        assert_eq!(unmet(&z4c, []).len(), 6, "nothing meets anything");
    }

    /// Needs compare as `meets` does: by kind, and by name without case.
    #[test]
    fn needs_compare_without_case() {
        use std::collections::{BTreeSet, HashSet};
        assert_eq!(Need::Capability("hdf5".into()), Need::Capability("HDF5".into()));
        assert_ne!(Need::Capability("Loop".into()), Need::Implementation("Loop".into()));
        let set: HashSet<Need> = [Need::Function("GetX".into()), Need::Function("getx".into())].into();
        assert_eq!(set.len(), 1);
        let ordered: BTreeSet<Need> = [Need::Thorn("carpetx".into()), Need::Thorn("CarpetX".into())].into();
        assert_eq!(ordered.len(), 1);
        assert_eq!(Need::Thorn("CarpetX".into()).key(), (2, "carpetx".to_owned()));
        // The first spelling is kept.
        let f = ThornFacts::scan("IMPLEMENTS: X\nINHERITS: ADMBase\nFRIENDS: admbase", None);
        assert_eq!(needs_of(&f)[0].name(), "ADMBase");
        assert_eq!(needs_of(&f).len(), 1);
    }

    /// `REQUIRES THORNS` names a thorn in any case; the thorn of that name meets it in any other.
    #[test]
    fn mixed_case_required_thorns() {
        let f = ThornFacts::scan("IMPLEMENTS: Z4c", Some("REQUIRES THORNS: CarpetX, loop\n"));
        let carpetx = ThornFacts::scan("IMPLEMENTS: CarpetX", None);
        let looped = ThornFacts::scan("IMPLEMENTS: Loop", None);
        for name in ["CarpetX", "carpetx", "CARPETX"] {
            assert!(meets(name, &carpetx, &Need::Thorn("CarpetX".into())), "{name}");
        }
        assert!(unmet(&f, [("carpetx", &carpetx), ("Loop", &looped)]).is_empty());
        assert_eq!(unmet(&f, [("Loop", &looped)]), [Need::Thorn("CarpetX".into())]);
    }

    /// Only CCL names are kept, so every need has one.
    #[test]
    fn names_are_ccl_names() {
        let f = ThornFacts::scan(
            "IMPLEMENTS: 2D_Ok\nINHERITS: good, b@d, 3rd\nREQUIRES FUNCTION F-1\nPROVIDES FUNCTION G WITH H LANGUAGE C",
            Some("REQUIRES 1HDF5(>=1) sp-ace\nREQUIRES THORNS: Fine, no/pe\n"),
        );
        assert_eq!(f.implements.as_deref(), Some("2D_Ok"));
        assert_eq!(f.inherits, ["good", "3rd"]);
        assert!(f.requires_functions.is_empty());
        assert_eq!(f.provides_functions, ["G"]);
        assert_eq!(f.requires, ["1HDF5"]);
        assert_eq!(f.requires_thorns, ["Fine"]);
        assert!(needs_of(&f).iter().all(|n| is_ccl_name(n.name())));
    }

    #[test]
    fn provided_includes() {
        let f = ThornFacts::scan(
            "IMPLEMENTS: TwoPunctures\nINCLUDE HEADER: TwoPunctures.h in TwoPunctures.h\n\
             INCLUDES SOURCE: guts.inc IN TP_Guts.h\ninclude: a.h in sub/../b.h\nINCLUDE HEADER: c.h\n",
            None,
        );
        assert_eq!(f.provides_includes, ["TwoPunctures.h", "TP_Guts.h"], "never a path, never without IN");
        assert!(f.needs_includes.is_empty(), "the CCL alone never says a header is needed");
    }

    #[test]
    fn includes_outside_conditionals() {
        let text = "\
#ifndef TP_X_H
#define TP_X_H
#include \"TP_Guts.h\"
#  include <loop.hxx>
#ifdef CCTK_MPI
#include \"mpi_only.h\"
#else
#include \"nompi.h\"
#endif
#if 0
#include \"never.h\"
#endif
#ifndef HAVE_X
#error no
#include \"guarded_too.h\"
#endif
  #include \"after.h\"
#endif
";
        assert_eq!(unguarded_includes(text), ["TP_Guts.h", "loop.hxx", "after.h"]);
        // A conditional that is not a guard: #ifndef followed by something else.
        assert!(unguarded_includes("#ifndef A\n#define B\n#include \"x.h\"\n#endif\n").is_empty());
    }

    #[test]
    fn sources_say_which_used_includes_are_needed() {
        let root = std::env::temp_dir().join(format!("ccl-sources-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/sub")).unwrap();
        std::fs::create_dir_all(root.join("src/.hidden")).unwrap();
        std::fs::write(root.join("src/a.cc"), "#include \"TP_Guts.h\"\n#include \"unused_by_ccl.h\"\n").unwrap();
        std::fs::write(root.join("src/sub/b.F90"), "#ifdef X\n#include \"opt.h\"\n#endif\n").unwrap();
        std::fs::write(root.join("src/c.cc"), "#include \"http_Content.h\"\n#if 1 && defined(__HTTP_CONTENT_H__)\nx\n#endif\n").unwrap();
        std::fs::write(root.join("src/notes.txt"), "#include \"txt.h\"\n").unwrap();
        std::fs::write(root.join("src/.hidden/c.c"), "#include \"hidden.h\"\n").unwrap();
        let mut f = ThornFacts::scan(
            "IMPLEMENTS: X\nUSES INCLUDE: opt.h txt.h hidden.h TP_Guts.h http_Content.h\n",
            None,
        );
        f.scan_sources(&root);
        assert_eq!(f.needs_includes, ["TP_Guts.h"]);
        assert_eq!(needs_of(&f), [Need::Include("TP_Guts.h".into())]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A scan whose stop flag is set reads nothing and says so, leaving no needs behind (not even
    /// an earlier scan's); one whose flag stays clear is the plain scan.
    #[test]
    fn a_stopped_source_scan_says_so_and_leaves_no_needs() {
        let root = std::env::temp_dir().join(format!("ccl-sources-stop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.cc"), "#include \"TP_Guts.h\"\n").unwrap();
        let mut f = ThornFacts::scan("IMPLEMENTS: X\nUSES INCLUDE: TP_Guts.h\n", None);
        assert!(f.scan_sources_until(&root, &AtomicBool::new(false)));
        assert_eq!(f.needs_includes, ["TP_Guts.h"]);
        assert!(!f.scan_sources_until(&root, &AtomicBool::new(true)));
        assert!(f.needs_includes.is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_included_header_is_met_by_its_provider() {
        let tpx = ThornFacts {
            implements: Some("TwoPuncturesX".into()),
            uses_includes: vec!["TP_Guts.h".into()],
            needs_includes: vec!["TP_Guts.h".into()],
            ..Default::default()
        };
        let tp = ThornFacts::scan("IMPLEMENTS: TwoPunctures\nINCLUDE HEADER: guts.h in TP_Guts.h\n", None);
        assert_eq!(unmet(&tpx, [("TwoPuncturesX", &tpx)]), [Need::Include("TP_Guts.h".into())]);
        assert!(unmet(&tpx, [("TwoPuncturesX", &tpx), ("TwoPunctures", &tp)]).is_empty());
        assert_eq!(Need::Include("TP_Guts.h".into()).to_string(), "includes the header TP_Guts.h");
        assert!(!meets("TP_Guts", &tpx, &Need::Include("TP_Guts.h".into())), "a name is not a header");
        // Held to only where a known thorn provides the header: one nothing provides is a hook.
        assert!(unmet_among(&tpx, [("TwoPuncturesX", &tpx)], [&tpx]).is_empty());
        assert_eq!(unmet_among(&tpx, [("TwoPuncturesX", &tpx)], [&tpx, &tp]), [Need::Include("TP_Guts.h".into())]);
        let hook = ThornFacts { implements: Some("ADMMacros".into()), needs_includes: vec!["CalcTmunu.inc".into()], ..Default::default() };
        assert!(!counts(&hook, &Need::Include("CalcTmunu.inc".into()), [&hook, &tp]));
        assert!(counts(&hook, &Need::Thorn("Anything".into()), []), "other needs always count");
        // A provider the thorn declares optional (a capability it provides, or what it
        // implements) is one it builds without.
        let carpet = ThornFacts::scan("IMPLEMENTS: Driver\nINCLUDE HEADER: carpet.hh in carpet.hh\n", Some("PROVIDES Carpet\n{\n}\n"));
        let grhydro = ThornFacts {
            optional: ThornFacts::scan("IMPLEMENTS: GRHydro", Some("OPTIONAL_IFACTIVE Carpet\n{\n}\n")).optional,
            needs_includes: vec!["carpet.hh".into()],
            ..Default::default()
        };
        assert_eq!(grhydro.optional, ["Carpet"]);
        assert!(!counts(&grhydro, &Need::Include("carpet.hh".into()), [&carpet]));
        let lc = ThornFacts::scan("IMPLEMENTS: LoopControl\nINCLUDE HEADER: lc.h in loopcontrol.h\n", None);
        let slab = ThornFacts { optional: vec!["loopcontrol".into()], ..Default::default() };
        assert!(!counts(&slab, &Need::Include("loopcontrol.h".into()), [&lc]), "by its implementation, any case");
        // Facts stored before the field existed read as needing no header.
        let old: ThornFacts = serde_json::from_str(r#"{"implements":"X","inherits":[],"friends":[],
            "requires-functions":[],"uses-functions":[],"provides-functions":[],"uses-includes":["a.h"],
            "requires":[],"requires-thorns":[],"optional":[],"provides":[]}"#).unwrap();
        assert!(needs_of(&old).is_empty());
    }

    #[test]
    fn keywords_need_word_boundaries() {
        let f = ThornFacts::scan("IMPLEMENTSX: nope\nIMPLEMENTS:Yes", None);
        assert_eq!(f.implements.as_deref(), Some("Yes"));
    }

    #[test]
    fn scans_both_tree_layouts() {
        let root = std::env::temp_dir().join(format!("cactarium-ccl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["CarpetX/Loop", "CarpetX/Arith", ".git/x", "doc"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("CarpetX/Loop/interface.ccl"), "IMPLEMENTS: Loop\n").unwrap();
        std::fs::write(root.join("CarpetX/Arith/interface.ccl"), "IMPLEMENTS: Arith\n").unwrap();
        std::fs::write(root.join(".git/x/interface.ccl"), "IMPLEMENTS: Hidden\n").unwrap();
        let found = scan_tree(&root);
        let names: Vec<_> = found.iter().map(|t| (t.arrangement.clone(), t.name.clone())).collect();
        assert_eq!(
            names,
            [
                (Some("CarpetX".into()), Some("Arith".into())),
                (Some("CarpetX".into()), Some("Loop".into()))
            ]
        );
        // Rooted at the arrangement, the same thorns have no arrangement.
        let found = scan_tree(&root.join("CarpetX"));
        assert!(found.iter().all(|t| t.arrangement.is_none() && t.name.is_some()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_ccl_files_are_never_followed() {
        let root = std::env::temp_dir().join(format!("cactarium-ccl-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Arr/Evil")).unwrap();
        std::fs::create_dir_all(root.join("Arr/Big")).unwrap();
        let secret = root.join("secret.txt");
        std::fs::write(&secret, "IMPLEMENTS: s3cret\n").unwrap();
        std::os::unix::fs::symlink(&secret, root.join("Arr/Evil/interface.ccl")).unwrap();
        std::os::unix::fs::symlink("/dev/zero", root.join("Arr/Evil/configuration.ccl")).unwrap();
        std::fs::write(root.join("Arr/Big/interface.ccl"), "#".repeat(MAX_CCL_BYTES as usize + 1)).unwrap();
        assert!(scan_tree(&root).is_empty(), "{:?}", scan_tree(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
