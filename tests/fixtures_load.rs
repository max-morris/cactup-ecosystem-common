//! Every shipped shape of each interchange format keeps loading. `tests/fixtures/` holds one file
//! per generation, written by this crate's own writers when it shipped; a fixture is never edited
//! or deleted once committed, only added to when a format gains a generation.

use cactup_ecosystem_common::manifest::{Manifest, SIM_FILE};
use cactup_ecosystem_common::sim::SimFile;
use cactup_ecosystem_common::stack::FrozenStack;
use cactup_ecosystem_common::{MANIFEST_FORMAT, SIM_FORMAT, STACK_FORMAT};
use std::path::PathBuf;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every `.sim` fixture loads, reports exactly the format its file declares (which pins
/// `required_format`: a plain input must not force format 2), and is written back byte for byte:
/// the writer emits the oldest generation that holds the content, which is the one each fixture
/// came in, and keeps the field order the old writers used.
#[test]
fn every_sim_fixture_loads_and_round_trips() {
    let all = [
        ("sim/v1.sim", 1),
        ("sim/v1-full.sim", 1),
        ("sim/v2.sim", 2),
        ("sim/v2-full.sim", 2),
        ("sim/v3.sim", 3),
        ("sim/v3-full.sim", 3),
    ];
    for (name, format) in all {
        let text = fixture(name);
        assert!(text.starts_with(&format!("format = {format}\n")), "{name}");
        let sim = SimFile::from_toml(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(sim.format, format, "{name}");
        assert_eq!(sim.required_format(), format, "{name} holds exactly what its generation adds");
        assert_eq!(sim.to_toml().unwrap(), text, "{name} is not written back byte for byte");
    }
    assert_eq!(SIM_FORMAT, 3, "a new generation needs its fixtures: add sim/v4.sim and v4-full.sim and list them above");
}

/// The `-full` fixtures really hold every optional table and key of their generation.
#[test]
fn full_sim_fixtures_hold_everything() {
    let v1 = SimFile::from_toml(&fixture("sim/v1-full.sim")).unwrap();
    assert!(v1.run.is_some() && v1.provenance.is_some());
    assert!(v1.sim.cactarium.is_some() && v1.sim.license.is_some() && v1.sim.contact.is_some());
    assert!(!v1.thornlist.untracked.is_empty() && !v1.parfile.vars.is_empty());
    assert!(v1.repos.iter().any(|r| r.dirty) && v1.repos.iter().any(|r| r.sha256.is_some()));
    assert!(v1.repos.iter().any(|r| r.cactarium.is_some()));
    assert!(v1.inputs.iter().any(|i| i.url.is_some() && i.sha256.is_some() && !i.unpack));
    assert!(v1.inputs.iter().any(|i| i.url.is_none()), "an input asked for on import");
    let v2 = SimFile::from_toml(&fixture("sim/v2-full.sim")).unwrap();
    assert!(v2.inputs.iter().any(|i| i.unpack && i.path.is_some() && i.fixups.len() == 2));
    let v3 = SimFile::from_toml(&fixture("sim/v3-full.sim")).unwrap();
    assert!(v3.inputs.iter().any(|i| i.repo_file.is_some() && !i.unpack));
    assert!(v3.inputs.iter().any(|i| i.repo_file.is_some() && i.unpack));
}

#[test]
fn every_manifest_generation_loads() {
    for name in ["manifest/v1.toml", "manifest/v1-stack.toml", "manifest/v1-sim.toml"] {
        let text = fixture(name);
        let m = Manifest::from_toml(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(m.format, 1, "{name}");
        assert_eq!(Manifest::from_toml(&m.to_toml().unwrap()).unwrap(), m, "{name}");
    }
    let shape = |name: &str| {
        let m = Manifest::from_toml(&fixture(name)).unwrap();
        (m.package.is_some(), m.stack.is_some(), m.sim.is_some())
    };
    assert_eq!(shape("manifest/v1.toml"), (true, false, false));
    assert_eq!(shape("manifest/v1-stack.toml"), (false, true, false));
    assert_eq!(shape("manifest/v1-sim.toml"), (false, false, true));
    // v1-sim.toml is `cactarium-sim.toml` as the writer emits it; it is written back byte for byte.
    let text = fixture("manifest/v1-sim.toml");
    assert_eq!(Manifest::from_toml(&text).unwrap().to_toml().unwrap(), text);
    let m = Manifest::from_repo_file(SIM_FILE, &text).unwrap();
    let sim = m.sim.unwrap();
    assert!(sim.description.is_some() && sim.contact.is_some() && sim.tag_pattern.is_some() && sim.since.is_some());
    assert_eq!(MANIFEST_FORMAT, 1, "a new generation needs its fixture: add manifest/v2.toml and list it above");
}

/// A newer manifest is refused with the upgrade message, an older one is not "unknown".
#[test]
fn manifest_generations_are_read_back_to_1_and_no_further_than_current() {
    let newer = fixture("manifest/v1.toml").replacen("format = 1", &format!("format = {}", MANIFEST_FORMAT + 1), 1);
    assert!(Manifest::from_toml(&newer).unwrap_err().to_string().contains("upgrade"));
    let zero = fixture("manifest/v1.toml").replacen("format = 1", "format = 0", 1);
    assert!(Manifest::from_toml(&zero).is_err());
}

#[test]
fn every_stack_generation_loads() {
    let text = fixture("stack/v1.toml");
    let stack: FrozenStack = toml::from_str(&text).unwrap();
    stack.validate().unwrap();
    assert_eq!(stack.format, 1);
    assert!(stack.format <= STACK_FORMAT);
    assert_eq!(STACK_FORMAT, 1, "a new generation needs its fixture: add stack/v2.toml and load it here");
}

/// What the Cactarium API sends (`GET .../releases/{version}`) is JSON: a release with a recipe,
/// a git pin and a sha256 pin. FrozenStack has not changed since it shipped (21313c3), so the
/// fixture is the current writer's output.
#[test]
fn the_api_frozen_stack_loads() {
    let text = fixture("stack/v1-full.json");
    let stack: FrozenStack = serde_json::from_str(&text).unwrap();
    stack.validate().unwrap();
    assert_eq!(stack.format, 1);
    assert!(stack.recipe.is_some());
    assert!(stack.repos.iter().any(|r| r.commit.is_some() && r.cactarium.is_some()));
    assert!(stack.repos.iter().any(|r| r.sha256.is_some()));
    assert_eq!(serde_json::to_string_pretty(&stack).unwrap() + "\n", text);
}
