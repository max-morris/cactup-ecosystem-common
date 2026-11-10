//! Writes parsed components back out as CRL text.
//!
//! The registry uses this to freeze a stack recipe (bases + overrides) into one thornlist, and
//! cactup can use it for the sections it generates. Text that already exists is never re-emitted:
//! a flat stack or a `.sim` keeps its thornlist verbatim.
//!
//! Consecutive components whose directives are identical share one section; everything else gets
//! its own section, with `$1`/`$2` already expanded. Each checkout goes back where the parser found
//! it ([`Component::fetch_only`]): on the `!CHECKOUT` line itself (fetched but not built, since
//! Cactus's ThornList reader skips `!` lines) or on a line of its own (built). Parsing the output with [`crate::crl::parse`]
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
        &c.checksum,
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
        // A checksum names one file, so a checksummed download is a section of its own.
        while j < components.len() && first.checksum.is_none() && section_key(&components[j]) == section_key(first) {
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
        d(&mut out, "CHECKSUM_SHA256", &first.checksum);
        d(&mut out, "NAME", &first.name);
        let group = &components[i..j];
        let inline: Vec<&str> = group.iter().filter(|c| c.fetch_only).map(|c| c.checkout.as_str()).collect();
        if inline.is_empty() {
            out.push_str("!CHECKOUT   =\n");
        } else {
            let _ = writeln!(out, "!CHECKOUT   = {}", inline.join(" "));
        }
        for c in group.iter().filter(|c| !c.fetch_only) {
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
    }

    /// Checksummed downloads round-trip, each in a section of its own even when every other
    /// directive matches (a checksum names one file).
    #[test]
    fn checksums_round_trip_one_section_each() {
        let h = "d".repeat(64);
        let src = format!(
            "!CRL_VERSION = 1.0\n!TARGET = dl\n!TYPE = https\n!URL = https://x/f\n!CHECKSUM_SHA256 = {h}\n!CHECKOUT = a.tgz\n\n\
             !TARGET = dl\n!TYPE = https\n!URL = https://x/f\n!CHECKSUM_SHA256 = {h}\n!CHECKOUT = b.tgz\n"
        );
        let a = parse(&src).unwrap();
        let text = emit(a.root(), a.components(), a.disabled_thorns(), None);
        let b = parse(&text).unwrap_or_else(|e| panic!("{e:#}\n{text}"));
        assert_eq!(a.components(), b.components(), "{text}");
        assert_eq!(text.matches("!CHECKSUM_SHA256").count(), 2, "{text}");
    }

    /// Every thorn written on a `!` line, i.e. fetched but not built.
    pub(crate) fn thorns_on_bang_lines(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|l| l.trim_start().strip_prefix("!CHECKOUT")?.trim_start().strip_prefix('=').map(str::to_owned))
            .flat_map(|v| v.split_whitespace().filter(|t| t.contains('/')).map(str::to_owned).collect::<Vec<_>>())
            .collect()
    }

    /// A real, full Einstein Toolkit list survives emit and pin_text with every checkout in place.
    /// Runs when CACTUP_TEST_ET_THORNLIST names one (e.g. the ET's einsteintoolkit.th).
    #[test]
    fn a_real_et_list_round_trips() {
        let Some(path) = std::env::var_os("CACTUP_TEST_ET_THORNLIST") else { return };
        let Ok(src) = std::fs::read_to_string(path) else { return };
        let a = parse(&src).unwrap();
        assert!(a.components().iter().any(|c| c.fetch_only), "an ET list has checkout-only thorns");
        let text = emit(a.root(), a.components(), a.disabled_thorns(), None);
        assert_eq!(parse(&text).unwrap().components(), a.components());
        let pins: std::collections::BTreeMap<String, crate::crl::Pin> = a
            .components()
            .iter()
            .filter(|c| c.ty == crate::crl::ComponentType::Git)
            .map(|c| (c.repo.clone(), crate::crl::Pin::Commit("a".repeat(40))))
            .collect();
        let pinned = parse(&crate::crl::pin_text(&src, &pins).unwrap()).unwrap();
        let key = |c: &crate::crl::Component| (c.repo.clone(), c.checkout.clone(), c.fetch_only);
        let mut want: Vec<_> = a.components().iter().map(key).collect();
        let mut got: Vec<_> = pinned.components().iter().map(key).collect();
        want.sort();
        got.sort();
        assert_eq!(want, got);
    }

    /// The Einstein Toolkit's checkout-only idiom: a thorn on the !CHECKOUT line is fetched but not
    /// built, and must stay there; a thorn on its own line must stay there too.
    #[test]
    fn checkouts_keep_their_placement() {
        let src = "!CRL_VERSION = 1.0\n!TARGET = arrangements\n!TYPE = git\n!URL = https://x/ce.git\n!REPO_PATH = $2\n\
                   !CHECKOUT = CactusElliptic/EllPETSc CactusElliptic/TATPETSc\nCactusElliptic/EllBase\n#DISABLED CactusElliptic/EllPETSc\n";
        let a = parse(src).unwrap();
        let only: Vec<(&str, bool)> = a.components().iter().map(|c| (c.checkout.as_str(), c.fetch_only)).collect();
        assert_eq!(only, [("CactusElliptic/EllPETSc", true), ("CactusElliptic/TATPETSc", true), ("CactusElliptic/EllBase", false)]);
        let text = emit(a.root(), a.components(), a.disabled_thorns(), None);
        assert!(text.contains("!CHECKOUT   = CactusElliptic/EllPETSc CactusElliptic/TATPETSc\nCactusElliptic/EllBase\n"), "{text}");
        assert_eq!(parse(&text).unwrap().components(), a.components());
        // A directive written alone on its line with its checkouts below has none inline.
        let b = parse("!CRL_VERSION = 1.0\n!TARGET = a\n!TYPE = git\n!URL = https://x/r.git\n!CHECKOUT =\nA/Built\n").unwrap();
        assert!(!b.components()[0].fetch_only);
        assert!(thorns_on_bang_lines(&emit(b.root(), b.components(), &[], None)).is_empty());
    }
}
