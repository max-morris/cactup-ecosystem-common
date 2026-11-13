//! Repo URLs compared as one repo, whatever their spelling. The Cactarium (matching a pin or a
//! sibling entry to a registered repo) and CactUp (telling a re-pointed checkout from a respelled
//! one) both compare with [`normalize_repo_url`], so they cannot drift apart.
//!
//! **The key is an interface.** Keys are compared across the server and CactUp, and the server
//! stores them (`url_norm`). Changing what [`normalize_repo_url`] returns for any input changes
//! which repos match and leaves stored keys stale, so don't change it casually. The golden test in
//! this module pins its output; a change there needs a note to every consumer (and a migration
//! that recomputes stored keys).
//!
//! What compares equal (the key [`normalize_repo_url`] returns):
//!
//! - **The transport is not identity.** A leading `scheme://` is dropped, matched
//!   case-insensitively (`https`, `http`, `ssh`, `git`, `git+ssh`, or any other scheme but
//!   `file`); so is the scp-like form `[user@]host:path` (no `://`, and its first `:` comes before
//!   any `/`). `https://github.com/x/y`, `git@github.com:x/y.git` and `ssh://git@github.com/x/y/`
//!   are one repo.
//! - **Login material is not identity:** `user[:password]@` is dropped from the host, and so is a
//!   query (`?private_token=…`) or fragment of a remote URL. A well-formed URL's credentials never
//!   reach the key. (A password with an unencoded `/` in it is malformed: nothing can tell its
//!   tail from the path, so part of it would. Such a URL is broken for git too.)
//! - **Default ports are not identity:** `:22`, `:80`, `:443` and `:9418` (ssh, http, https, git)
//!   are dropped, whatever the scheme; any other port is kept (`host:2222/x`), and an empty one
//!   (`host:`) loses its `:`. The tradeoff: the scp-like form can't carry a port (ssh's config
//!   chooses it), so `git@host:x` equals `ssh://git@host:22/x` but not `ssh://git@host:2222/x`,
//!   even when the user's ssh config sends `host` to port 2222. Calling two servers one would be
//!   worse than missing that.
//! - **The host is folded to lowercase** (DNS names are case-insensitive): `GitHub.com` is
//!   `github.com`.
//! - **The path keeps its case**, in the key and in anything stored from it:
//!   `EinsteinToolkit/Cottonmouth` stays as written. Some forges ignore path case (GitHub, GitLab
//!   and Bitbucket do, [`forge_ignores_path_case`]); others and local paths don't. Where it
//!   matters, a caller compares keys case-insensitively as a fallback for those forges; the key
//!   itself never folds the path.
//! - **Path decoration is dropped:** leading and trailing `/`s, then one trailing `.git`
//!   (lowercase, as git writes it): `x/y.git/` is `x/y`.
//! - **Local paths can never equal a remote repo.** `file://…` (with `localhost` or no host),
//!   `/…`, `./…`, `../…`, `~…` and anything else without a scheme or an scp-like host (including a
//!   scheme-less `github.com/x/y`, which git reads as a path) are keyed `file://<path>`, keeping
//!   their case and their leading character and losing only trailing `/`s and one `.git`. A remote
//!   key never starts with `file://`; one with an empty host (`https:///srv/x`) is `/srv/x`, which
//!   no local path's key is.
//! - Surrounding whitespace is ignored; an empty URL stays empty. Nothing else (percent-encoding,
//!   `.`/`..` segments, doubled slashes inside the path) is interpreted.
//!
//! The key is for comparing and indexing only, never for fetching.

/// The prefix of every local path's key.
const LOCAL: &str = "file://";

/// The ports dropped from a key: ssh, http, https and git's defaults.
pub const DEFAULT_PORTS: [&str; 4] = ["22", "80", "443", "9418"];

/// The comparison key for a repo URL: two URLs name the same repo exactly when their keys are
/// equal. See the [module docs](self) for what that treats as equal.
pub fn normalize_repo_url(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() {
        return String::new();
    }
    if let Some(rest) = strip_prefix_ignore_case(url, "file://") {
        // `file:///p` and `file://localhost/p` are the local `/p`; another host stays in the path.
        let rest = strip_prefix_ignore_case(rest, "localhost")
            .filter(|r| r.starts_with('/'))
            .unwrap_or(rest);
        return local(rest);
    }
    if url.starts_with('/') || url.starts_with("./") || url.starts_with("../") || url.starts_with('~') {
        return local(url);
    }
    let (authority, path) = match scheme_end(url) {
        // URL form: `[user[:password]@]host[:port][/path][?query][#fragment]`.
        Some(end) => {
            let rest = &url[end..];
            let rest = rest.split(['?', '#']).next().unwrap_or(rest);
            rest.split_once('/').unwrap_or((rest, ""))
        }
        None => match scp(url) {
            Some((authority, path)) => (authority, path.split(['?', '#']).next().unwrap_or(path)),
            None => return local(url),
        },
    };
    let host_port = authority.rsplit_once('@').map_or(authority, |(_userinfo, host)| host);
    let host = match host_port.rsplit_once(':') {
        Some((host, port)) if port.is_empty() || DEFAULT_PORTS.contains(&port) => host,
        _ => host_port,
    }
    .to_ascii_lowercase();
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() { host } else { format!("{host}/{path}") }
}

/// Whether two URLs name the same repo: their [`normalize_repo_url`] keys are equal.
pub fn same_repo(a: &str, b: &str) -> bool {
    normalize_repo_url(a) == normalize_repo_url(b)
}

/// Whether a key's host is a forge known to ignore the case of owner and repo names (github.com,
/// gitlab.com, bitbucket.org), where comparing keys case-insensitively is a fair fallback.
pub fn forge_ignores_path_case(key: &str) -> bool {
    let host = key.split('/').next().unwrap_or("");
    ["github.com", "gitlab.com", "bitbucket.org"].contains(&host)
}

/// The scp-like form `[user[:password]@]host:path`: (authority, path) when the URL has one. The
/// separating `:` is the first after the last `@` before any `/`, when one follows it; else the
/// first `:` (so the path may contain an `@`). It must come before any `/`.
fn scp(url: &str) -> Option<(&str, &str)> {
    let head = &url[..url.find('/').unwrap_or(url.len())];
    if let Some(at) = head.rfind('@')
        && let Some(c) = head[at..].find(':')
    {
        return Some((&url[..at + c], &url[at + c + 1..]));
    }
    let c = head.find(':')?;
    Some((&url[..c], &url[c + 1..]))
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

/// A local path's key: `file://` and the path, its trailing `/`s and one `.git` dropped, its
/// leading character (which tells an absolute path from a relative or home-relative one) and its
/// case kept.
fn local(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    // `/` alone is still the root.
    let path = if trimmed.is_empty() && !path.is_empty() { "/" } else { trimmed };
    format!("{LOCAL}{}", path.strip_suffix(".git").unwrap_or(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key's output, pinned: the server stores keys and CactUp compares against them, so a
    /// change here changes which repos match. Don't edit a row without telling every consumer.
    #[test]
    fn golden_keys() {
        let table: &[(&str, &str)] = &[
            ("https://github.com/EinsteinToolkit/Cottonmouth", "github.com/EinsteinToolkit/Cottonmouth"),
            (
                "https://github.com/EinsteinToolkit/Cottonmouth.git",
                "github.com/EinsteinToolkit/Cottonmouth",
            ),
            (
                "https://GitHub.com/EinsteinToolkit/Cottonmouth/",
                "github.com/EinsteinToolkit/Cottonmouth",
            ),
            ("git@github.com:EinsteinToolkit/Cottonmouth.git", "github.com/EinsteinToolkit/Cottonmouth"),
            ("ssh://git@github.com/x/y.git", "github.com/x/y"),
            ("ssh://git@github.com:22/x/y/", "github.com/x/y"),
            ("ssh://git@host.org:2222/x/y", "host.org:2222/x/y"),
            ("git://host.org:9418/x", "host.org/x"),
            ("http://host.org:80/x", "host.org/x"),
            ("https://host.org:443/x", "host.org/x"),
            ("https://host.org:8443/x", "host.org:8443/x"),
            ("https://host.org:/x", "host.org/x"),
            ("ssh://host:abc/r", "host:abc/r"),
            ("https://user:token@github.com/x/y", "github.com/x/y"),
            ("user:secret@host.org:x/y", "host.org/x/y"),
            ("host.org:x/repo@v1", "host.org/x/repo@v1"),
            ("https://gitlab.com/x/y.git?private_token=secret", "gitlab.com/x/y"),
            ("https://gitlab.com/x/y#readme", "gitlab.com/x/y"),
            ("https://github.com/x/y.git//", "github.com/x/y"),
            ("https://github.com//x/y", "github.com/x/y"),
            ("HTTPS://github.com/x/y", "github.com/x/y"),
            ("foo+bar://Host/x", "host/x"),
            ("https://Host", "host"),
            ("https://x.org/r.git.git", "x.org/r.git"),
            ("https://x.org/r.GIT", "x.org/r.GIT"),
            ("https:///srv/x", "/srv/x"),
            ("/srv/mirrors/Repo.git/", "file:///srv/mirrors/Repo"),
            ("file:///srv/mirrors/Repo.git", "file:///srv/mirrors/Repo"),
            ("FILE://localhost/srv/mirrors/Repo", "file:///srv/mirrors/Repo"),
            ("file://server/share/Repo", "file://server/share/Repo"),
            ("./Repo.git", "file://./Repo"),
            ("../Repo", "file://../Repo"),
            ("~User/Repo", "file://~User/Repo"),
            ("/", "file:///"),
            ("GitHub.com/x/Y.git", "file://GitHub.com/x/Y"),
            ("", ""),
            ("   ", ""),
        ];
        for (url, key) in table {
            assert_eq!(normalize_repo_url(url), *key, "{url}");
        }
    }

    #[test]
    fn spellings_of_one_repo_are_equal() {
        let key = "github.com/max-morris/SpacetimeX";
        for u in [
            "https://github.com/max-morris/SpacetimeX",
            "https://github.com/max-morris/SpacetimeX.git/",
            "HTTPS://GitHub.com/max-morris/SpacetimeX.git",
            "http://github.com/max-morris/SpacetimeX",
            "git://github.com/max-morris/SpacetimeX.git",
            "git@github.com:max-morris/SpacetimeX.git",
            "github.com:max-morris/SpacetimeX",
            "ssh://git@github.com:22/max-morris/SpacetimeX/",
            "git+ssh://git@github.com/max-morris/SpacetimeX",
            "https://user:token@github.com:443/max-morris/SpacetimeX.git",
            "  https://github.com/max-morris/SpacetimeX\n",
        ] {
            assert_eq!(normalize_repo_url(u), key, "{u}");
        }
    }

    /// The path keeps its case, in the key too: only the host is folded.
    #[test]
    fn path_case_is_kept() {
        assert!(!same_repo(
            "https://github.com/EinsteinToolkit/Cottonmouth",
            "https://github.com/einsteintoolkit/cottonmouth"
        ));
        assert!(!same_repo("https://x.org/repo", "https://x.org/Repo"));
        assert!(forge_ignores_path_case(&normalize_repo_url(
            "git@GitHub.com:EinsteinToolkit/Cottonmouth"
        )));
        assert!(
            forge_ignores_path_case("gitlab.com/x/y") && forge_ignores_path_case("bitbucket.org/x/y")
        );
        assert!(
            !forge_ignores_path_case("git.example.org/x/y")
                && !forge_ignores_path_case("file:///github.com/x")
        );
    }

    #[test]
    fn different_repos_differ() {
        assert!(!same_repo(
            "https://github.com/max-morris/SpacetimeX",
            "https://github.com/EinsteinToolkit/SpacetimeX"
        ));
        assert!(!same_repo("https://x.org/repo", "https://y.org/repo"));
        assert!(!same_repo("https://x.org/a/repo", "https://x.org/repo"));
        assert!(!same_repo("ssh://git@host:2222/x", "git@host:x"), "another port is another server");
    }

    /// A local path's key can never equal a remote repo's.
    #[test]
    fn local_and_remote_never_meet() {
        for (local, remote) in [
            ("github.com/x/y", "https://github.com/x/y"),
            ("file://localhost/srv/x", "https://localhost/srv/x"),
            ("/srv/x", "https:///srv/x"),
            ("file:///srv/x", "https:///srv/x"),
            ("file/x", "file:x"),
        ] {
            assert!(!same_repo(local, remote), "{local} vs {remote}");
        }
        assert!(same_repo("/srv/x", "file:///srv/x") && same_repo("/srv/x", "file://localhost/srv/x/"));
        assert!(!same_repo("/srv/repo", "srv/repo"));
        // No remote spelling yields a key in the local shape.
        for remote in ["https://file:/x", "file:x", "https://file:///x", "ssh://file:22//x", "file:/x"] {
            assert!(!normalize_repo_url(remote).starts_with(LOCAL), "{remote}");
        }
    }

    #[test]
    fn credentials_never_reach_the_key() {
        for u in [
            "https://alice:s3cret@host.org/x/y",
            "alice:s3cret@host.org:x/y",
            "https://host.org/x/y?private_token=s3cret",
            "https://host.org/x/y.git?private_token=s3cret#frag",
            "alice:s3cret@host.org:x/y?private_token=s3cret",
        ] {
            let key = normalize_repo_url(u);
            assert!(!key.contains("s3cret") && !key.contains("alice"), "{u} -> {key}");
            assert_eq!(key, "host.org/x/y", "{u}");
        }
    }
}
