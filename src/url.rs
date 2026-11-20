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
//!
//! Whether a URL may be published for others to fetch at all (no credentials, no local paths,
//! ssh only as an https URL that answers anonymously) is [`shareability`], shared the same way.

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

/// Whether a repo URL may be published for others to fetch: what the Cactarium checks before it
/// stores a URL and what `cactup sim export` checks before it writes one, so both refuse exactly
/// the same URLs.
///
/// - **[`Shareable`](Shareability::Shareable):** `https://` or `http://`, with no userinfo and no
///   query, so far as its form goes. It still has to answer the anonymous probe
///   ([`crate::probe`]): every git remote is probed before it is published.
/// - **[`Credentials`](Shareability::Credentials):** userinfo in any URL but an ssh one
///   (`https://user:TOKEN@host/…`, a token alone as `https://TOKEN@host/…`), a password in any URL
///   (`ssh://git:pw@host/…`, scp-like `git:pw@host:…`), or a query string, which is where forges
///   take tokens (`?private_token=…`). An ssh user name alone (`git@`) is not a credential.
/// - **[`Local`](Shareability::Local):** `file://…` and paths (anything [`normalize_repo_url`]
///   keys as local, including a scheme-less `github.com/x/y`, which git reads as a path). Never
///   shareable.
/// - **[`Ssh`](Shareability::Ssh):** `ssh://`, `git+ssh://`, `ssh+git://` and scp-like
///   `[user@]host:path`. Nobody can fetch these without an account on the host, so they are
///   shareable only as their https equivalent ([`https_equivalent`]), and only when that answers
///   with no credentials at all. Then the published URL is that https URL; otherwise the URL is
///   refused ([`Shareability::refusal`], or [`moved_refusal`] when the answer was a redirect).
/// - **[`Unsupported`](Shareability::Unsupported):** an empty URL, `git://` (it can't be probed
///   over HTTP, and has no encryption: refused rather than published unchecked), another scheme
///   (`s3://`, `ftp://`, …), a remote helper (`hg::…`, `ext::…`), a URL with no host, or one whose port
///   isn't a number (`https://user:pa/ss@host/x`: a password with an unencoded `/`, where the
///   `/` ends the authority before the `@`).
///
/// Checked in that order: a local path is `Local` even with an `@` in it, and an ssh URL with a
/// password is `Credentials`.
///
/// # The anonymous probe
///
/// For [`Ssh`](Shareability::Ssh), the caller asks the https equivalent itself (this crate does
/// no network), by the rules in [`crate::probe`], whose [`next_step`](crate::probe::next_step)
/// decides what follows. **The probe must carry no credentials of any kind:** no credential helpers
/// (`-c credential.helper=`), no `.netrc` (a `HOME` of its own), no `http.extraHeader` or cookie
/// file, no `insteadOf` rewriting from the user's config, no prompts (`GIT_TERMINAL_PROMPT=0`, no
/// askpass), and of course no ssh keys (it is https). Otherwise a private repo looks public to
/// whoever happens to have access to it, and gets published to people who can't fetch it. A
/// redirect or an authentication challenge (401/403) counts as not answering. Ask what `git
/// ls-remote` would (the smart-HTTP ref advertisement); a web page answering at the URL is not
/// enough.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shareability {
    /// Publishable as written.
    Shareable,
    /// Carries a user name, password, token or query string; refused, whatever else it is.
    Credentials,
    /// A local path or `file://` URL; refused.
    Local,
    /// An ssh remote: shareable as `https` when that answers anonymously (see the
    /// [probe rules](Shareability#the-anonymous-probe)); refused when it doesn't, or when there is
    /// no https equivalent (`None`: a home-relative `~` path, or no path at all).
    Ssh { https: Option<String> },
    /// Not something others can fetch with git: refused.
    Unsupported,
}

/// Whether `url` may be published for others to fetch. See [`Shareability`]. Surrounding
/// whitespace is ignored.
pub fn shareability(url: &str) -> Shareability {
    let url = unquote(url);
    let Some(p) = Parts::of(url) else {
        return if is_local(url) { Shareability::Local } else { Shareability::Unsupported };
    };
    if p.userinfo.is_some_and(|u| u.contains(':') || !p.ssh) || p.query {
        return Shareability::Credentials;
    }
    // A port that isn't a number is no port: most likely a password with an unencoded `/` in it
    // (`https://user:pa/ss@host/x`), which leaves no userinfo to see. Nothing to publish.
    if split_port(p.host_port).1.is_some_and(|port| !port.bytes().all(|b| b.is_ascii_digit())) {
        return Shareability::Unsupported;
    }
    if p.ssh {
        return Shareability::Ssh { https: https_for(p.host_port, p.path) };
    }
    if p.host_port.is_empty() {
        return Shareability::Unsupported;
    }
    match p.scheme.map(str::to_ascii_lowercase).as_deref() {
        Some("https" | "http") => Shareability::Shareable,
        _ => Shareability::Unsupported,
    }
}

/// `url` without surrounding whitespace and one layer of matching quotes (`"…"` or `'…'`): a
/// thornlist may quote its URLs, and the quotes are no part of the URL. [`shareability`],
/// [`download_shareability`] and [`Redacted`] all look through them.
pub fn unquote(url: &str) -> &str {
    let t = url.trim();
    for q in ['"', '\''] {
        if let Some(inner) = t.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner.trim();
        }
    }
    t
}

/// Whether a URL that names a file rather than a git remote (a download: a tarball component, a
/// sim's input, a `.sim` pin by sha256) may be published for others to fetch. The same verdicts
/// as [`shareability`], under rules for downloads:
///
/// - **[`Shareable`](Shareability::Shareable):** `https://`, `http://` or `ftp://` with a host and
///   no userinfo. A query string is fine (`?download=1`, `?raw=true`) unless one of its keys (or a
///   fragment's) [looks like a credential](is_credential_key).
/// - **[`Credentials`](Shareability::Credentials):** a password anywhere, userinfo in any URL but an
///   ssh one, or a credential-like query or fragment key (`private_token`, `X-Amz-Signature`, …).
///   A presigned URL is refused this way, and it would expire anyway.
/// - **[`Local`](Shareability::Local):** `file://…` and paths.
/// - **[`Unsupported`](Shareability::Unsupported):** anything over ssh (`ssh://`, `svn+ssh://`,
///   scp-like), which needs an account on the host; other schemes; no host; a non-numeric port.
///   Downloads have no https equivalent to swap in, so [`Ssh`](Shareability::Ssh) never comes up.
///
/// Refusals read best through [`Shareability::download_refusal`].
pub fn download_shareability(url: &str) -> Shareability {
    let url = unquote(url);
    let Some(p) = Parts::of(url) else {
        return if is_local(url) { Shareability::Local } else { Shareability::Unsupported };
    };
    let over_ssh = p.ssh || p.scheme.is_some_and(|s| s.to_ascii_lowercase().contains("ssh"));
    if p.userinfo.is_some_and(|u| u.contains(':') || !over_ssh) || credential_in_query(&url[p.after_userinfo..]) {
        return Shareability::Credentials;
    }
    if over_ssh
        || p.host_port.is_empty()
        || split_port(p.host_port).1.is_some_and(|port| !port.bytes().all(|b| b.is_ascii_digit()))
    {
        return Shareability::Unsupported;
    }
    match p.scheme.map(str::to_ascii_lowercase).as_deref() {
        Some("https" | "http" | "ftp") => Shareability::Shareable,
        _ => Shareability::Unsupported,
    }
}

/// Whether a query or fragment key names a credential, compared case-insensitively: exactly
/// `auth`, `key`, `sig`, `signature`, `token`, `password`, `pass`, `pwd` or `secret`; containing
/// `token`, `secret`, `passw`, `credential`, `signature`, `apikey`, `api_key`, `api-key`,
/// `access_key`, `accesskey` or `session`; or starting with `x-amz-`, `x-goog-` or `x-ms-`
/// (presigned cloud-storage URLs). A denylist, so an ordinary download link's query passes.
pub fn is_credential_key(key: &str) -> bool {
    let k = key.trim().to_ascii_lowercase();
    const EXACT: [&str; 9] = ["auth", "key", "sig", "signature", "token", "password", "pass", "pwd", "secret"];
    const WITHIN: [&str; 11] = [
        "token", "secret", "passw", "credential", "signature", "apikey", "api_key", "api-key", "access_key", "accesskey",
        "session",
    ];
    const PREFIX: [&str; 3] = ["x-amz-", "x-goog-", "x-ms-"];
    EXACT.contains(&k.as_str()) || WITHIN.iter().any(|w| k.contains(w)) || PREFIX.iter().any(|p| k.starts_with(p))
}

/// Whether the query or fragment of `tail` (a URL from its host on) has a credential-like key.
fn credential_in_query(tail: &str) -> bool {
    let Some(at) = tail.find(['?', '#']) else { return false };
    tail[at + 1..]
        .split(['&', ';', '?', '#'])
        .map(|pair| pair.split_once('=').map_or(pair, |(k, _)| k))
        .any(is_credential_key)
}

/// The https URL an ssh remote would be published as: same host (its ssh port dropped), same path
/// with its trailing `/`s dropped and any `.git` kept as written (the forges serve a repo with and
/// without it). `None` when `url` isn't an ssh remote, carries credentials, or has no https
/// equivalent (a `~` path, no path). `git@github.com:x/y.git` → `https://github.com/x/y.git`;
/// `ssh://git@host:2222/x/y` → `https://host/x/y`.
pub fn https_equivalent(url: &str) -> Option<String> {
    match shareability(url) {
        Shareability::Ssh { https } => https,
        _ => None,
    }
}

impl Shareability {
    /// Why `url` (this is its [`shareability`]) can't be published as written and what to do about
    /// it, as one sentence with the URL [redacted](Redacted); `None` when it is
    /// [`Shareable`](Shareability::Shareable). For [`Ssh`](Shareability::Ssh) with an https
    /// equivalent, the message is for when the probe got no anonymous answer; a caller whose probe
    /// succeeded publishes the https URL instead.
    pub fn refusal(&self, url: &str) -> Option<String> {
        let shown = Redacted(url);
        Some(match self {
            Shareability::Shareable => return None,
            Shareability::Credentials => credentials_refusal(url, false),
            Shareability::Local => format!(
                "{shown} is a local path, which nobody else can fetch; push the repo to a host others \
                 can reach and use its https URL"
            ),
            Shareability::Ssh { https: Some(https) } => format!(
                "{shown} is an ssh URL, which needs an account on the host, and {https} doesn't answer \
                 without credentials; use the https URL of a repo anyone can fetch"
            ),
            Shareability::Ssh { https: None } => format!(
                "{shown} is an ssh URL, which needs an account on the host, and has no https \
                 equivalent; use the https URL of a repo anyone can fetch"
            ),
            Shareability::Unsupported if unquote(url).get(..6).is_some_and(|h| h.eq_ignore_ascii_case("git://")) => {
                format!(
                    "{shown} uses git://, which can't be checked for anonymous access and has no \
                     encryption; use the repo's https URL"
                )
            }
            Shareability::Unsupported => format!(
                "{shown} is not a URL others can fetch with git; use the repo's https URL"
            ),
        })
    }
}

impl Shareability {
    /// [`Shareability::refusal`] for a download's URL (its [`download_shareability`]): why it
    /// can't be published and what to do, with the URL [redacted](Redacted); `None` when it is
    /// shareable.
    pub fn download_refusal(&self, url: &str) -> Option<String> {
        let shown = Redacted(url);
        Some(match self {
            Shareability::Shareable => return None,
            Shareability::Credentials => credentials_refusal(url, true),
            Shareability::Local => format!(
                "{shown} is a local path, which nobody else can download; put the file where others can \
                 reach it and use that URL"
            ),
            Shareability::Ssh { .. } | Shareability::Unsupported => format!(
                "{shown} is not a URL others can download from (ssh needs an account on the host); use \
                 an https URL"
            ),
        })
    }
}

/// The one wording for a URL refused for its credentials, git remote or download alike, naming
/// what was found: a user name or token (userinfo), a password, a credential-like query key (by
/// name: `private_token`), or, for a git remote, any query string.
fn credentials_refusal(url: &str, download: bool) -> String {
    let u = unquote(url);
    let mut found: Vec<String> = Vec::new();
    if let Some(p) = Parts::of(u) {
        match p.userinfo.map(|ui| ui.split_once(':')) {
            Some(Some(("", _))) => found.push("a password".into()),
            Some(Some(_)) => found.push("a user name and password".into()),
            Some(None) if !p.ssh => found.push("a user name or token".into()),
            _ => {}
        }
        let tail = &u[p.after_userinfo..];
        if let Some(at) = tail.find(['?', '#']) {
            let keys: Vec<&str> = tail[at + 1..]
                .split(['&', ';', '?', '#'])
                .map(|pair| pair.split_once('=').map_or(pair, |(k, _)| k))
                .filter(|k| is_credential_key(k))
                .collect();
            match keys.first() {
                Some(k) => found.push(format!("`{k}` in its query")),
                None if !download && tail[at..].starts_with('?') => found.push("a query string".into()),
                None => {}
            }
        }
    }
    let what = if found.is_empty() { "credentials".to_owned() } else { found.join(" and ") };
    format!("{} carries {what}, which must never be published; remove it from the URL", Redacted(url))
}

/// The refusal for a remote whose probe never asked anything: its host resolves to an address that
/// isn't public ([`crate::probe::is_public_address`]). `host` is the host as written.
pub fn private_address_refusal(url: &str, host: &str) -> String {
    format!(
        "{} can't be checked: {host} is not a public address, so nobody outside your network can fetch \
         it; publish a URL on a public host",
        Redacted(url)
    )
}

/// The refusal for a remote whose host couldn't be reached at all (no such host, refused, timed
/// out, no route): nothing was asked, so nothing is known about the repo. `detail` says what
/// happened; it is shown as given, so keep URLs out of it or redact them.
pub fn unreachable_refusal(url: &str, detail: &str) -> String {
    format!("{} can't be checked: its host couldn't be reached ({detail}); try again later", Redacted(url))
}

/// The refusal for a git remote `url` whose https URL `https` (`url` itself, or an ssh URL's
/// [`https_equivalent`]) answered the anonymous probe with a redirect, which counts as no answer
/// (the probe follows none, but one to the same repo is asked once more:
/// [`crate::probe::next_step`]). `to` is where the redirect pointed, when the probe saw it.
///
/// A redirect is a move ([`redirect_is_move`]: GitHub answers a renamed or transferred repo with a
/// 301) only when it stays on the same host, at another repo path, with no query and nothing that
/// looks like a sign-in page. Then the message says to point the remote at the new URL. Any other
/// redirect (to a sign-in page, which is how some forges answer a stranger asking for a private
/// repo, to another host, or with no location seen) gets [`unanswered_refusal`]. All URLs are
/// [redacted](Redacted).
pub fn moved_refusal(url: &str, https: &str, to: Option<&str>) -> String {
    match to.filter(|to| redirect_is_move(https, to) && !redirect_is_same_repo(https, to)) {
        Some(to) if is_ssh(url) => format!(
            "{} is an ssh URL, and its https equivalent {} redirects to {}: the repo has moved; \
             update the remote to the new URL",
            Redacted(url),
            Redacted(https),
            Redacted(to)
        ),
        Some(to) => format!(
            "{} redirects to {}: the repo has moved; update the remote to the new URL",
            Redacted(url),
            Redacted(to)
        ),
        None => unanswered_refusal(url, https),
    }
}

/// The refusal for a git remote `url` whose https URL `https` (`url` itself, or an ssh URL's
/// [`https_equivalent`]) didn't answer the anonymous probe: private, not a git repo, or not there.
/// For an ssh URL it is [`Shareability::refusal`]'s; for an https one, its own wording.
pub fn unanswered_refusal(url: &str, https: &str) -> String {
    if is_ssh(url) {
        return Shareability::Ssh { https: Some(https.to_owned()) }.refusal(url).unwrap_or_default();
    }
    format!(
        "{} doesn't answer git without credentials (the repo is private, or isn't there); make it \
         public, or publish a URL anyone can fetch",
        Redacted(url)
    )
}

/// Whether `url` is an ssh remote ([`Shareability::Ssh`], or one with credentials in it).
fn is_ssh(url: &str) -> bool {
    Parts::of(unquote(url)).is_some_and(|p| p.ssh)
}

/// Whether a redirect from `https` to `to` stays at the same repo: `to` is a shareable https URL
/// with the same [`normalize_repo_url`] key, as when GitLab sends `…/g/p/info/refs` to the `.git`
/// form `…/g/p.git/info/refs`. The probe then asks `to` once more ([`crate::probe::next_step`]),
/// and if it answers, publishes the URL it started from (git follows such a redirect itself).
pub fn redirect_is_same_repo(https: &str, to: &str) -> bool {
    let to = unquote(to);
    strip_prefix_ignore_case(to, "https://").is_some()
        && shareability(to) == Shareability::Shareable
        && same_repo(https, to)
}

/// Path segments of a sign-in page, compared case-insensitively: a redirect to a path with one of
/// them is a forge asking for credentials, not a moved repo. (A repo really named `auth` that
/// moved is then refused as private: the safe side.)
pub const SIGN_IN_SEGMENTS: [&str; 13] = [
    "login", "log_in", "log-in", "signin", "sign_in", "sign-in", "session", "sessions", "auth", "oauth", "sso",
    "saml", "cas",
];

/// Whether a redirect from `https` (an ssh URL's https equivalent) to `to` is a moved repo, as
/// [`moved_refusal`] decides: same host (ignoring case, a `www.` and the port), another non-empty
/// path, no query or userinfo, and no [sign-in segment](SIGN_IN_SEGMENTS) in the path.
pub fn redirect_is_move(https: &str, to: &str) -> bool {
    let (Some(from), Some(to_parts)) = (Parts::of(https), Parts::of(to)) else { return false };
    let host = |p: &Parts| {
        let h = split_port(p.host_port).0.to_ascii_lowercase();
        h.strip_prefix("www.").map(str::to_owned).unwrap_or(h)
    };
    let path = |p: &Parts| p.path.trim_matches('/').to_owned();
    let to_path = path(&to_parts);
    host(&from) == host(&to_parts)
        && !host(&from).is_empty()
        && !to_parts.query
        && to_parts.userinfo.is_none()
        && !to_path.is_empty()
        && to_path != path(&from)
        && !to_path.split('/').any(|seg| SIGN_IN_SEGMENTS.iter().any(|s| seg.eq_ignore_ascii_case(s)))
}

/// A URL for messages and logs, with anything that may be a credential replaced by `***`: the
/// userinfo of any URL but an ssh one, an ssh URL's password (its user name stays), and a query
/// string (`?***`). Everything else is shown as written. `https://alice:TOKEN@host/x?t=1` displays
/// as `https://***@host/x?***`; `git:pw@host:x` as `git:***@host:x`.
#[derive(Debug, Clone, Copy)]
pub struct Redacted<'a>(pub &'a str);

impl std::fmt::Display for Redacted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let raw = self.0.trim();
        let url = unquote(raw);
        // The quotes stay, around the redacted URL.
        let quote = if url.len() < raw.len() { &raw[..1] } else { "" };
        f.write_str(quote)?;
        redact(f, url)?;
        f.write_str(quote)
    }
}

/// [`Redacted`]'s work on an unquoted URL. A URL [`Parts`] takes apart cleanly keeps an ssh user
/// name; anything else (a local or `file://` URL, a remote helper's address, a port that isn't a
/// number, nothing parseable) loses everything up to its last `@`, when it has a scheme or the
/// `@` comes before any `/`. A query string always becomes `?***`.
fn redact(f: &mut std::fmt::Formatter<'_>, url: &str) -> std::fmt::Result {
    if is_helper(url)
        && let Some((transport, address)) = url.split_once("::")
    {
        write!(f, "{transport}::")?;
        return redact(f, address);
    }
    let clean = Parts::of(url)
        .filter(|p| split_port(p.host_port).1.is_none_or(|port| port.bytes().all(|b| b.is_ascii_digit())));
    let tail = match clean {
        Some(p) => {
            f.write_str(p.prefix)?;
            if let Some(userinfo) = p.userinfo {
                match userinfo.split_once(':') {
                    Some((user, _)) if p.ssh => write!(f, "{user}:***@")?,
                    None if p.ssh => write!(f, "{userinfo}@")?,
                    _ => f.write_str("***@")?,
                }
            }
            &url[p.after_userinfo..]
        }
        None => {
            let start = scheme_end(url).unwrap_or(0);
            let end = url[start..].find(['?', '#']).map_or(url.len(), |q| start + q);
            match url[start..end].rfind('@').map(|at| start + at) {
                Some(at) if start > 0 || !url[..at].contains('/') => {
                    write!(f, "{}***", &url[..start])?;
                    &url[at..]
                }
                _ => url,
            }
        }
    };
    match tail.find('?') {
        Some(q) => write!(f, "{}?***", &tail[..q]),
        None => f.write_str(tail),
    }
}

/// A remote URL taken apart for [`shareability`] and [`Redacted`]: `None` for an empty URL, a
/// remote helper, and anything [`normalize_repo_url`] keys as local.
struct Parts<'a> {
    /// The URL's scheme (`None`: scp-like).
    scheme: Option<&'a str>,
    /// Everything before the userinfo (`https://`, or nothing for scp-like).
    prefix: &'a str,
    /// The userinfo, when there is an `@` in the authority.
    userinfo: Option<&'a str>,
    /// Where the host starts in the trimmed URL.
    after_userinfo: usize,
    host_port: &'a str,
    /// The path, without the `/` (or scp `:`) that starts it.
    path: &'a str,
    /// A query string anywhere after the userinfo.
    query: bool,
    /// An ssh remote: an ssh scheme, or scp-like.
    ssh: bool,
}

impl<'a> Parts<'a> {
    fn of(url: &'a str) -> Option<Parts<'a>> {
        let url = url.trim();
        if url.is_empty() || is_helper(url) || is_local(url) {
            return None;
        }
        let (scheme, start, authority, path) = match scheme_end(url) {
            Some(end) => {
                let rest = &url[end..];
                let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
                (Some(&url[..end - 3]), end, authority, path)
            }
            None => {
                let (authority, path) = scp(url)?;
                (None, 0, authority, path)
            }
        };
        let (userinfo, host_port) = match authority.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, authority),
        };
        let after_userinfo = start + userinfo.map_or(0, |u| u.len() + 1);
        let ssh = scheme.is_none_or(|s| ["ssh", "git+ssh", "ssh+git"].iter().any(|k| s.eq_ignore_ascii_case(k)));
        Some(Parts {
            scheme,
            prefix: &url[..start],
            userinfo,
            after_userinfo,
            host_port: before_query(host_port),
            path: before_query(path),
            query: url[after_userinfo..].contains('?'),
            ssh,
        })
    }
}

/// An ssh remote's https equivalent, from its authority's host and port and its path.
fn https_for(host_port: &str, path: &str) -> Option<String> {
    let host = split_port(host_port).0.to_ascii_lowercase();
    let path = path.trim_start_matches('/').trim_end_matches('/');
    if host.is_empty() || path.is_empty() || path.starts_with('~') {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

/// Whether git reads `url` as a local repo (the same rule as [`normalize_repo_url`]'s keys).
fn is_local(url: &str) -> bool {
    normalize_repo_url(url).starts_with(LOCAL)
}

/// Git's remote-helper syntax `<transport>::<address>` (`hg::…`, `ext::…`), which runs a helper
/// program rather than fetching: a transport name of letters, digits, `+`, `-` and `.`.
fn is_helper(url: &str) -> bool {
    url.split_once("::").is_some_and(|(t, _)| {
        !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    })
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

    #[test]
    fn shareability_of_each_form() {
        use Shareability::*;
        let ssh = |h: &str| Ssh { https: Some(h.to_owned()) };
        let table: Vec<(&str, Shareability)> = vec![
            // Shareable as written.
            ("https://github.com/x/y", Shareable),
            ("https://github.com/x/y.git", Shareable),
            ("  HTTPS://GitHub.com/x/y/  ", Shareable),
            ("http://host.org:8080/x", Shareable),
            ("git://host.org/x.git", Unsupported),
            ("https://github.com/x/repo@v1", Shareable),
            ("https://[2001:db8::1]/x", Shareable),
            ("https://github.com/x/y#readme", Shareable),
            // Credentials.
            ("https://user:TOKEN@github.com/x/y", Credentials),
            ("https://TOKEN@github.com/x/y", Credentials),
            ("https://@github.com/x/y", Credentials),
            ("http://u:p@host/x", Credentials),
            ("git://user@host/x", Credentials),
            ("https://us@er:pass@host.org/x", Credentials),
            ("https://user:pa/ss@host.org/x", Unsupported), // a password's `/` ends the authority
            ("ssh://host:abc/r", Unsupported),
            ("https://host.org:/x", Shareable),
            ("ssh://git:pw@github.com/x/y", Credentials),
            ("git:pw@github.com:x/y.git", Credentials),
            ("https://gitlab.com/x/y.git?private_token=s3cret", Credentials),
            ("https://host.org?token=t", Credentials),
            ("git@github.com:x/y?private_token=t", Credentials),
            // Local.
            ("/srv/repos/x.git", Local),
            ("./x", Local),
            ("../x", Local),
            ("~/x", Local),
            ("file:///srv/x", Local),
            ("FILE://localhost/srv/x", Local),
            ("file://server/share/x", Local),
            ("file://user:pw@server/x", Local),
            ("github.com/x/y", Local),
            ("x", Local),
            ("git@[::1/x", Local),
            // ssh, shareable only as https.
            ("git@github.com:x/y.git", ssh("https://github.com/x/y.git")),
            ("git@GitHub.com:EinsteinToolkit/Cottonmouth", ssh("https://github.com/EinsteinToolkit/Cottonmouth")),
            ("github.com:x/y", ssh("https://github.com/x/y")),
            ("ssh://git@github.com/x/y/", ssh("https://github.com/x/y")),
            ("ssh://git@host.org:2222/x/y", ssh("https://host.org/x/y")),
            ("git+ssh://git@github.com/x/y", ssh("https://github.com/x/y")),
            ("SSH+GIT://github.com/x/y", ssh("https://github.com/x/y")),
            ("git@host.org:/srv/git/x.git", ssh("https://host.org/srv/git/x.git")),
            ("git@[::1]:x/y", ssh("https://[::1]/x/y")),
            ("ssh://git@[::1]:2222/x", ssh("https://[::1]/x")),
            ("user@host.org:~user/x", Ssh { https: None }),
            ("ssh://host.org/~user/x", Ssh { https: None }),
            ("git@github.com:", Ssh { https: None }),
            ("ssh://host.org", Ssh { https: None }),
            // Unsupported.
            ("", Unsupported),
            ("   ", Unsupported),
            ("s3://bucket/x", Unsupported),
            ("ftp://host/x", Unsupported),
            ("hg::https://host/x", Unsupported),
            ("ext::ssh -i key host %S x", Unsupported),
            ("host::x", Unsupported),
            ("https:///srv/x", Unsupported),
            ("https://", Unsupported),
        ];
        for (url, want) in table {
            assert_eq!(shareability(url), want, "{url:?}");
        }
    }

    #[test]
    fn https_equivalents() {
        assert_eq!(https_equivalent("git@github.com:x/y.git").as_deref(), Some("https://github.com/x/y.git"));
        assert_eq!(https_equivalent("https://github.com/x/y"), None, "not ssh");
        assert_eq!(https_equivalent("git:pw@github.com:x/y"), None, "credentials");
        assert_eq!(https_equivalent("/srv/x"), None);
    }

    #[test]
    fn redaction() {
        let table = [
            ("https://alice:TOKEN@host/x?t=1", "https://***@host/x?***"),
            ("https://TOKEN@github.com/x/y", "https://***@github.com/x/y"),
            ("https://us@er:pass@host.org/x", "https://***@host.org/x"),
            ("https://gitlab.com/x/y.git?private_token=s3cret#f", "https://gitlab.com/x/y.git?***"),
            ("https://host.org?token=t", "https://host.org?***"),
            ("git://user@host/x", "git://***@host/x"),
            ("ssh://git:pw@github.com/x/y", "ssh://git:***@github.com/x/y"),
            ("git:pw@github.com:x/y.git", "git:***@github.com:x/y.git"),
            ("git@github.com:x/y.git", "git@github.com:x/y.git"),
            ("ssh://git@host:2222/x", "ssh://git@host:2222/x"),
            ("https://github.com/x/y", "https://github.com/x/y"),
            ("  https://a:b@h/x  ", "https://***@h/x"),
            ("/srv/x", "/srv/x"),
            ("", ""),
        ];
        for (url, shown) in table {
            assert_eq!(Redacted(url).to_string(), shown, "{url:?}");
        }
        // No refusal shows the secret.
        for url in ["https://alice:s3cret@host/x", "https://s3cret@host/x", "https://host/x?k=s3cret", "git:s3cret@host:x"] {
            let why = shareability(url).refusal(url).unwrap();
            assert!(!why.contains("s3cret"), "{why}");
        }
    }

    #[test]
    fn moved_refusals() {
        let why = moved_refusal("git@github.com:old/y.git", "https://github.com/old/y.git", Some("https://github.com/new/y.git"));
        assert!(why.contains("redirects to https://github.com/new/y.git") && why.contains("update the remote"), "{why}");
        // No location seen, a sign-in page, another host: the repo is private as far as anyone
        // can tell, so the plain refusal.
        for to in [
            None,
            Some("https://gitlab.example.org/users/sign_in"),
            Some("https://git.example.org/login?redirect=%2Fx%2Fy"),
            Some("https://sso.example.org/x/y"),
            Some("https://tok@git.example.org/z?k=s3cret"),
        ] {
            let host = to.map_or("git.example.org", |t| if t.contains("gitlab") { "gitlab.example.org" } else { "git.example.org" });
            let why = moved_refusal(&format!("git@{host}:x/y"), &format!("https://{host}/x/y"), to);
            assert!(why.contains("doesn't answer without credentials") && !why.contains("moved"), "{to:?}: {why}");
            assert!(!why.contains("s3cret") && !why.contains("tok@"), "{why}");
        }
    }

    #[test]
    fn which_redirects_are_moves() {
        let from = "https://github.com/old/y.git";
        assert!(redirect_is_move(from, "https://github.com/new/y.git"));
        assert!(redirect_is_move(from, "https://www.GitHub.com/new/y"));
        assert!(redirect_is_move(from, "https://github.com:443/old/z"));
        assert!(!redirect_is_move(from, "https://github.com/old/y.git/"), "the same path");
        assert!(!redirect_is_move(from, "https://github.com/"), "no path");
        assert!(!redirect_is_move(from, "https://gitlab.com/new/y"), "another host");
        assert!(!redirect_is_move(from, "https://github.com/login"));
        assert!(!redirect_is_move(from, "https://github.com/session/new"));
        assert!(!redirect_is_move(from, "https://github.com/x/SIGN_IN"));
        assert!(!redirect_is_move(from, "https://github.com/new/y?return_to=1"), "a query");
        assert!(!redirect_is_move(from, "https://u@github.com/new/y"), "userinfo");
        assert!(!redirect_is_move(from, "/new/y"), "not absolute");
    }

    #[test]
    fn refusals_name_the_fix() {
        let r = |u: &str| shareability(u).refusal(u);
        assert_eq!(r("https://github.com/x/y"), None);
        assert!(r("https://t@github.com/x/y").unwrap().contains("remove it from the URL"));
        assert!(r("/srv/x").unwrap().contains("local path"));
        let why = r("git@github.com:x/y").unwrap();
        assert!(why.contains("https://github.com/x/y") && why.contains("without credentials"), "{why}");
        assert!(r("s3://b/x").unwrap().contains("https URL"));
    }

    #[test]
    fn quoted_urls() {
        assert_eq!(unquote(r#"  "https://github.com/x/y"  "#), "https://github.com/x/y");
        assert_eq!(unquote("'git@github.com:x/y'"), "git@github.com:x/y");
        assert_eq!(unquote(r#""https://x"#), r#""https://x"#, "unbalanced");
        assert_eq!(shareability(r#""https://github.com/x/y.git""#), Shareability::Shareable);
        assert_eq!(shareability(r#""https://TOKEN@github.com/o/x.git""#), Shareability::Credentials);
        assert_eq!(download_shareability("'https://zenodo.org/x.h5?download=1'"), Shareability::Shareable);
    }

    #[test]
    fn downloads() {
        use Shareability::*;
        for (url, want) in [
            ("https://zenodo.org/records/1/files/x.h5?download=1", Shareable),
            ("https://github.com/o/r/raw/main/x.tgz?raw=true", Shareable),
            ("http://host.org/x.tgz", Shareable),
            ("ftp://ftp.host.org/pub/x.tgz", Shareable),
            ("https://host.org/x.tgz#sha256", Shareable),
            ("https://gitlab.com/api/v4/projects/1/repository/archive?private_token=glpat-s3cret", Credentials),
            ("https://b.s3.amazonaws.com/x?X-Amz-Algorithm=AWS4&X-Amz-Credential=a&X-Amz-Signature=s3cret", Credentials),
            ("https://storage.googleapis.com/b/x?X-Goog-Signature=s3cret", Credentials),
            ("https://acct.blob.core.windows.net/c/x?sv=1&sig=s3cret", Credentials),
            ("https://host/x?Expires=1&Signature=s3cret", Credentials),
            ("https://host/x?access_token=s3cret", Credentials),
            ("https://host/x?apikey=s3cret", Credentials),
            ("https://host/x?key=s3cret", Credentials),
            ("https://host/x#access_token=s3cret", Credentials),
            ("https://u:s3cret@host/x", Credentials),
            ("https://s3cret@host/x", Credentials),
            ("svn+ssh://u:s3cret@host/x", Credentials),
            ("ftp://anon:pw@host/x", Credentials),
            ("svn+ssh://svn.host.org/repo", Unsupported),
            ("ssh://git@host/x.tgz", Unsupported),
            ("git@host:x.tgz", Unsupported),
            ("s3://bucket/x", Unsupported),
            ("https:///x", Unsupported),
            ("https://u:pa/ss@host/x", Unsupported),
            ("/data/x.h5", Local),
            ("file:///data/x.h5", Local),
        ] {
            assert_eq!(download_shareability(url), want, "{url}");
        }
        assert!(!is_credential_key("download") && !is_credential_key("raw") && !is_credential_key("ref") && !is_credential_key("dl"));
        let why = download_shareability("https://host/x?private_token=s3cret").download_refusal("https://host/x?private_token=s3cret").unwrap();
        assert!(why.contains("`private_token` in its query") && !why.contains("s3cret"), "{why}");
        assert!(Shareability::Unsupported.download_refusal("svn+ssh://h/x").unwrap().contains("ssh needs an account"));
    }

    /// What a review found shown in clear, and what `Redacted` shows now.
    #[test]
    fn redaction_of_what_does_not_parse() {
        for (url, shown) in [
            ("file://user:s3cret@server/x.git", "file://***@server/x.git"),
            ("https://user:s3c/ret@github.com/o/x", "https://***@github.com/o/x"),
            ("hg::https://u:s3cret@h/x", "hg::https://***@h/x"),
            (r#""https://s3cret@github.com/o/x.git""#, r#""https://***@github.com/o/x.git""#),
            ("'git:s3cret@host:x'", "'git:***@host:x'"),
            ("foo://u:s3cret@h/x?k=s3cret", "foo://***@h/x?***"),
            ("/home/u/repo@v1", "/home/u/repo@v1"),
            ("https://github.com/o/repo@v1", "https://github.com/o/repo@v1"),
            ("git@github.com:o/x", "git@github.com:o/x"),
        ] {
            let got = Redacted(url).to_string();
            assert_eq!(got, shown, "{url}");
            assert!(!got.contains("s3c"), "{url}: {got}");
        }
    }

    #[test]
    fn same_repo_redirects() {
        assert!(redirect_is_same_repo("https://gitlab.com/g/p", "https://gitlab.com/g/p.git"));
        assert!(redirect_is_same_repo("https://gitlab.com/g/p.git", "https://GitLab.com/g/p/"));
        assert!(!redirect_is_same_repo("https://gitlab.com/g/p", "http://gitlab.com/g/p.git"), "not https");
        assert!(!redirect_is_same_repo("https://gitlab.com/g/p", "https://gitlab.com/g/q.git"));
        assert!(!redirect_is_same_repo("https://gitlab.com/g/p", "https://t@gitlab.com/g/p.git"));
        assert!(!redirect_is_same_repo("https://gitlab.com/g/p", "https://gitlab.com/users/sign_in"));
        // A same-repo redirect isn't a move (when asking once more didn't answer either).
        let why = moved_refusal("git@gitlab.com:g/p", "https://gitlab.com/g/p", Some("https://gitlab.com/g/p.git"));
        assert!(!why.contains("moved") && why.contains("without credentials"), "{why}");
    }

    #[test]
    fn credentials_named() {
        let r = |u: &str| shareability(u).refusal(u).unwrap();
        let d = |u: &str| download_shareability(u).download_refusal(u).unwrap();
        assert!(r("https://alice:s3cret@host/x").contains("carries a user name and password,"));
        assert!(r("https://s3cret@host/x").contains("carries a user name or token,"));
        assert!(r("https://:s3cret@host/x").contains("carries a password,"));
        assert!(r("git:s3cret@host:x").contains("carries a user name and password,"));
        assert!(r("https://host/x?private_token=s3cret").contains("carries `private_token` in its query,"));
        assert!(r("https://host/x?ref=main").contains("carries a query string,"));
        assert!(d("https://host/x?download=1&X-Amz-Signature=s3cret").contains("carries `X-Amz-Signature` in its query,"));
        assert!(d("https://alice:s3cret@host/x").contains("carries a user name and password,"));
        assert!(d("https://alice@host/x?token=s3cret").contains("carries a user name or token and `token` in its query,"));
        for m in [r("https://alice:s3cret@host/x?k=s3cret"), d("https://host/x?sig=s3cret")] {
            assert!(!m.contains("s3cret") && m.ends_with("remove it from the URL"), "{m}");
        }
        assert!(private_address_refusal("https://10.0.0.5/x", "10.0.0.5").contains("10.0.0.5 is not a public address"));
        assert!(unreachable_refusal("https://t@h/x", "connection refused").starts_with("https://***@h/x can't be checked"));
    }
}
