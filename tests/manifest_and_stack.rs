use cactup_ecosystem_common::manifest::Manifest;
use cactup_ecosystem_common::stack::FrozenStack;
use cactup_ecosystem_common::Error;

const PACKAGE: &str = r#"
format = 1

[package]
name = "carpetx"
owners = ["max-morris"]
description = "CarpetX with mixed-precision grid functions"
license = "LGPL-2.1-or-later"
default-track = "mixed-precision"

[tracks.mixed-precision]
description = "CCTK_REAL4/REAL2 storage; needs a mixed-precision flesh"

[[fork-of]]
target = "einsteintoolkit/carpetx"

[[dependency]]
target = "max-morris/cactus@mixed-precision:^1"

[[dependency]]
target = "einsteintoolkit/amrex#0123abcd"
needs = ["AMReX"]
needed-by = ["CarpetX", "TestReal2"]
note = "a pinned AMReX build"

[[works-with]]
target = "max-morris/carpetx-mp"
range = "^1"

[[incompatible-with]]
target = "einsteintoolkit/et"
range = "*"
note = "needs the mixed-precision flesh"

[[caveat]]
thorn = "TestReal2"
note = "needs _Float16 (GCC >= 12)"
"#;

#[test]
fn package_manifest_parses_and_round_trips() {
    let m = Manifest::from_toml(PACKAGE).unwrap();
    let p = m.package.as_ref().unwrap();
    assert_eq!(p.owners[0].as_str(), "max-morris");
    let dep = &m.dependencies[0];
    assert!(dep.target.req().unwrap().matches(&"mixed-precision:1.3.0".parse().unwrap()));
    assert!(dep.needs.is_empty() && dep.needed_by.is_empty());
    let pin = &m.dependencies[1];
    assert_eq!(pin.target.sha(), Some("0123abcd"));
    assert_eq!(pin.needs, ["AMReX"]);
    assert_eq!(pin.needed_by, ["CarpetX", "TestReal2"]);
    assert_eq!(m.works_with[0].target.to_string(), "max-morris/carpetx-mp@^1");
    assert_eq!(Manifest::from_toml(&m.to_toml().unwrap()).unwrap(), m);
}

const HEAD: &str = "format = 1\n[package]\nname = \"p\"\nowners = [\"o\"]\n";

#[test]
fn claims_from_before_inline_ranges_still_read() {
    // A published tag (et/cottonmouth v2026.5.0) gives the range as its own key.
    let old = format!("{HEAD}[[works-with]]\ntarget = \"et/cottonmouth-stack\"\nrange = \"^2026.5\"\n");
    let m = Manifest::from_toml(&old).unwrap();
    assert_eq!(m.works_with[0].target.to_string(), "et/cottonmouth-stack@^2026.5");
    let written = m.to_toml().unwrap();
    assert!(written.contains("target = \"et/cottonmouth-stack@^2026.5\"") && !written.contains("range"), "{written}");

    let both = format!("{HEAD}[[incompatible-with]]\ntarget = \"et/et@^2026\"\nrange = \"^2026.5\"\n");
    assert!(Manifest::from_toml(&both).is_err());
}

#[test]
fn dependency_targets_and_thorn_lists_are_checked() {
    let dep = |body: &str| Manifest::from_toml(&format!("{HEAD}[[dependency]]\n{body}\n"));
    assert!(dep("target = \"et/carpetx\"").is_ok());
    assert!(dep("target = \"et/carpetx@^2026.5\"\nneeds = [\"CarpetX\", \"Loop\"]").is_ok());
    assert!(dep("target = \"et/carpetx#2bb7f9bc10d7\"").is_ok());
    for bad in [
        "target = \"et/carpetx/Loop\"",
        "target = \"et/carpetx@head\"",
        "target = \"et/carpetx@not a range\"",
        "target = \"et/carpetx\"\nneeds = [\"1Loop\"]",
        "target = \"et/carpetx\"\nneeds = [\"Loop\", \"loop\"]",
        "target = \"et/carpetx\"\nrange = \"^2026.5\"",
        "target = \"et/carpetx\"\noptional = true",
    ] {
        assert!(dep(bad).is_err(), "{bad} should be rejected");
    }

    // needed-by names this package's thorns: only a package has them, and a listed set is checked.
    let listed = "format = 1\n[package]\nname = \"p\"\nowners = [\"o\"]\nthorns = [\"A\", \"B\"]\n";
    assert!(Manifest::from_toml(&format!("{listed}[[dependency]]\ntarget = \"et/x\"\nneeded-by = [\"B\"]\n")).is_ok());
    assert!(Manifest::from_toml(&format!("{listed}[[dependency]]\ntarget = \"et/x\"\nneeded-by = [\"C\"]\n")).is_err());
    let stack = "format = 1\n[stack]\nname = \"s\"\nowners = [\"o\"]\nextends = [\"et/et@2026.5.0\"]\n[[dependency]]\ntarget = \"et/x\"\nneeded-by = [\"A\"]\n";
    assert!(Manifest::from_toml(stack).is_err());
}

/// The carpetx-mp-forks case: ET with nine repos redirected to forks, two thorns added.
const RECIPE: &str = r#"
format = 1

[stack]
name = "carpetx-mp-forks"
extends = ["einsteintoolkit/et@2026.5.0"]
enable = ["CarpetX/TestReal4", "CarpetX/TestReal2"]

[[stack.override]]
repo = "flesh"
url = "https://github.com/max-morris/Cactus.git"
ref = "mixed-precision"

[[stack.override]]
repo = "CarpetX"
package = "max-morris/carpetx@mixed-precision:^1"

[[stack.override]]
repo = "ExternalLibraries-AMReX"
url = "https://github.com/max-morris/ExternalLibraries-AMReX.git"
ref = "mpi-cmake-hints"

[[incompatible-with]]
target = "einsteintoolkit/et"
note = "the mixed-precision flesh changes generated bindings"
"#;

/// A standard CarpetX stack: just a thornlist, no recipe.
const FLAT: &str = r#"
format = 1

[stack]
name = "carpetx"
description = "CarpetX driver, AsterX, Z4c and their external libraries"
thornlist = "carpetx.th"
"#;

/// Both: the cottonmouth union, with extra sections from a thornlist on top of two bases.
const BOTH: &str = r#"
format = 1

[stack]
name = "carpetx-mp-cottonmouth"
extends = ["max-morris/carpetx-mp-forks@1.0.0", "rhaas/cottonmouth@2.1.0"]
thornlist = "asterx-additions.th"
enable = ["ExternalLibraries/PAPI"]
"#;

#[test]
fn every_stack_shape_parses() {
    let recipe = Manifest::from_toml(RECIPE).unwrap().stack.unwrap();
    assert!(!recipe.is_flat());
    assert_eq!(recipe.overrides.len(), 3);
    let flat = Manifest::from_toml(FLAT).unwrap().stack.unwrap();
    assert!(flat.is_flat());
    let both = Manifest::from_toml(BOTH).unwrap().stack.unwrap();
    assert_eq!(both.extends.len(), 2);
    assert!(both.thornlist.is_some());
}

fn invalid(text: &str) -> String {
    match Manifest::from_toml(text) {
        Err(Error::Invalid(msg)) => msg,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[test]
fn stack_validation() {
    let empty = "format = 1\n[stack]\nname = \"x\"\n";
    assert!(invalid(empty).contains("needs a thornlist, extends, or both"));

    let unversioned_base = "format = 1\n[stack]\nname = \"x\"\nextends = [\"a/b\"]\n";
    assert!(invalid(unversioned_base).contains("exact release"));
    let ranged_base = "format = 1\n[stack]\nname = \"x\"\nextends = [\"a/b@^1\"]\n";
    assert!(invalid(ranged_base).contains("exact release"));

    let overrides_flat = "format = 1\n[stack]\nname = \"x\"\nthornlist = \"x.th\"\n\
                          [[stack.override]]\nrepo = \"r\"\nref = \"b\"\n";
    assert!(invalid(overrides_flat).contains("extends nothing"));

    let both_ways = "format = 1\n[stack]\nname = \"x\"\nextends = [\"a/b@1.0.0\"]\n\
                     [[stack.override]]\nrepo = \"r\"\nref = \"b\"\npackage = \"a/c@^1\"\n";
    assert!(invalid(both_ways).contains("both a package and url/ref"));

    let clash = "format = 1\n[stack]\nname = \"x\"\nthornlist = \"x.th\"\n\
                 enable = [\"A/B\"]\ndisable = [\"A/B\"]\n";
    assert!(invalid(clash).contains("both enabled and disabled"));
}

#[test]
fn contact_must_be_a_url_or_mailto() {
    use cactup_ecosystem_common::manifest::validate_contact;
    validate_contact("https://trac.einsteintoolkit.org/").unwrap();
    validate_contact("mailto:users@einsteintoolkit.org").unwrap();
    for bad in
        ["http://x.org", "https://", "mailto:nobody", "javascript:alert(1)", "https://a b", "issues"]
    {
        assert!(validate_contact(bad).is_err(), "{bad}");
    }
    let m = PACKAGE.replace(
        "license = \"LGPL-2.1-or-later\"",
        "license = \"LGPL-2.1-or-later\"\ncontact = \"https://trac.einsteintoolkit.org/\"",
    );
    assert_eq!(
        Manifest::from_toml(&m).unwrap().package.unwrap().contact.as_deref(),
        Some("https://trac.einsteintoolkit.org/")
    );
    assert!(
        Manifest::from_toml(
            &PACKAGE.replace("license = \"LGPL-2.1-or-later\"", "contact = \"javascript:x\"")
        )
        .is_err()
    );
}

#[test]
fn manifest_validation() {
    let neither = "format = 1\n";
    assert!(invalid(neither).contains("[package], [stack] or [sim]"));

    let thorn_target = PACKAGE
        .replace("target = \"max-morris/carpetx-mp\"", "target = \"max-morris/carpetx-mp/TestReal4\"");
    assert!(invalid(&thorn_target).contains("names a thorn"));
    let thorn_fork = PACKAGE.replace("target = \"einsteintoolkit/carpetx\"", "target = \"einsteintoolkit/carpetx/CarpetX\"");
    assert!(invalid(&thorn_fork).contains("names a thorn"));

    let no_owners = "format = 1\n[package]\nname = \"x\"\nowners = []\n";
    assert!(invalid(no_owners).contains("owners is empty"));
}

#[test]
fn frozen_stack_requires_every_pin() {
    let text = r#"
format = 1
stack = "max-morris/carpetx-mp-forks@1.0.0"
root = "Cactus"

[thornlist]
filename = "carpetx-mp-forks.th"
text = "!CRL_VERSION = 1.0\n"

[[repo]]
name = "flesh"
url = "https://github.com/max-morris/Cactus.git"
ref = "mixed-precision"
commit = "3f2c9e1a00112233445566778899aabbccddeeff"

[[repo]]
name = "CarpetX"
url = "https://github.com/max-morris/CarpetX.git"
"#;
    let frozen: FrozenStack = toml::from_str(text).unwrap();
    assert!(frozen.validate().unwrap_err().to_string().contains("'CarpetX' is not pinned"));
}

/// A flat stack whose definition pins some repos and lets the rest follow their branches.
#[test]
fn flat_stack_with_partial_pins() {
    let text = r#"
format = 1

[stack]
name = "carpetx"
thornlist = "carpetx.th"

[[stack.pin]]
name = "AsterX"
url = "https://github.com/jaykalinani/AsterX.git"
ref = "main"
commit = "3f2c9e1a00112233445566778899aabbccddeeff"
"#;
    let s = Manifest::from_toml(text).unwrap().stack.unwrap();
    assert!(s.is_flat());
    assert_eq!(s.pins.len(), 1);

    let unpinned = text.replace("commit = \"3f2c9e1a00112233445566778899aabbccddeeff\"\n", "");
    assert!(invalid(&unpinned).contains("needs exactly one of commit, sha256 or tag"));
}

/// `pin-tag` pins every repo that has the tag; a repo's own pin, by tag or commit, wins over it.
#[test]
fn pin_tag_and_pins_by_tag() {
    let text = r#"
format = 1

[stack]
name = "et-stack"
thornlist = "einsteintoolkit.th"
pin-tag = "ET_2026_05_v0"

[[stack.pin]]
name = "CarpetX"
tag = "ET_2026_05_v1"

[[stack.pin]]
name = "Cottonmouth"
commit = "7b3a6b19bb6278d7ae9205bce817042c253a9591"
"#;
    let s = Manifest::from_toml(text).unwrap().stack.unwrap();
    assert_eq!(s.pin_tag.as_deref(), Some("ET_2026_05_v0"));
    assert_eq!(s.pins[0].tag.as_deref(), Some("ET_2026_05_v1"));
    assert_eq!(s.pins[1].commit.as_deref(), Some("7b3a6b19bb6278d7ae9205bce817042c253a9591"));
    let back =
        Manifest::from_toml(&toml::to_string(&Manifest::from_toml(text).unwrap()).unwrap()).unwrap();
    assert_eq!(back.stack.unwrap(), s);

    let both = text.replace(
        "tag = \"ET_2026_05_v1\"",
        "tag = \"ET_2026_05_v1\"\ncommit = \"7b3a6b19bb6278d7ae9205bce817042c253a9591\"",
    );
    assert!(invalid(&both).contains("exactly one of commit, sha256 or tag"));
    let twice = text.replace("name = \"Cottonmouth\"", "name = \"CarpetX\"");
    assert!(invalid(&twice).contains("pinned twice"));
    assert!(
        invalid(&text.replace("pin-tag = \"ET_2026_05_v0\"", "pin-tag = \"ET 2026\""))
            .contains("not a valid tag name")
    );
    assert!(
        invalid(&text.replace("tag = \"ET_2026_05_v1\"", "tag = \"a..b\""))
            .contains("not a valid tag name")
    );
    assert!(
        invalid(&text.replace("7b3a6b19bb6278d7ae9205bce817042c253a9591", "7b3a6b1"))
            .contains("not a full commit id")
    );
}

#[test]
fn tips_are_never_a_base() {
    for tip in ["head", "master"] {
        let text = format!("format = 1\n[stack]\nname = \"x\"\nextends = [\"a/b@{tip}\"]\n");
        assert!(invalid(&text).contains("exact release"));
    }
}

#[test]
fn manifest_repo_file_names_fix_the_kind() {
    use cactup_ecosystem_common::manifest::{PACKAGE_FILE, STACK_FILE};
    // A manifest repo holding only the curated CarpetX stack.
    let stack = "format = 1\n[stack]\nname = \"carpetx\"\nowners = [\"cactarium\"]\nthornlist = \"carpetx.th\"\n";
    Manifest::from_repo_file(STACK_FILE, stack).unwrap();
    assert!(Manifest::from_repo_file(PACKAGE_FILE, stack).is_err());
    assert!(Manifest::from_repo_file(STACK_FILE, PACKAGE).is_err());
    Manifest::from_repo_file(PACKAGE_FILE, PACKAGE).unwrap();

    let ownerless = stack.replace("owners = [\"cactarium\"]\n", "");
    let err = Manifest::from_repo_file(STACK_FILE, &ownerless).unwrap_err();
    assert!(err.to_string().contains("needs owners"));
    assert!(Manifest::from_repo_file("stack.toml", stack).is_err());
}

/// `tag-pattern` and `since`: release tags that aren't vX.Y.Z, in a package or a stack.
#[test]
fn tag_pattern_and_since() {
    use cactup_ecosystem_common::version::TagPattern;
    let p: TagPattern = "ET_{major}_{minor}_v{patch}".parse().unwrap();
    assert_eq!(p.version("ET_2026_05_v0").unwrap().version, semver::Version::new(2026, 5, 0));
    assert_eq!(p.version("refs/tags/ET_2025_11_v2").unwrap().version, semver::Version::new(2025, 11, 2));
    assert!(p.version("ET_2026_05").is_none() && p.version("v1.2.3").is_none() && p.version("xET_2026_05_v0").is_none());
    for bad in ["ET_{minor}_{patch}", "ET_{major}_v{patch}", "ET_{major}_{major}_v{patch}", "{major}.{minor}.{build}", "a b{major}{minor}{patch}", "{major"] {
        assert!(bad.parse::<TagPattern>().is_err(), "{bad}");
    }
    // A pattern may leave out {patch}, or {minor} and {patch}: they read as 0 (AsterX tags v1.0).
    let two: TagPattern = "v{major}.{minor}".parse().unwrap();
    assert_eq!(two.version("v1.0").unwrap().version, semver::Version::new(1, 0, 0));
    assert_eq!(two.version("v1.12").unwrap().version, semver::Version::new(1, 12, 0));
    assert!(two.version("v0.1.0").is_none(), "only tags the pattern matches are versions");
    let one: TagPattern = "release-{major}".parse().unwrap();
    assert_eq!(one.version("release-7").unwrap().version, semver::Version::new(7, 0, 0));

    let pkg = r#"
format = 1

[package]
name = "hdf5"
owners = ["et"]
arrangement = "ExternalLibraries"
thorns = ["HDF5"]
tag-pattern = "ET_{major}_{minor}_v{patch}"
since = "2026.5.0"
"#;
    let m = Manifest::from_toml(pkg).unwrap();
    let p = m.package.as_ref().unwrap();
    assert_eq!(p.tag_pattern.as_deref(), Some("ET_{major}_{minor}_v{patch}"));
    assert_eq!(p.since, Some(semver::Version::new(2026, 5, 0)));
    assert_eq!(Manifest::from_toml(&toml::to_string(&m).unwrap()).unwrap(), m);
    assert!(invalid(&pkg.replace("{patch}", "{build}")).contains("tag-pattern"));
    assert!(Manifest::from_toml(&pkg.replace("2026.5.0", "2026.5")).is_err());

    let stack = r#"
format = 1

[stack]
name = "et-stack"
thornlist = "einsteintoolkit.th"
tag-pattern = "ET_{major}_{minor}_v{patch}"
"#;
    let s = Manifest::from_toml(stack).unwrap().stack.unwrap();
    assert_eq!(s.tag_pattern.as_deref(), Some("ET_{major}_{minor}_v{patch}"));
}

/// cactarium-sim.toml: a sim published from its repo.
#[test]
fn sim_manifest() {
    let text = r#"
format = 1

[sim]
name = "bhns-gw230529-dense"
owners = ["max-morris"]
file = "bhns-gw230529-dense.sim"
tag-pattern = "run-{major}.{minor}.{patch}"
"#;
    let m = Manifest::from_repo_file("cactarium-sim.toml", text).unwrap();
    let s = m.sim.as_ref().unwrap();
    assert_eq!(s.file, "bhns-gw230529-dense.sim");
    assert_eq!(Manifest::from_toml(&toml::to_string(&m).unwrap()).unwrap(), m);
    assert!(Manifest::from_repo_file("cactarium.toml", text).is_err());
    assert!(Manifest::from_repo_file("cactarium-stack.toml", text).is_err());
    for bad in ["../x.sim", "/x.sim", "a//b.sim", "./x.sim", ""] {
        assert!(invalid(&text.replace("bhns-gw230529-dense.sim\"", &format!("{bad}\""))).contains("relative path"), "{bad}");
    }
    assert!(invalid(&text.replace("owners = [\"max-morris\"]", "owners = []")).contains("needs owners"));
    let both = format!("{text}\n[package]\nname = \"x\"\nowners = [\"y\"]\n");
    assert!(invalid(&both).contains("one of [package], [stack] or [sim]"));
}

/// `owner/pkg#<sha>` names only a commit (full or abbreviated); `@<hex>` is a commit only as a
/// fallback, after every other reading.
#[test]
fn slugs_name_commits() {
    use cactup_ecosystem_common::slug::Slug;
    let sha = "7b3a6b19bb6278d7ae9205bce817042c253a9591";
    let full: Slug = format!("et/cottonmouth#{sha}").parse().unwrap();
    assert_eq!(full.sha(), Some(sha));
    assert!(full.req().unwrap_err().to_string().contains("names a commit"));
    assert_eq!(full.to_string(), format!("et/cottonmouth#{sha}"));
    let short: Slug = "et/cottonmouth/CottonmouthZ4c4m#7B3A6B1".parse().unwrap();
    assert_eq!((short.sha(), short.thorn.as_deref()), (Some("7b3a6b1"), Some("CottonmouthZ4c4m")));
    for bad in ["et/cottonmouth#7b3a6b", "et/cottonmouth#xyz1234", "et/cottonmouth@1.0#7b3a6b1", "et/cottonmouth#"] {
        assert!(bad.parse::<Slug>().is_err(), "{bad}");
    }
    // `@` prefers every other reading; hex is only a fallback.
    let at: Slug = "et/cottonmouth@7b3a6b1".parse().unwrap();
    assert_eq!(at.commit_fallback(), Some("7b3a6b1"));
    assert!(at.req().is_err(), "7b3a6b1 is no version or track");
    assert_eq!("et/cottonmouth@deadbeef".parse::<Slug>().unwrap().commit_fallback(), Some("deadbeef"));
    assert!("et/cottonmouth@deadbeef".parse::<Slug>().unwrap().req().is_ok(), "reads as a track first");
    for not in ["et/cottonmouth@1.2.3", "et/cottonmouth@head", "et/cottonmouth@beef12"] {
        assert_eq!(not.parse::<Slug>().unwrap().commit_fallback(), None, "{not}");
    }
}

#[test]
fn fork_of_takes_a_range_or_a_commit_and_stands_in_where_they_overlap() {
    use cactup_ecosystem_common::manifest::ForkOf;
    use cactup_ecosystem_common::version::TrackedVersion;
    let fork = |t: &str| ForkOf { target: t.parse().unwrap(), note: None };
    let slug = |s: &str| s.parse::<cactup_ecosystem_common::slug::Slug>().unwrap();
    let published: Vec<(TrackedVersion, Option<String>)> = [
        ("2026.5.0", "aaaaaaa1111"),
        ("2026.5.1", "bbbbbbb2222"),
        ("2026.11.0", "ccccccc3333"),
    ]
    .iter()
    .map(|(v, c)| (v.parse().unwrap(), Some((*c).to_owned())))
    .collect();

    // A fork of the 2026.5 series.
    let f = fork("et/carpet@~2026.5");
    assert!(f.stands_in_for(&slug("et/carpet@^2026.5"), &published), "overlap at 2026.5.x");
    assert!(f.stands_in_for(&slug("et/carpet@2026.5.1"), &published), "a sim's exact pin inside the range");
    assert!(!f.stands_in_for(&slug("et/carpet@2026.11.0"), &published), "outside the fork's range");
    assert!(!f.stands_in_for(&slug("et/carpet@>=2027"), &published), "no published version in both");
    assert!(!f.stands_in_for(&slug("et/carpetx@^2026.5"), &published), "another package");
    assert!(!f.stands_in_for(&slug("et/carpet#aaaaaaa"), &published), "a commit dependency needs a fork of that commit");

    // A bare fork-of covers every version on the default track.
    assert!(fork("et/carpet").stands_in_for(&slug("et/carpet@2026.11.0"), &published));

    // A fork of one commit stands in for that commit's version and that commit.
    let c = fork("et/carpet#bbbbbbb");
    assert!(c.stands_in_for(&slug("et/carpet@^2026.5"), &published));
    assert!(c.stands_in_for(&slug("et/carpet#bbbbbbb2222"), &published));
    assert!(!c.stands_in_for(&slug("et/carpet@2026.5.0"), &published));

    // In a manifest: ranges and commits are accepted, thorns are not.
    let m = |target: &str| {
        format!(
            "format = 1\n[package]\nname = \"carpet\"\nowners = [\"steve\"]\n\n[[fork-of]]\ntarget = \"{target}\"\n"
        )
    };
    for ok in ["et/carpet", "et/carpet@~2026.5", "et/carpet@>=2026.5.0 <2026.11", "et/carpet#abc1234"] {
        Manifest::from_repo_file("cactarium.toml", &m(ok)).unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    let err = Manifest::from_repo_file("cactarium.toml", &m("et/carpet/CarpetLib")).unwrap_err();
    assert!(err.to_string().contains("names a thorn"), "{err}");
}

#[test]
fn a_package_root_is_a_folder_inside_the_repo() {
    let m = |root: &str| {
        format!("format = 1\n[package]\nname = \"kranc\"\nowners = [\"et\"]\nroot = \"{root}\"\n")
    };
    let ok = Manifest::from_repo_file("cactarium.toml", &m("Auxiliary/Cactus")).unwrap();
    assert_eq!(ok.package.unwrap().root.as_deref(), Some("Auxiliary/Cactus"));
    for bad in ["", "../elsewhere", "/abs", "a/../../b", "./x"] {
        let err = Manifest::from_repo_file("cactarium.toml", &m(bad)).unwrap_err();
        assert!(err.to_string().contains("folder inside the repo"), "{bad}: {err}");
    }
}

#[test]
fn a_flesh_package_has_no_thorns() {
    let m = |extra: &str| format!("format = 1\n[package]\nname = \"cactus\"\nowners = [\"et\"]\nflesh = true\n{extra}");
    let p = Manifest::from_repo_file("cactarium.toml", &m("")).unwrap().package.unwrap();
    assert!(p.flesh);
    for bad in ["thorns = [\"x\"]\n", "arrangement = \"A\"\n", "root = \"src\"\n"] {
        let err = Manifest::from_repo_file("cactarium.toml", &m(bad)).unwrap_err();
        assert!(err.to_string().contains("a flesh has no thorns"), "{bad}: {err}");
    }
    // A mixed-precision flesh forks et/cactus.
    let fork = m("\n[[fork-of]]\ntarget = \"et/cactus@~2026.5\"\n").replace("name = \"cactus\"", "name = \"cactus-mp\"");
    assert_eq!(Manifest::from_repo_file("cactarium.toml", &fork).unwrap().fork_of.len(), 1);
}
