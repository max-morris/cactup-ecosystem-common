//! Repo URLs compared as one repo, whatever their spelling. The Cactarium (matching a pin or a
//! sibling entry to a registered repo) and CactUp (telling a re-pointed checkout from a respelled
//! one) both compare with [`normalize_repo_url`], so they cannot drift apart.
//!
//! What compares equal (the key [`normalize_repo_url`] returns):
//!
//! - **The transport is not identity.** A leading `scheme://` is dropped, matched
//!   case-insensitively (`https`, `http`, `ssh`, `git`, `git+ssh`, or any other scheme); so is
//!   the scp-like form `[user@]host:path` (no `://`, and its first `:` comes before any `/`).
//!   `https://github.com/x/y`, `git@github.com:x/y.git` and `ssh://git@github.com:22/x/y/` are one
//!   repo.
//! - **Login material is not identity:** `user[:password]@` and a numeric `:port` are dropped from
//!   the host.
//! - **The host is folded to lowercase** (DNS names are case-insensitive): `GitHub.com` is
//!   `github.com`.
//! - **The path keeps its case.** `EinsteinToolkit/Cottonmouth` stays as written, both in the key
//!   and in anything stored from it. Comparison folds the host only: a forge that ignores path
//!   case (GitHub does) would call `einsteintoolkit/cottonmouth` the same repo, but other forges
//!   and local paths do not, and calling two repos one is worse than asking a user to spell a URL
//!   the way it was registered.
//! - **Path decoration is dropped:** leading and trailing `/`s, then one trailing `.git`
//!   (lowercase, as git writes it): `x/y.git/` is `x/y`.
//! - **Local paths** (`file://…`, `/…`, `./…`, `../…`, `~…`) and anything else without a scheme or
//!   an scp-like host keep their case and their leading character, losing only trailing `/`s and
//!   one `.git`. A scheme-less `github.com/x/y` is such a path, as git reads it.
//! - Surrounding whitespace is ignored; an empty URL stays empty. Nothing else (percent-encoding,
//!   `.`/`..` segments, doubled slashes inside the path) is interpreted.
//!
//! The key is for comparing and indexing only, never for fetching: it has no scheme.

/// The comparison key for a repo URL: two URLs name the same repo exactly when their keys are
/// equal. See the [module docs](self) for what that treats as equal.
pub fn normalize_repo_url(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() {
        return String::new();
    }
    if let Some(rest) = strip_prefix_ignore_case(url, "file://") {
        return clean_local(rest);
    }
    if url.starts_with('/') || url.starts_with("./") || url.starts_with("../") || url.starts_with('~') {
        return clean_local(url);
    }
    let (authority, path) = match scheme_end(url) {
        // URL form: `[user[:password]@]host[:port][/path]`.
        Some(end) => {
            let rest = &url[end..];
            rest.split_once('/').unwrap_or((rest, ""))
        }
        // scp-like `[user@]host:path`: the first `:` comes before any `/`.
        None => match (url.find(':'), url.find('/')) {
            (Some(c), slash) if slash.is_none_or(|s| c < s) => (&url[..c], &url[c + 1..]),
            _ => return clean_local(url),
        },
    };
    let host_port = authority.rsplit_once('@').map_or(authority, |(_userinfo, host)| host);
    let host = host_port
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(host_port, |(host, _port)| host)
        .to_ascii_lowercase();
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() { host } else { format!("{host}/{path}") }
}

/// Whether two URLs name the same repo: their [`normalize_repo_url`] keys are equal.
pub fn same_repo(a: &str, b: &str) -> bool {
    normalize_repo_url(a) == normalize_repo_url(b)
}

/// Where the authority starts after a `scheme://` prefix (`[A-Za-z][A-Za-z0-9+.-]*://`), if the
/// URL has one.
fn scheme_end(url: &str) -> Option<usize> {
    let (scheme, _) = url.split_once("://")?;
    let mut chars = scheme.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c));
    ok.then_some(scheme.len() + 3)
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    s.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &s[prefix.len()..])
}

/// A local path's trailing `/`s and one `.git` dropped; its leading character (which tells an
/// absolute path from a relative or home-relative one) and its case kept.
fn clean_local(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    // `/` alone is still the root.
    let path = if trimmed.is_empty() && !path.is_empty() { "/" } else { trimmed };
    path.strip_suffix(".git").unwrap_or(path).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_of_one_repo_are_equal() {
        let key = "github.com/max-morris/SpacetimeX";
        for u in [
            "https://github.com/max-morris/SpacetimeX",
            "https://github.com/max-morris/SpacetimeX.git",
            "https://github.com/max-morris/SpacetimeX/",
            "https://github.com/max-morris/SpacetimeX.git/",
            "https://github.com/max-morris/SpacetimeX//",
            "HTTPS://GitHub.com/max-morris/SpacetimeX.git",
            "http://github.com/max-morris/SpacetimeX",
            "git://github.com/max-morris/SpacetimeX.git",
            "git@github.com:max-morris/SpacetimeX.git",
            "github.com:max-morris/SpacetimeX",
            "ssh://git@github.com/max-morris/SpacetimeX.git",
            "ssh://git@github.com:22/max-morris/SpacetimeX/",
            "git+ssh://git@github.com/max-morris/SpacetimeX",
            "https://user:token@github.com:443/max-morris/SpacetimeX.git",
            "https://github.com//max-morris/SpacetimeX",
            "  https://github.com/max-morris/SpacetimeX\n",
        ] {
            assert_eq!(normalize_repo_url(u), key, "{u}");
        }
        assert!(same_repo(
            "git@github.com:EinsteinToolkit/Cottonmouth.git",
            "https://github.com/EinsteinToolkit/Cottonmouth"
        ));
    }

    /// The path keeps its case, in the key too: only the host is folded.
    #[test]
    fn path_case_is_kept() {
        assert_eq!(
            normalize_repo_url("https://GitHub.com/EinsteinToolkit/Cottonmouth.git"),
            "github.com/EinsteinToolkit/Cottonmouth"
        );
        assert_eq!(
            normalize_repo_url("git@GITHUB.COM:EinsteinToolkit/Cottonmouth"),
            "github.com/EinsteinToolkit/Cottonmouth"
        );
        assert!(!same_repo(
            "https://github.com/EinsteinToolkit/Cottonmouth",
            "https://github.com/einsteintoolkit/cottonmouth"
        ));
        assert!(!same_repo("https://x.org/repo", "https://x.org/Repo"));
    }

    #[test]
    fn different_repos_differ() {
        assert!(!same_repo(
            "https://github.com/max-morris/SpacetimeX",
            "https://github.com/EinsteinToolkit/SpacetimeX"
        ));
        assert!(!same_repo("https://x.org/repo", "https://x.org/other"));
        assert!(!same_repo("https://x.org/repo", "https://y.org/repo"));
        assert!(!same_repo("https://x.org/a/repo", "https://x.org/repo"));
    }

    #[test]
    fn local_paths_keep_their_shape() {
        assert_eq!(normalize_repo_url("/srv/mirrors/Repo.git"), "/srv/mirrors/Repo");
        assert_eq!(normalize_repo_url("/srv/mirrors/Repo/"), "/srv/mirrors/Repo");
        assert_eq!(normalize_repo_url("file:///srv/mirrors/Repo.git"), "/srv/mirrors/Repo");
        assert_eq!(normalize_repo_url("FILE:///srv/mirrors/Repo"), "/srv/mirrors/Repo");
        assert_eq!(normalize_repo_url("./Repo.git"), "./Repo");
        assert_eq!(normalize_repo_url("../Repo"), "../Repo");
        assert_eq!(normalize_repo_url("~User/Repo"), "~User/Repo");
        assert_eq!(normalize_repo_url("/"), "/");
        // No scheme and no scp-like host: a path, case and all, as git reads it.
        assert_eq!(normalize_repo_url("GitHub.com/x/Y.git"), "GitHub.com/x/Y");
        assert_ne!(normalize_repo_url("/srv/repo"), normalize_repo_url("srv/repo"));
    }

    #[test]
    fn edge_cases() {
        assert_eq!(normalize_repo_url(""), "");
        assert_eq!(normalize_repo_url("   "), "");
        assert_eq!(normalize_repo_url("https://GitHub.com"), "github.com");
        assert_eq!(normalize_repo_url("https://GitHub.com/"), "github.com");
        // Only a lowercase `.git`, once.
        assert_eq!(normalize_repo_url("https://x.org/r.git.git"), "x.org/r.git");
        assert_eq!(normalize_repo_url("https://x.org/r.GIT"), "x.org/r.GIT");
        // A non-numeric "port" is not one.
        assert_eq!(normalize_repo_url("ssh://host:abc/r"), "host:abc/r");
        // A port is dropped, and only the host folded.
        assert_eq!(normalize_repo_url("https://Host.Example:8443/A/B.git"), "host.example/A/B");
    }
}
