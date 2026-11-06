//! Writes parsed components back out as CRL text.
//!
//! The registry uses this to freeze a stack recipe (bases + overrides) into one thornlist, and
//! cactup can use it for the sections it generates. Text that already exists is never re-emitted:
//! a flat stack or a `.sim` keeps its thornlist verbatim.
//!
//! Consecutive components whose directives are identical share one section; everything else gets
//! its own section, with `$1`/`$2` already expanded. Checkouts always go one per line after
//! `!CHECKOUT =`, never on that line: Cactus reads a CRL list as its ThornList too, and its reader
//! skips every `!` line. Parsing the output with [`crate::crl::parse`]
//! gives back the same components (the round-trip tests check this).

use std::fmt::Write;

use crate::crl::{Component, ComponentType};

fn type_name(t: ComponentType) -> &'static str {
    use ComponentType::*;
    match t {
        Cvs => "cvs",
        Svn => "svn",
        Git => "git",
        Darcs => "darcs",
        Http => "http",
        Https => "https",
        Ftp => "ftp",
        Hg => "hg",
        Ignore => "ignore",
    }
}

/// Everything about a component except its checkout: components with equal keys share a section.
fn section_key(c: &Component) -> impl PartialEq + '_ {
    (
        &c.ty,
        &c.target,
        &c.name,
        &c.url,
        &c.auth_url,
        &c.anon_user,
        &c.anon_pass,
        &c.repo_path,
        &c.branch,
        &c.commit,
    )
}

/// One CRL file: the header, `!DEFINE ROOT`, a section per run of alike components, then
/// `#DISABLED` lines.
pub fn emit(root: &str, components: &[Component], disabled: &[String], header_comment: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(h) = header_comment {
        for line in h.lines() {
            let _ = writeln!(out, "# {line}");
        }
        out.push('\n');
    }
    out.push_str("!CRL_VERSION = 1.0\n\n");
    let _ = writeln!(out, "!DEFINE ROOT = {root}\n");
    let mut i = 0;
    while i < components.len() {
        let first = &components[i];
        let mut j = i + 1;
        while j < components.len() && section_key(&components[j]) == section_key(first) {
            j += 1;
        }
        let d = |out: &mut String, k: &str, v: &Option<String>| {
            if let Some(v) = v {
                let _ = writeln!(out, "!{k:<11} = {v}");
            }
        };
        let _ = writeln!(out, "!{:<11} = {}", "TARGET", first.target);
        let _ = writeln!(out, "!{:<11} = {}", "TYPE", type_name(first.ty));
        d(&mut out, "URL", &first.url);
        d(&mut out, "AUTH_URL", &first.auth_url);
        d(&mut out, "ANON_USER", &first.anon_user);
        d(&mut out, "ANON_PASS", &first.anon_pass);
        d(&mut out, "REPO_PATH", &first.repo_path);
        d(&mut out, "REPO_BRANCH", &first.branch);
        d(&mut out, "REPO_COMMIT", &first.commit);
        d(&mut out, "NAME", &first.name);
        // One checkout per line, never on the !CHECKOUT line itself: Cactus also reads this file
        // as its ThornList, skipping every `!` line and squeezing out spaces.
        out.push_str("!CHECKOUT   =\n");
        for c in &components[i..j] {
            let _ = writeln!(out, "{}", c.checkout);
        }
        out.push('\n');
        i = j;
    }
    for t in disabled {
        let _ = writeln!(out, "#DISABLED {t}");
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::crl::parse;

    const SRC: &str = "\
!CRL_VERSION = 1.0
!DEFINE ROOT = Cactus
!DEFINE ARR  = $ROOT/arrangements

# Cactus Flesh
!TARGET   = $ROOT
!TYPE     = git
!URL      = https://bitbucket.org/cactuscode/cactus.git
!REPO_BRANCH = ET_2026_05
!NAME     = flesh
!CHECKOUT = .clang-format CONTRIBUTORS lib Makefile src

!TARGET   = $ARR
!TYPE     = git
!URL      = https://github.com/max-morris/CarpetX.git
!REPO_PATH= $2
!REPO_BRANCH = mixed-precision
!REPO_COMMIT = 0123456789abcdef0123456789abcdef01234567
!CHECKOUT = CarpetX/CarpetX CarpetX/Loop
#DISABLED CarpetX/PDESolvers
CarpetX/Arith

!TARGET   = $ARR
!TYPE     = git
!URL      = https://github.com/EinsteinToolkit/ExternalLibraries-$2.git
!REPO_PATH = ../$2
!CHECKOUT = ExternalLibraries/AMReX ExternalLibraries/zlib

!TARGET   = $ARR
!TYPE     = ignore
!CHECKOUT = Local/Thing
";

    #[test]
    fn round_trips_through_the_parser() {
        let a = parse(SRC).unwrap();
        let text = emit(a.root(), a.components(), a.disabled_thorns(), Some("frozen by a test"));
        let b = parse(&text).unwrap_or_else(|e| panic!("{e:#}\n{text}"));
        assert_eq!(a.components(), b.components(), "{text}");
        assert_eq!(a.disabled_thorns(), b.disabled_thorns());
        assert_eq!(a.root(), b.root());
        assert!(text.starts_with("# frozen by a test\n"));
        // The CarpetX checkouts share one section; the $2 repos get one each.
        assert_eq!(text.matches("max-morris/CarpetX.git").count(), 1, "{text}");
        assert_eq!(text.matches("ExternalLibraries-").count(), 2, "{text}");
        assert_no_thorn_on_bang_lines(&text);
    }

    /// No thorn (an `Arrangement/Thorn` checkout) may sit on a `!` line, where Cactus's ThornList
    /// reader would skip it.
    pub(crate) fn assert_no_thorn_on_bang_lines(text: &str) {
        for line in text.lines().filter(|l| l.trim_start().starts_with('!')) {
            if let Some(v) = line.split_once('=').map(|(k, v)| (k.trim(), v)).filter(|(k, _)| *k == "!CHECKOUT").map(|(_, v)| v) {
                assert!(!v.split_whitespace().any(|t| t.contains('/')), "a thorn on a ! line: {line:?}\n{text}");
            }
        }
    }

    #[test]
    fn a_lone_thorn_gets_its_own_line() {
        let a = parse("!CRL_VERSION = 1.0\n!TARGET = arrangements\n!TYPE = git\n!URL = https://x/r.git\n!CHECKOUT = A/Only\n").unwrap();
        let text = emit(a.root(), a.components(), &[], None);
        assert!(text.contains("!CHECKOUT   =\nA/Only\n"), "{text}");
        assert_no_thorn_on_bang_lines(&text);
        assert_eq!(parse(&text).unwrap().components(), a.components());
    }
}
