//! Code shared across the Cactup ecosystem: the `cactup` CLI on users' machines and the
//! Cactarium registry.
//!
//! - [`sim`]: `.sim` files, which hold a parfile together with the thornlist it ran on, both kept
//!   verbatim, plus per-repo pins.
//! - [`manifest`]: `cactarium.toml` (a package) and `cactarium-stack.toml` (a stack), committed
//!   in a repo and read at each tag.
//! - [`stack`]: stack definitions (flat thornlist, recipe, or both) and frozen releases.
//! - [`api`]: the Cactarium HTTP API's JSON shapes.
//! - [`ccl`]: dependency facts from a thorn's interface.ccl and configuration.ccl, the hard
//!   requirements they make ([`ccl::Need`]) and which thorns meet them.
//! - [`crl`]: the CRL 1.0 thornlist parser (shared with cactup); [`crl_emit`] writes components back.
//! - [`par`]: which parts of a parfile are comments, and which `@NAME@` / `@KNOB(...)@` /
//!   `@ENV(...)@` tokens it uses, lexed exactly as cactup substitutes them.
//! - [`url`]: repo URLs compared as one repo whatever their spelling, the same way everywhere.
//! - [`name`], [`version`], [`slug`]: owner/name identifiers, track-namespaced semver, and the
//!   `owner/name[/Thorn|/sim][@...]` references users type.
//!
//! Every format carries a top-level `format` generation. Readers are strict: unknown keys are an
//! error, and a generation newer than this crate knows is rejected with an upgrade message (the
//! same discipline as cactup's MDB generations).

pub mod api;
pub mod ccl;
pub mod crl;
pub mod crl_emit;
pub mod error;
pub mod manifest;
pub mod name;
pub mod par;
pub mod sim;
pub mod slug;
pub mod stack;
pub mod url;
pub mod version;

mod generation;

pub use error::{Error, Result};

/// The newest `.sim` format generation this crate reads and writes. It reads every generation
/// back to 1, and writes the oldest one that can hold a file's content
/// ([`sim::SimFile::required_format`]), so a file that needs nothing new stays readable by
/// older readers. Generation 2 added archive inputs (`unpack`, `path`, `[[input.fixup]]`);
/// generation 3, inputs that are a file of the sim's own repository (`repo-file`); generation 4,
/// a `[sim] cactarium` stamp naming one sim of a multi-sim entry (`owner/name/sim@X.Y.Z`), which a
/// reader from before it would take for a thorn, and a `[[repo]]` pin stamped with a commit no
/// version froze (`cactarium = "owner/pkg#<sha>"`), which a reader from before commit references
/// would refuse as malformed instead of asking to be upgraded.
pub const SIM_FORMAT: u32 = 4;
/// The newest manifest format generation (`cactarium.toml`, `cactarium-stack.toml`,
/// `cactarium-sim.toml`) this crate reads. It reads every generation back to 1; a manifest must
/// declare at least the generation its content needs ([`manifest::Manifest::required_format`]).
/// Generation 2 added `[sim.files]`: several sims released together from one repo.
pub const MANIFEST_FORMAT: u32 = 2;
/// The frozen-stack format generation this crate reads: the `format` of a stack release
/// ([`stack::FrozenStack`]) the Cactarium serves. A client checks a response's `format` against
/// this before the typed parse, so a release from a newer server says "upgrade" rather than
/// failing on a key it does not know.
pub const STACK_FORMAT: u32 = 1;
