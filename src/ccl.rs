//! A conservative scanner for the dependency facts in a thorn's `interface.ccl` and
//! `configuration.ccl`.
//!
//! This is not a full CCL parser (that lives in the flesh's Perl). It extracts what the registry
//! indexes and what a thornlist trim needs:
//! - `interface.ccl`: `IMPLEMENTS`, `INHERITS`, `FRIEND(S)`, `REQUIRES/USES/PROVIDES FUNCTION`,
//!   `USES INCLUDE`.
//! - `configuration.ccl`: `REQUIRES` capabilities, `REQUIRES THORNS`, `OPTIONAL`, `PROVIDES`.
//!
//! Keywords are case-insensitive, `#` starts a comment, and the bodies of `{ ... }` blocks (a
//! `PROVIDES` block's `SCRIPT`/`LANG`, an `OPTIONAL` block's `DEFINE`) are skipped. Names keep the
//! case they were written in; Cactus compares implementation and capability names
//! case-insensitively, so callers should too.

use std::fmt;
use std::path::{Path, PathBuf};

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
    /// Capabilities from `OPTIONAL`.
    pub optional: Vec<String>,
    /// Capabilities from `PROVIDES`.
    pub provides: Vec<String>,
}

fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim()
}

/// Splits a list written with spaces and/or commas.
fn words(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).map(str::to_owned)
}

/// A capability name without a version constraint: `HDF5(>=1.10)` → `HDF5`.
fn bare_capability(w: &str) -> Option<String> {
    let name = w.split('(').next().unwrap_or("").trim();
    (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then(|| name.to_owned())
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
                f.implements = words(after_colon(rest)).next();
            } else if let Some(rest) = after_keyword(line, "INHERITS") {
                f.inherits.extend(words(after_colon(rest)));
            } else if let Some(rest) = after_keyword(line, "FRIENDS").or_else(|| after_keyword(line, "FRIEND")) {
                f.friends.extend(words(after_colon(rest)));
            } else if let Some(rest) = after_keyword(line, "REQUIRES") {
                if let Some(name) = after_keyword(rest.trim_start(), "FUNCTION") {
                    f.requires_functions.extend(words(name).take(1));
                }
            } else if let Some(rest) = after_keyword(line, "PROVIDES") {
                if let Some(name) = after_keyword(rest.trim_start(), "FUNCTION") {
                    f.provides_functions.extend(words(name).take(1));
                }
            } else if let Some(rest) = after_keyword(line, "USES") {
                let rest = rest.trim_start();
                if let Some(name) = after_keyword(rest, "FUNCTION") {
                    f.uses_functions.extend(words(name).take(1));
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
                        f.requires_thorns.extend(words(after_colon(thorns)));
                    } else {
                        f.requires.extend(words(rest).filter_map(|w| bare_capability(&w)));
                    }
                } else if let Some(rest) = after_keyword(&line, "OPTIONAL") {
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
        ] {
            let mut seen = std::collections::BTreeSet::new();
            v.retain(|x| seen.insert(x.to_ascii_lowercase()));
        }
        f
    }
}

/// A hard CCL requirement of a thorn, as [`needs_of`] reads it from its [`ThornFacts`]. The
/// Cactarium checks every published thorn's needs against the package and its dependencies'
/// `needs` thorns; CactUp finds the thorns that meet them when it composes or trims a thornlist.
/// Names keep the case they were written in and are compared case-insensitively ([`meets`]).
///
/// Soft declarations are not needs: `OPTIONAL` capabilities, `USES FUNCTION`, and `USES INCLUDE`
/// (no thorn's facts say which includes it provides yet). A capability's version constraint
/// (`HDF5(>=1.10)`) is not read, only its name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
}

impl Need {
    /// The name required: an implementation, capability, thorn or function.
    pub fn name(&self) -> &str {
        match self {
            Need::Implementation(n) | Need::Capability(n) | Need::Thorn(n) | Need::Function(n) => n,
        }
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
        }
    }
}

/// The hard requirements of a thorn with `facts`: the implementations it inherits from or
/// befriends, then the capabilities, thorns and functions it requires, each once (compared
/// case-insensitively, the first spelling kept).
pub fn needs_of(facts: &ThornFacts) -> Vec<Need> {
    let mut v: Vec<Need> = Vec::new();
    v.extend(facts.inherits.iter().chain(&facts.friends).map(|i| Need::Implementation(i.clone())));
    v.extend(facts.requires.iter().map(|c| Need::Capability(c.clone())));
    v.extend(facts.requires_thorns.iter().map(|t| Need::Thorn(t.clone())));
    v.extend(facts.requires_functions.iter().map(|x| Need::Function(x.clone())));
    let mut seen = std::collections::HashSet::new();
    v.retain(|n| seen.insert((std::mem::discriminant(n), n.name().to_ascii_lowercase())));
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
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_CCL_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
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
        out.push(ScannedThorn {
            dir: rel.to_owned(),
            name,
            arrangement,
            facts: ThornFacts::scan(&iface, conf.as_deref()),
        });
    }
}

/// Finds thorns (directories with an `interface.ccl`) at the root, one level down
/// (`Thorn/`, when the root is an arrangement), and two levels down (`Arrangement/Thorn/`).
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
