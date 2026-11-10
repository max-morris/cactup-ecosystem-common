use cactup_ecosystem_common::Error;
use cactup_ecosystem_common::sim::{
    ArchiveKind, Fixup, Input, ParfileKind, Parfile, RepoPin, SimFile, SimMeta, ThornlistText, Unspecified,
};

const SHA: &str = "3f2c9e1a00112233445566778899aabbccddeeff";

fn sample(thornlist: &str, par: &str) -> SimFile {
    SimFile {
        format: 1,
        sim: SimMeta {
            name: "bhns-gw230529-dense".into(),
            summary: Some("BHNS merger with dense 2D output".into()),
            description: None,
            authors: vec!["Max Morris".into()],
            license: None,
            contact: None,
            cactarium: None,
            root: "Cactus".into(),
        },
        thornlist: ThornlistText { filename: "bhns.th".into(), text: thornlist.into(), untracked: vec![] },
        parfile: Parfile {
            filename: "bhns.par".into(),
            kind: ParfileKind::Par,
            vars: vec![],
            text: par.into(),
        },
        inputs: vec![],
        repos: vec![RepoPin {
            name: "carpetx".into(),
            url: "https://github.com/max-morris/CarpetX.git".into(),
            git_ref: Some("mixed-precision".into()),
            commit: Some(SHA.into()),
            sha256: None,
            cactarium: Some("max-morris/carpetx@mixed-precision:1.4.0".parse().unwrap()),
            dirty: false,
        }],
        run: None,
        provenance: None,
    }
}

/// Text that stresses TOML string encoding: literal-string delimiters, basic-string delimiters,
/// backslashes, CRLF, lone CR, tabs, trailing spaces, non-ASCII, control chars, no final newline.
const NASTY: &[&str] = &[
    "ActiveThorns = \"CarpetX\"\n",
    "a = '''x'''\nb = \"\"\"y\"\"\"\n",
    "path = \"C:\\\\x\\n\" # \\ backslash\n",
    "line1\r\nline2\r\n",
    "lone\rcr\n",
    "\ttabbed  \ntrailing   \n",
    "Ψ₄ extracted at r = 500 M — “quotes”\n",
    "bell\u{7}and\u{1b}esc\n",
    "no final newline",
    "",
    "\n\n\nleading newlines",
];

#[test]
fn verbatim_text_round_trips_byte_for_byte() {
    for (i, t) in NASTY.iter().enumerate() {
        let sim = sample(t, t);
        let out = sim.to_toml().unwrap();
        let back = SimFile::from_toml(&out).unwrap_or_else(|e| panic!("case {i}: {e}\n{out}"));
        assert_eq!(back.thornlist.text.as_bytes(), t.as_bytes(), "thornlist case {i}");
        assert_eq!(back.parfile.text.as_bytes(), t.as_bytes(), "parfile case {i}");
        assert_eq!(back, sim, "case {i}");
    }
}

#[test]
fn hand_written_file_parses() {
    let text = format!(
        r#"
format = 1

[sim]
name = "qc0-bbh"
root = "Cactus"

[thornlist]
filename = "qc0.th"
text = '''
!CRL_VERSION = 1.0
!DEFINE ROOT = Cactus
'''

[parfile]
filename = "qc0.par"
kind = "par"
vars = ["WALLTIME_HOURS"]
text = '''
ActiveThorns = "Carpet"
'''

[[input]]
knob = "kadath-initial-data"
description = "FUKA initial data"

[[repo]]
name = "flesh"
url = "https://bitbucket.org/cactuscode/cactus.git"
ref = "ET_2026_05"
commit = "{SHA}"

[run]
tasks = 256
cpus-per-task = 8
walltime = "48:00:00"

[provenance]
machine = "qbd"
exported-at = 2026-11-05T22:00:00Z
"#
    );
    let sim = SimFile::from_toml(&text).unwrap();
    assert!(sim.is_fully_specified());
    assert_eq!(sim.inputs[0].knob, "kadath-initial-data");
    assert_eq!(sim.run.unwrap().cpus_per_task, Some(8));
}

#[test]
fn unknown_keys_are_rejected() {
    let mut out = sample("x\n", "y\n").to_toml().unwrap();
    out = out.replacen("[sim]\n", "[sim]\nflavour = \"intel\"\n", 1);
    assert!(matches!(SimFile::from_toml(&out), Err(Error::Toml { .. })));
}

#[test]
fn newer_format_asks_for_an_upgrade() {
    let out = sample("x\n", "y\n").to_toml().unwrap().replacen("format = 1", "format = 3", 1);
    let err = SimFile::from_toml(&out).unwrap_err();
    assert!(matches!(err, Error::NewerFormat { found: 3, supported: 2, .. }), "{err}");
    assert!(err.to_string().contains("upgrade"));
}

/// The pretty-bhns case: a gallery tarball unpacked, the knob pointing at its `.info` file, and
/// one placeholder path in it rewritten against the unpack directory.
fn archive_input() -> Input {
    Input {
        knob: "kadath-initial-data".into(),
        description: Some("FUKA initial data".into()),
        url: Some("https://einsteintoolkit.org/gallery/bhns/bhns_gw230529_ID.tar.gz".into()),
        sha256: Some("ab".repeat(32)),
        unpack: true,
        path: Some("bhns_gw230529_ID/X.info".into()),
        fixups: vec![Fixup {
            file: "bhns_gw230529_ID/X.info".into(),
            replace: "/path/to/gam2.polytrope".into(),
            with: "{dir}/bhns_gw230529_ID/gam2.polytrope".into(),
        }],
    }
}

/// A file is written as the oldest format that holds it: 1 without archive inputs (readable by
/// every older reader), 2 with one; both read back, and a format-1 file claiming one is refused.
#[test]
fn archive_inputs_need_format_2_and_nothing_else_does() {
    let plain = sample("x\n", "y\n");
    assert!(plain.to_toml().unwrap().starts_with("format = 1\n"));
    let mut sim = sample("x\n", "y\n");
    sim.format = 2;
    assert!(sim.to_toml().unwrap().starts_with("format = 1\n"), "nothing needs 2, so 1 is written");

    sim.inputs.push(archive_input());
    let text = sim.to_toml().unwrap();
    assert!(text.starts_with("format = 2\n"), "{text}");
    assert!(text.contains("[[input.fixup]]"), "{text}");
    let back = SimFile::from_toml(&text).unwrap();
    assert_eq!(back.inputs, sim.inputs);
    assert_eq!(back.format, 2);

    let claims_1 = text.replacen("format = 2", "format = 1", 1);
    let err = SimFile::from_toml(&claims_1).unwrap_err().to_string();
    assert!(err.contains("need format 2"), "{err}");
}

#[test]
fn archive_inputs_are_validated() {
    let check = |edit: &dyn Fn(&mut Input)| {
        let mut sim = sample("x\n", "y\n");
        let mut input = archive_input();
        edit(&mut input);
        sim.inputs.push(input);
        sim.format = 2;
        sim.validate().map_err(|e| e.to_string())
    };
    check(&|_| {}).unwrap();
    // Without path the knob gets the unpack directory itself.
    check(&|i| i.path = None).unwrap();
    for (what, edit, expect) in [
        ("no sha256", Box::new(|i: &mut Input| i.sha256 = None) as Box<dyn Fn(&mut Input)>, "sha256"),
        ("no url", Box::new(|i: &mut Input| i.url = None), "url"),
        ("not an archive", Box::new(|i: &mut Input| i.url = Some("https://x/data.h5".into())), "not an archive"),
        ("absolute path", Box::new(|i: &mut Input| i.path = Some("/etc/passwd".into())), "relative path"),
        ("dotdot path", Box::new(|i: &mut Input| i.path = Some("a/../../b".into())), "relative path"),
        ("backslash", Box::new(|i: &mut Input| i.fixups[0].file = "a\\b".into()), "relative path"),
        ("empty replace", Box::new(|i: &mut Input| i.fixups[0].replace.clear()), "replaces nothing"),
        ("other placeholder", Box::new(|i: &mut Input| i.fixups[0].with = "{home}/x".into()), "only placeholder"),
        ("path without unpack", Box::new(|i: &mut Input| i.unpack = false), "need unpack"),
    ] {
        let err = check(&*edit).expect_err(what);
        assert!(err.contains(expect), "{what}: {err}");
    }
    for url in ["https://x/a.tar", "https://x/a.TGZ", "https://x/a.tar.xz?dl=1", "https://x/a.zip#frag"] {
        assert!(ArchiveKind::of_url(url).is_some(), "{url}");
    }
    assert_eq!(archive_input().fixups[0].replacement("/work/in"), "/work/in/bhns_gw230529_ID/gam2.polytrope");
}

#[test]
fn missing_format_is_reported() {
    let out = sample("x\n", "y\n").to_toml().unwrap().replacen("format = 1\n", "", 1);
    assert!(matches!(SimFile::from_toml(&out), Err(Error::MissingFormat { .. })));
}

#[test]
fn fully_specified_rules() {
    let mut sim = sample("x\n", "y\n");
    assert!(sim.is_fully_specified());

    sim.repos[0].commit = None;
    sim.repos[0].dirty = true;
    sim.thornlist.untracked.push("LocalThorns/Boost".into());
    let problems = sim.unspecified();
    assert_eq!(
        problems,
        vec![
            Unspecified::Unpinned { repo: "carpetx".into() },
            Unspecified::Dirty { repo: "carpetx".into() },
            Unspecified::Untracked { thorn: "LocalThorns/Boost".into() },
        ]
    );
    // Unpinned files are still valid .sim files; they just can't be published.
    SimFile::from_toml(&sim.to_toml().unwrap()).unwrap();
}

#[test]
fn structural_errors() {
    let mut sim = sample("x\n", "y\n");
    sim.repos[0].commit = Some("3f2c9e1".into());
    assert!(sim.to_toml().is_err(), "abbreviated commit");

    let mut sim = sample("x\n", "y\n");
    sim.repos.push(sim.repos[0].clone());
    assert!(sim.to_toml().is_err(), "duplicate repo");

    let mut sim = sample("x\n", "y\n");
    sim.parfile.kind = ParfileKind::Py;
    assert!(sim.to_toml().is_err(), "kind/extension mismatch");

    let mut sim = sample("x\n", "y\n");
    sim.thornlist.filename = "../etc/passwd".into();
    assert!(sim.to_toml().is_err(), "path in filename");
}

/// Round-trips a real thornlist and parfile, verbatim, when CACTUP_TEST_THORNLIST and
/// CACTUP_TEST_PARFILE name them.
#[test]
fn real_files_round_trip() {
    let (Some(th), Some(par)) = (std::env::var_os("CACTUP_TEST_THORNLIST"), std::env::var_os("CACTUP_TEST_PARFILE")) else {
        return;
    };
    let (Ok(th), Ok(par)) = (std::fs::read_to_string(th), std::fs::read_to_string(par)) else { return };
    let sim = sample(&th, &par);
    let back = SimFile::from_toml(&sim.to_toml().unwrap()).unwrap();
    assert_eq!(back.thornlist.text, th);
    assert_eq!(back.parfile.text, par);
}
