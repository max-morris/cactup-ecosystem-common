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
//!   query (`?private_token=…`) or fragment of a remote URL. The authority runs to the first `/`
//!   and the userinfo to its last `@`, so even an unencoded `?`, `#` or `@` in a password stays out
//!   of the key. (A password with an unencoded `/` in it is malformed beyond that: nothing can tell
//!   its tail from the path, so part of it would reach the key. Such a URL is broken for git too.
//!   And a URL with no path but a query holding an `@` loses its host; no repo URL is one.)
//! - **Default ports are not identity:** `:22`, `:80`, `:443` and `:9418` (ssh, http, https, git)
//!   are dropped, whatever the scheme; any other port is kept (`host:2222/x`), and an empty one
//!   (`host:`) loses its `:`. The tradeoff: the scp-like form can't carry a port (ssh's config
//!   chooses it), so `git@host:x` equals `ssh://git@host:22/x` but not `ssh://git@host:2222/x`,
//!   even when the user's ssh config sends `host` to port 2222. Calling two servers one would be
//!   worse than missing that.
//! - **IPv6 hosts** are written in brackets, `ssh://git@[::1]:22/x` or scp-like `git@[::1]:x`, and
//!   keyed with them (`[::1]/x`). A bracket that doesn't close right before the scp `:` makes it
//!   no scp form (keyed as a local path).
//! - **The host is folded to lowercase** (DNS names are case-insensitive): `GitHub.com` is
//!   `github.com`.
//! - **The path keeps its case**, in the key and in anything stored from it:
//!   `EinsteinToolkit/Cottonmouth` stays as written. Some forges ignore path case (GitHub, GitLab
//!   and Bitbucket do, [`forge_ignores_path_case`]); others and local paths don't. Where it
//!   matters, a caller uses [`same_repo_lenient`], the shared fallback that compares paths
//!   case-insensitively on those forges (and counts `www.github.com` as `github.com`); the key
//!   itself never folds the path or a `www.`.
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

/// The forges known to ignore the case of owner and repo names.
const CASELESS_FORGES: [&str; 3] = ["github.com", "gitlab.com", "bitbucket.org"];

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
        // URL form: `[user[:password]@]host[:port][/path][?query][#fragment]`. The authority runs
        // to the first `/` and the userinfo to its last `@`, so a `?` or `#` in a (malformed,
        // unencoded) password can't hide the host; only then is a query or fragment cut off.
        Some(end) => {
            let rest = &url[end..];
            rest.split_once('/').unwrap_or((rest, ""))
        }
        None => match scp(url) {
            Some(parts) => parts,
            None => return local(url),
        },
    };
    let host_port = authority.rsplit_once('@').map_or(authority, |(_userinfo, host)| host);
    let host_port = before_query(host_port);
    let host = match split_port(host_port) {
        (host, Some(port)) if port.is_empty() || DEFAULT_PORTS.contains(&port) => host,
        _ => host_port,
    }
    .to_ascii_lowercase();
    let path = before_query(path).trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() { host } else { format!("{host}/{path}") }
}

/// Whether two URLs name the same repo: their [`normalize_repo_url`] keys are equal.
pub fn same_repo(a: &str, b: &str) -> bool {
    normalize_repo_url(a) == normalize_repo_url(b)
}

/// Whether two URLs name the same repo, allowing for forges that ignore path case: [`same_repo`],
/// or both on the same such forge ([`forge_ignores_path_case`]; `www.github.com` counts as
/// `github.com`, which it redirects to) with paths equal but for case. The shared fallback for a
/// match [`same_repo`] misses, such as a pin spelled `einsteintoolkit/cottonmouth` against a repo
/// registered as `EinsteinToolkit/Cottonmouth`. A caller that stores keys still stores
/// [`normalize_repo_url`]'s: `www.` and path case are folded only here, never in the key.
pub fn same_repo_lenient(a: &str, b: &str) -> bool {
    let (ka, kb) = (normalize_repo_url(a), normalize_repo_url(b));
    if ka == kb {
        return true;
    }
    let path = |k: &str| k.split_once('/').map(|(_, p)| p.to_owned()).unwrap_or_default();
    match (caseless_forge(&ka), caseless_forge(&kb)) {
        (Some(fa), Some(fb)) => fa == fb && path(&ka).eq_ignore_ascii_case(&path(&kb)),
        _ => false,
    }
}

/// Whether a key's host is a forge known to ignore the case of owner and repo names (github.com,
/// gitlab.com, bitbucket.org, or their `www.` names), where comparing paths case-insensitively is
/// a fair fallback ([`same_repo_lenient`]).
pub fn forge_ignores_path_case(key: &str) -> bool {
    caseless_forge(key).is_some()
}

/// The forge a key's host is, when it is one of [`CASELESS_FORGES`] (or its `www.` name).
fn caseless_forge(key: &str) -> Option<&'static str> {
    let host = key.split('/').next().unwrap_or("");
    let host = host.strip_prefix("www.").unwrap_or(host);
    CASELESS_FORGES.into_iter().find(|f| *f == host)
}

/// `s` up to its first `?` or `#`.
fn before_query(s: &str) -> &str {
    s.split(['?', '#']).next().unwrap_or(s)
}

/// A host and its port, if one is written: `host:2222` → (`host`, `2222`), `[::1]:22` →
/// (`[::1]`, `22`), `[::1]` → (`[::1]`, none). The port may be empty (`host:`).
fn split_port(host_port: &str) -> (&str, Option<&str>) {
    if host_port.starts_with('[')
        && let Some(close) = host_port.find(']')
    {
        let (host, rest) = host_port.split_at(close + 1);
        return (host, rest.strip_prefix(':'));
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (host_port, None),
    }
}

/// The scp-like form `[user[:password]@]host:path`: (authority, path) when the URL has one. The
/// host starts after the last `@` before any `/` when a `:` follows that `@`, else at the start
/// (so the path may contain an `@`); the separating `:` is the host's first, before any `/`. A
/// bracketed IPv6 host (`git@[::1]:x/y`) separates at the `:` right after its `]`; without one
/// there, it is no scp form.
fn scp(url: &str) -> Option<(&str, &str)> {
    let head = &url[..url.find('/').unwrap_or(url.len())];
    let host_at = head.rfind('@').filter(|at| head[*at..].contains(':')).map_or(0, |at| at + 1);
    if head[host_at..].starts_with('[') {
        let close = host_at + head[host_at..].find(']')?;
        return head[close + 1..]
            .starts_with(':')
            .then(|| (&url[..close + 1], &url[close + 2..]));
    }
    let c = host_at + head[host_at..].find(':')?;
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
            (
                "https://www.github.com/EinsteinToolkit/Cottonmouth",
                "www.github.com/EinsteinToolkit/Cottonmouth",
            ),
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
            ("https://user:pa?ss@host.org/x", "host.org/x"),
            ("https://user:pa#ss@host.org/x.git", "host.org/x"),
            ("https://user:p?a#s@host.org:443/x?token=t#f", "host.org/x"),
            ("https://us@er:pass@host.org/x", "host.org/x"),
            ("https://host.org?token=t", "host.org"),
            ("host.org:x/repo@v1", "host.org/x/repo@v1"),
            ("https://gitlab.com/x/y.git?private_token=secret", "gitlab.com/x/y"),
            ("https://gitlab.com/x/y#readme", "gitlab.com/x/y"),
            ("git@[::1]:x/y.git", "[::1]/x/y"),
            ("[::1]:x/y", "[::1]/x/y"),
            ("ssh://git@[::1]:22/x/y", "[::1]/x/y"),
            ("ssh://git@[::1]:2222/x/y", "[::1]:2222/x/y"),
            ("https://[2001:DB8::1]/x", "[2001:db8::1]/x"),
            ("git@[::1/x", "file://git@[::1/x"),
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
        assert!(forge_ignores_path_case("www.github.com/x/y"));
        assert!(
            !forge_ignores_path_case("git.example.org/x/y")
                && !forge_ignores_path_case("file:///github.com/x")
        );
    }

    /// The shared fallback for forges that ignore path case.
    #[test]
    fn lenient_matching() {
        let registered = "https://github.com/EinsteinToolkit/Cottonmouth.git";
        for u in [
            registered,
            "git@github.com:einsteintoolkit/cottonmouth",
            "https://www.github.com/EinsteinToolkit/Cottonmouth",
            "https://WWW.GitHub.com/einsteintoolkit/COTTONMOUTH/",
        ] {
            assert!(same_repo_lenient(registered, u), "{u}");
        }
        assert!(
            !same_repo("https://www.github.com/EinsteinToolkit/Cottonmouth", registered),
            "the key keeps www."
        );
        assert!(same_repo_lenient("https://gitlab.com/Group/Proj", "https://gitlab.com/group/proj"));
        assert!(same_repo_lenient(
            "https://bitbucket.org/Team/Repo",
            "https://www.bitbucket.org/team/repo"
        ));
        // Elsewhere, and across forges, case and hosts still count.
        assert!(!same_repo_lenient("https://git.example.org/X/Y", "https://git.example.org/x/y"));
        assert!(!same_repo_lenient("https://github.com/x/y", "https://gitlab.com/x/y"));
        assert!(!same_repo_lenient("https://github.com/x/y", "https://github.com/x/z"));
        assert!(!same_repo_lenient("https://github.com/x/y", "/x/y"));
        assert!(!same_repo_lenient("file://github.com/x/y", "https://github.com/x/y"));
        assert!(!same_repo_lenient("github.com/x/y", "https://github.com/x/y"));
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
        assert!(!same_repo("git@[::1]:x", "git@[::2]:x"));
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
            "https://alice:s3?cret@host.org/x/y",
            "https://alice:s3#cret@host.org/x/y",
            "https://alice:s3cret?x#y@host.org:443/x/y?private_token=s3cret",
            "https://alice@s3cret:pw@host.org/x/y",
            "ssh://alice:s3cret@host.org:22/x/y.git",
        ] {
            let key = normalize_repo_url(u);
            assert!(!key.contains("s3") && !key.contains("alice"), "{u} -> {key}");
            assert_eq!(key, "host.org/x/y", "{u}");
        }
    }
}
