//! Every shipped shape of each interchange format keeps loading. `tests/fixtures/` holds one file
//! per generation, written by this crate's own writers when it shipped; a fixture is never edited
//! or deleted once committed, only added to when a format gains a generation.

use cactup_ecosystem_common::manifest::Manifest;
use cactup_ecosystem_common::sim::SimFile;
use cactup_ecosystem_common::stack::FrozenStack;
use cactup_ecosystem_common::{MANIFEST_FORMAT, SIM_FORMAT, STACK_FORMAT};
use std::path::PathBuf;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Each `.sim` generation loads, reports its own `format`, and is written back as the oldest
/// generation that holds its content (which is the one it came in).
#[test]
fn every_sim_generation_loads() {
    for (name, format) in [("sim/v1.sim", 1), ("sim/v2.sim", 2), ("sim/v3.sim", 3)] {
        let text = fixture(name);
        let sim = SimFile::from_toml(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(sim.format, format, "{name}");
        assert_eq!(sim.required_format(), format, "{name} holds exactly what its generation adds");
        let back = sim.to_toml().unwrap();
        assert!(back.starts_with(&format!("format = {format}\n")), "{name}: {back}");
        assert_eq!(SimFile::from_toml(&back).unwrap(), sim, "{name}");
    }
    assert_eq!(SIM_FORMAT, 3, "a new generation needs its fixture: add sim/v4.sim and list it above");
}

#[test]
fn every_manifest_generation_loads() {
    for (name, package) in [("manifest/v1.toml", true), ("manifest/v1-stack.toml", false)] {
        let m = Manifest::from_toml(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(m.format, 1, "{name}");
        assert_eq!(m.package.is_some(), package, "{name}");
        assert_eq!(Manifest::from_toml(&m.to_toml().unwrap()).unwrap(), m, "{name}");
    }
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
