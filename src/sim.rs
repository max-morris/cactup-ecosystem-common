//! `.sim` files: a parfile plus the configuration it ran on.
//!
//! The thornlist (CRL) and the parfile are stored **verbatim**: this module never interprets or
//! rewrites them, and a read/write round trip preserves them byte for byte. The structure around
//! them is the per-repo pin table, declared inputs, run hints and provenance.

use std::collections::BTreeSet;
use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::slug::Slug;
use crate::{Error, Result, SIM_FORMAT, generation};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SimFile {
    pub format: u32,
    pub sim: SimMeta,
    pub thornlist: ThornlistText,
    pub parfile: Parfile,
    #[serde(default, rename = "input", skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<Input>,
    #[serde(default, rename = "repo", skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<RepoPin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunHints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SimMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Markdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// An SPDX license expression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Where to go for help with this simulation: an `https://` URL or a `mailto:` address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    /// Set by the registry when the file is published: `owner/name@X.Y.Z` (`@track:X.Y.Z` on a
    /// track), or `owner/name/sim@X.Y.Z` for one sim of a multi-sim entry (`[sim.files]`), which
    /// needs format 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cactarium: Option<Slug>,
    /// The thornlist's `!DEFINE ROOT`, the source tree's directory under the install.
    pub root: String,
}

/// A CRL thornlist kept verbatim. Shared with frozen stacks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ThornlistText {
    pub filename: String,
    /// The CRL exactly as written, with any `!INCLUDE`s spliced in.
    pub text: String,
    /// Thorns in the build that no repo backs (no commit can pin them). Present only in files
    /// exported with an override; a fully specified file has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub untracked: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParfileKind {
    /// A Cactus parameter file.
    Par,
    /// A Python script that prints a parameter file.
    Py,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Parfile {
    pub filename: String,
    pub kind: ParfileKind,
    /// The `@VAR@` tokens the parfile uses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vars: Vec<String>,
    /// The master copy, verbatim and un-substituted.
    pub text: String,
}

/// Something the parfile needs from outside the source tree, reached through a cactup knob
/// (`@KNOB(name)@`). On import, cactup asks for a value or fetches `url`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Input {
    pub knob: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Of the file at `url` (the archive itself, when `unpack`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The download is an archive ([`ArchiveKind`], by the URL's extension) to unpack; the knob
    /// gets `path` inside the unpacked tree, else the tree itself. Needs `url` and `sha256`: an
    /// archive is never unpacked unverified. Format 2.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unpack: bool,
    /// The file (or directory) in the unpacked tree the knob points at, relative to its root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Text edits applied once, in order, after unpacking.
    #[serde(default, rename = "fixup", skip_serializing_if = "Vec::is_empty")]
    pub fixups: Vec<Fixup>,
    /// A file in the sim's own repository, at the version's commit, instead of `url` (the commit
    /// pins its content, so there is no `sha256`). Only a sim published from its repository can
    /// have one (`cactarium-sim.toml`). Unpacked like a download when `unpack`. Format 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_file: Option<String>,
}

/// One edit to a file of an unpacked archive: every occurrence of the literal `replace` becomes
/// `with`, in which `{dir}` stands for the absolute directory the archive was unpacked into (the
/// only placeholder). `replace` must occur at least once; only the archive can tell, so that is
/// checked when the edit is applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Fixup {
    /// Relative to the unpacked tree's root.
    pub file: String,
    pub replace: String,
    pub with: String,
}

impl Fixup {
    /// `with`, its `{dir}` resolved against `dir`.
    pub fn replacement(&self, dir: &str) -> String {
        self.with.replace("{dir}", dir)
    }
}

/// The archive formats an `unpack` input may use, told apart by the URL's extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveKind {
    Tar,
    TarGz,
    TarXz,
    Zip,
}

impl ArchiveKind {
    /// The kind `url`'s path ends in (a query or fragment is ignored), if any.
    pub fn of_url(url: &str) -> Option<ArchiveKind> {
        let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
        if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
            Some(ArchiveKind::TarGz)
        } else if path.ends_with(".tar.xz") || path.ends_with(".txz") {
            Some(ArchiveKind::TarXz)
        } else if path.ends_with(".tar") {
            Some(ArchiveKind::Tar)
        } else if path.ends_with(".zip") {
            Some(ArchiveKind::Zip)
        } else {
            None
        }
    }
}

/// A path inside an unpacked archive: relative, `/`-separated, with no empty, `.` or `..`
/// component and no backslash, so it can never leave the tree.
pub fn validate_member_path(what: &str, p: &str) -> Result<()> {
    if p.is_empty() || p.starts_with('/') || p.contains('\\') || p.split('/').any(|c| c.is_empty() || c == "." || c == "..")
    {
        return Err(Error::invalid(format!(
            "{what} '{p}' must be a relative path inside the archive (no leading '/', no '.' or '..', no backslashes)"
        )));
    }
    Ok(())
}

impl Input {
    /// The checks that need only the input itself.
    fn validate(&self) -> Result<()> {
        let knob = &self.knob;
        if let Some(h) = &self.sha256
            && !is_hex(h, &[64])
        {
            return Err(Error::invalid(format!("input '{knob}': '{h}' is not a sha256")));
        }
        if let Some(file) = &self.repo_file {
            if self.url.is_some() || self.sha256.is_some() {
                return Err(Error::invalid(format!(
                    "input '{knob}': repo-file takes the place of url and sha256 (the commit pins it)"
                )));
            }
            validate_member_path(&format!("input '{knob}': repo-file"), file)?;
        }
        if !self.unpack {
            if self.path.is_some() || !self.fixups.is_empty() {
                return Err(Error::invalid(format!("input '{knob}': 'path' and fixups need unpack = true")));
            }
            return Ok(());
        }
        let source = match (&self.repo_file, &self.url) {
            (Some(file), _) => file,
            (None, Some(url)) => {
                if self.sha256.is_none() {
                    return Err(Error::invalid(format!("input '{knob}': unpack needs the archive's sha256")));
                }
                url
            }
            (None, None) => return Err(Error::invalid(format!("input '{knob}': unpack needs a url or a repo-file"))),
        };
        if ArchiveKind::of_url(source).is_none() {
            return Err(Error::invalid(format!(
                "input '{knob}': '{source}' is not an archive cactup can unpack (.tar, .tar.gz, .tgz, .tar.xz, .zip)"
            )));
        }
        if let Some(p) = &self.path {
            validate_member_path(&format!("input '{knob}': path"), p)?;
        }
        for f in &self.fixups {
            validate_member_path(&format!("input '{knob}': fixup file"), &f.file)?;
            if f.replace.is_empty() {
                return Err(Error::invalid(format!("input '{knob}': a fixup of '{}' replaces nothing", f.file)));
            }
            let rest = f.with.replace("{dir}", "");
            if rest.contains('{') || rest.contains('}') {
                return Err(Error::invalid(format!(
                    "input '{knob}': fixup of '{}': '{{dir}}' is the only placeholder 'with' may use",
                    f.file
                )));
            }
        }
        Ok(())
    }
}

/// One source repo (not one CRL section: a section with `$1`/`$2` in its URL spans many repos).
/// Shared with frozen stacks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct RepoPin {
    /// The repo's name as the thornlist derives it (`!NAME`, else the URL's basename).
    pub name: String,
    pub url: String,
    /// The branch or tag the thornlist names. A label only: the commit is what gets fetched.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// The pinned commit (git). Absent means unpinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The pinned content hash (tarball components).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The Cactarium package this commit belongs to, if any: `owner/pkg@X.Y.Z` for the version
    /// that froze it, or `owner/pkg#<sha>` for a commit of the package's repo no version froze
    /// (which needs format 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cactarium: Option<Slug>,
    /// The tree had local modifications at export (only with an explicit override).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

impl RepoPin {
    /// What this record pins its repo to, as [`crate::crl::pin_text`] writes it.
    pub fn pin(&self) -> Option<crate::crl::Pin> {
        use crate::crl::Pin;
        self.commit.clone().map(Pin::Commit).or_else(|| self.sha256.clone().map(Pin::Sha256))
    }
}

/// Suggested run shape, used only as defaults by `cactup import`. Machine-shaped settings
/// (nodes, tasks per node, queue) are deliberately absent: they follow from the target machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct RunHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus_per_task: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpus_per_task: Option<u32>,
    /// Total wall time wanted, `HH:MM:SS`; cactup chains restarts against the queue's limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walltime: Option<String>,
}

/// Where the file came from. Informational: import never requires any of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exported_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exported_at: Option<toml::value::Datetime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// The machine flavor the config was built with. At most one of `flavor` and `optionlist`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavor: Option<String>,
    /// The optionlist file's name, when the config was built from `--optionlist` instead of a flavor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optionlist: Option<String>,
    /// The build universe (e.g. a container image), when not the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub universe: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    /// Thorns the source machine's configuration turned off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub machine_disabled: Vec<String>,
    /// The stack the install was based on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Slug>,
    /// Anything else an exporter wants to record, as strings.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub extra: IndexMap<String, String>,
}

/// Why a `.sim` is not fully specified. Only fully specified files can be published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unspecified {
    Unpinned { repo: String },
    Dirty { repo: String },
    Untracked { thorn: String },
}

impl fmt::Display for Unspecified {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unspecified::Unpinned { repo } => write!(f, "repo '{repo}' has no pinned commit or sha256"),
            Unspecified::Dirty { repo } => write!(f, "repo '{repo}' had local modifications at export"),
            Unspecified::Untracked { thorn } => write!(f, "thorn '{thorn}' is not backed by any repo"),
        }
    }
}

pub(crate) fn is_hex(s: &str, lens: &[usize]) -> bool {
    lens.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn validate_repos(repos: &[RepoPin]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for r in repos {
        if !seen.insert(r.name.as_str()) {
            return Err(Error::invalid(format!("repo '{}' is listed twice", r.name)));
        }
        if r.commit.is_some() && r.sha256.is_some() {
            return Err(Error::invalid(format!("repo '{}' has both a commit and a sha256", r.name)));
        }
        if let Some(c) = &r.commit
            && !is_hex(c, &[40, 64])
        {
            return Err(Error::invalid(format!("repo '{}': '{c}' is not a full lowercase commit id", r.name)));
        }
        if let Some(h) = &r.sha256
            && !is_hex(h, &[64])
        {
            return Err(Error::invalid(format!("repo '{}': '{h}' is not a sha256", r.name)));
        }
    }
    Ok(())
}

fn plain_filename(what: &str, f: &str) -> Result<()> {
    if f.is_empty() || f.contains('/') || f.contains('\\') || f == "." || f == ".." {
        return Err(Error::invalid(format!("{what} '{f}' must be a plain file name")));
    }
    Ok(())
}

impl SimFile {
    /// Any generation from 1 to [`SIM_FORMAT`]; one newer is refused with an upgrade message.
    pub fn from_toml(text: &str) -> Result<SimFile> {
        generation::check(text, "the .sim file", 1, SIM_FORMAT)?;
        let sim: SimFile = toml::from_str(text).map_err(|source| Error::Toml { what: "the .sim file", source })?;
        sim.validate()?;
        Ok(sim)
    }

    /// Written as the oldest generation that holds this content ([`Self::required_format`]),
    /// whatever `format` says, so readers older than a feature the file does not use still read it.
    pub fn to_toml(&self) -> Result<String> {
        let file = SimFile { format: self.required_format(), ..self.clone() };
        file.validate()?;
        toml::to_string(&file).map_err(|source| Error::Serialize { what: "the .sim file", source })
    }

    /// The oldest generation that can hold this file: 4 when it is stamped as one sim of a
    /// multi-sim entry (`owner/name/sim@X.Y.Z`) or a pin is stamped with a commit
    /// (`owner/pkg#<sha>`), 3 when an input is a file of the sim's own repository, 2 when an input
    /// unpacks an archive, else 1.
    pub fn required_format(&self) -> u32 {
        let names_a_sim = self.sim.cactarium.as_ref().is_some_and(|s| s.thorn.is_some());
        let commit_stamp = self.repos.iter().any(|r| r.cactarium.as_ref().is_some_and(|s| s.sha.is_some()));
        if names_a_sim || commit_stamp {
            4
        } else if self.inputs.iter().any(|i| i.repo_file.is_some()) {
            3
        } else if self.inputs.iter().any(|i| i.unpack) {
            2
        } else {
            1
        }
    }

    /// Structural checks that hold for every `.sim`, pinned or not.
    pub fn validate(&self) -> Result<()> {
        if !(1..=SIM_FORMAT).contains(&self.format) {
            return Err(Error::invalid(format!("format must be {}", generation::readable(SIM_FORMAT))));
        }
        if self.format < self.required_format() {
            return Err(Error::invalid(format!(
                "format {} cannot hold this file (archive inputs need format 2, repo-file inputs 3, \
                 a stamp naming one sim of a multi-sim entry or a pin stamped with a commit \
                 (owner/pkg#<sha>) 4); it needs format {}",
                self.format,
                self.required_format()
            )));
        }
        if let Some(stamp) = &self.sim.cactarium {
            stamp.as_sim().map_err(|e| Error::invalid(format!("[sim] cactarium: {e}")))?;
        }
        if self.sim.name.trim().is_empty() {
            return Err(Error::invalid("[sim] name is empty"));
        }
        if let Some(c) = &self.sim.contact {
            crate::manifest::validate_contact(c)?;
        }
        plain_filename("thornlist filename", &self.thornlist.filename)?;
        plain_filename("parfile filename", &self.parfile.filename)?;
        let ext = match self.parfile.kind {
            ParfileKind::Par => ".par",
            ParfileKind::Py => ".py",
        };
        if !self.parfile.filename.ends_with(ext) {
            return Err(Error::invalid(format!(
                "parfile '{}' must end in '{ext}' to match kind = \"{}\"",
                self.parfile.filename,
                &ext[1..]
            )));
        }
        let mut knobs = BTreeSet::new();
        for i in &self.inputs {
            if !knobs.insert(i.knob.as_str()) {
                return Err(Error::invalid(format!("input '{}' is declared twice", i.knob)));
            }
            i.validate()?;
        }
        // `base` may be `owner/stack@head` or `@master`: the [[repo]] pins, not the base, make a .sim reproducible.
        if let Some(p) = &self.provenance
            && p.flavor.is_some()
            && p.optionlist.is_some()
        {
            return Err(Error::invalid("[provenance] has both a flavor and an optionlist"));
        }
        // Package and stack references: a third segment, if any, is a thorn, as before slugs widened.
        if let Some(b) = self.provenance.as_ref().and_then(|p| p.base.as_ref()) {
            b.as_thorn()?;
        }
        for r in &self.repos {
            if let Some(c) = &r.cactarium {
                c.as_thorn()?;
            }
        }
        validate_repos(&self.repos)
    }

    /// Every reason this file cannot be published. Empty means fully specified: every repo is
    /// pinned, nothing was dirty, and every thorn is backed by a repo.
    pub fn unspecified(&self) -> Vec<Unspecified> {
        let mut out = Vec::new();
        for r in &self.repos {
            if r.commit.is_none() && r.sha256.is_none() {
                out.push(Unspecified::Unpinned { repo: r.name.clone() });
            }
            if r.dirty {
                out.push(Unspecified::Dirty { repo: r.name.clone() });
            }
        }
        for t in &self.thornlist.untracked {
            out.push(Unspecified::Untracked { thorn: t.clone() });
        }
        out
    }

    pub fn is_fully_specified(&self) -> bool {
        self.unspecified().is_empty()
    }
}
