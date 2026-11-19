//! The anonymous probe, decided here so the Cactarium and cactup decide the same way: whether an
//! ssh remote's https equivalent ([`crate::url::https_equivalent`]) answers a stranger, so the
//! https URL can be published in its place. This module is pure; each side does only the I/O.
//!
//! # Running it
//!
//! 1. Ask `GET` [`probe_url`]`(https)` with no credentials of any kind: no credential helper, no
//!    `.netrc`, no cookies, no extra headers, no `insteadOf` rewriting, no prompts, no keys. Send a
//!    `User-Agent` starting with [`USER_AGENT_PREFIX`] (`git/`), since forges answer git clients
//!    and browsers differently. Follow no redirect. Connect only to an address
//!    [`is_public_address`] accepts, and to the address you checked (resolve once, then pin it).
//! 2. Turn the response into an [`Outcome`]: [`is_answer`] on the status and `Content-Type` gives
//!    [`Outcome::Answer`]; a 3xx gives [`Outcome::Redirect`] with
//!    [`redirect_target`]`(request, Location)`; anything else (an error, a 401 or 403, no
//!    connection) is [`Outcome::Other`].
//! 3. [`next_step`] says what to do: publish an https URL, ask once more at the target of a
//!    redirect to the same repo (GitLab's `.git` form), or refuse with a message to show.

use crate::url::{Shareability, moved_refusal, redirect_is_same_repo, unquote};
use std::net::IpAddr;

/// What the probe asks for after the repo URL: git's smart-HTTP ref advertisement, the first thing
/// `git ls-remote` asks.
pub const SUFFIX: &str = "/info/refs?service=git-upload-pack";

/// The `Content-Type` of git's ref advertisement.
pub const ADVERTISEMENT: &str = "application/x-git-upload-pack-advertisement";

/// How the probe's `User-Agent` starts (`git/cactup-1.2.3`, `git/2 (cactarium)`).
pub const USER_AGENT_PREFIX: &str = "git/";

/// The URL to ask: `https`, its trailing `/`s dropped, and [`SUFFIX`].
pub fn probe_url(https: &str) -> String {
    format!("{}{SUFFIX}", unquote(https).trim_end_matches('/'))
}

/// Whether a response is an answer: status exactly 200, and a `Content-Type` that is
/// [`ADVERTISEMENT`] (compared case-insensitively, before any `;` parameters).
pub fn is_answer(status: u16, content_type: Option<&str>) -> bool {
    status == 200
        && content_type.is_some_and(|t| t.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case(ADVERTISEMENT))
}

/// Where a redirect from `request_url` (a [`probe_url`]) to `location` (its `Location` header)
/// points, as a repo URL: resolved against the request when relative (`/new/y/info/refs?…`,
/// `//host/…`, `y.git/info/refs?…`), then less [`SUFFIX`] (or a bare `/info/refs`) at its end.
/// `None` for an empty location or a request URL without a scheme.
pub fn redirect_target(request_url: &str, location: &str) -> Option<String> {
    let loc = location.trim();
    let req = request_url.trim();
    if loc.is_empty() {
        return None;
    }
    let scheme_len = req.find("://")?;
    let after = scheme_len + 3;
    let authority_end = req[after..].find(['/', '?', '#']).map_or(req.len(), |i| after + i);
    let origin = &req[..authority_end];
    let abs = if loc.contains("://") {
        loc.to_owned()
    } else if let Some(rest) = loc.strip_prefix("//") {
        format!("{}//{rest}", &req[..scheme_len + 1])
    } else if loc.starts_with('/') {
        format!("{origin}{loc}")
    } else {
        // Relative to the request's directory: its path up to the last `/`.
        let path_end = req[authority_end..].find(['?', '#']).map_or(req.len(), |i| authority_end + i);
        let dir = req[authority_end..path_end].rfind('/').map_or(authority_end, |i| authority_end + i);
        format!("{}/{loc}", &req[..dir])
    };
    let repo = abs.strip_suffix(SUFFIX).or_else(|| abs.strip_suffix("/info/refs")).unwrap_or(&abs);
    Some(repo.to_owned())
}

/// Whether the probe may connect to `ip`: only globally routable addresses. Refused: private,
/// loopback, link-local, unspecified, broadcast, multicast and documentation ranges; 0/8,
/// 100.64/10 (shared), 192.0.0/24, 198.18/15 (benchmarking), 240/4; IPv6 unique-local, link-local,
/// NAT64 (64:ff9b::/96 and /48), documentation, and v4-mapped or v4-compatible forms of a refused
/// IPv4 address.
pub fn is_public_address(ip: IpAddr) -> bool {
    !private(ip)
}

fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || v.is_documentation()
                || o[0] == 0
                || o[0] == 100 && (o[1] & 0xc0) == 64
                || o[0] == 192 && o[1] == 0 && o[2] == 0
                || o[0] == 198 && (o[1] & 0xfe) == 18
                || o[0] >= 240
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || s[0] == 0x0064 && s[1] == 0xff9b
                || s[0] == 0x2001 && s[1] == 0x0db8
                || v.to_ipv4_mapped().is_some_and(|m| private(IpAddr::V4(m)))
                || v.to_ipv4().is_some_and(|m| private(IpAddr::V4(m)))
        }
    }
}

/// What one probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// [`is_answer`].
    Answer,
    /// A 3xx, with [`redirect_target`] of its `Location` when it had one.
    Redirect(Option<String>),
    /// Anything else: an error status (401, 403, 404, …), a 200 that isn't the advertisement, a
    /// refused address, no connection.
    Other,
}

/// What to do next ([`next_step`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Publish this https URL in place of the ssh one.
    Publish(String),
    /// Probe this URL (a redirect's target at the same repo), once, then call [`next_step`] again
    /// with `first: false`.
    Probe(String),
    /// Refuse, showing this message (URLs redacted). A caller may append its own detail.
    Refuse(String),
}

/// The decision after probing `asked` for the ssh remote `url`. `first` is true for the probe of
/// `url`'s https equivalent, false for the one more probe a [`Step::Probe`] asks for. An answer
/// publishes `asked`; on the first probe, a redirect to the same repo
/// ([`redirect_is_same_repo`]) is asked once more; any other redirect is refused by
/// [`moved_refusal`] (a move, or a sign-in page); anything else by the plain ssh refusal.
pub fn next_step(url: &str, asked: &str, outcome: &Outcome, first: bool) -> Step {
    match outcome {
        Outcome::Answer => Step::Publish(asked.to_owned()),
        Outcome::Redirect(Some(to)) if first && redirect_is_same_repo(asked, to) => Step::Probe(to.clone()),
        Outcome::Redirect(to) => Step::Refuse(moved_refusal(url, asked, to.as_deref())),
        Outcome::Other => {
            Step::Refuse(Shareability::Ssh { https: Some(asked.to_owned()) }.refusal(url).unwrap_or_default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_urls_and_answers() {
        assert_eq!(probe_url("https://github.com/x/y.git/"), "https://github.com/x/y.git/info/refs?service=git-upload-pack");
        assert!(is_answer(200, Some("application/x-git-upload-pack-advertisement")));
        assert!(is_answer(200, Some("Application/X-Git-Upload-Pack-Advertisement; charset=utf-8")));
        assert!(!is_answer(203, Some(ADVERTISEMENT)), "exactly 200");
        assert!(!is_answer(200, Some("text/html")));
        assert!(!is_answer(200, None));
        assert!(!is_answer(401, Some(ADVERTISEMENT)));
    }

    #[test]
    fn redirect_targets() {
        let req = "https://gitlab.com/g/p/info/refs?service=git-upload-pack";
        let t = |loc: &str| redirect_target(req, loc);
        assert_eq!(t("https://gitlab.com/g/p.git/info/refs?service=git-upload-pack").as_deref(), Some("https://gitlab.com/g/p.git"));
        assert_eq!(t("/g/p.git/info/refs?service=git-upload-pack").as_deref(), Some("https://gitlab.com/g/p.git"));
        assert_eq!(t("//other.org/g/p/info/refs").as_deref(), Some("https://other.org/g/p"));
        assert_eq!(t("p.git/info/refs?service=git-upload-pack").as_deref(), Some("https://gitlab.com/g/p/info/p.git"));
        assert_eq!(t("https://gitlab.com/users/sign_in").as_deref(), Some("https://gitlab.com/users/sign_in"));
        assert_eq!(t("  "), None);
        assert_eq!(redirect_target("no-scheme", "/x"), None);
    }

    #[test]
    fn public_addresses() {
        for ip in ["140.82.112.3", "2606:4700::6810:84e5"] {
            assert!(is_public_address(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["10.0.0.1", "127.0.0.1", "169.254.1.1", "192.168.1.1", "100.64.0.1", "0.1.2.3", "::1", "fe80::1", "fd00::1", "::ffff:10.0.0.1", "64:ff9b::a00:1"] {
            assert!(!is_public_address(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn steps() {
        let ssh = "git@gitlab.com:g/p";
        let https = "https://gitlab.com/g/p";
        assert_eq!(next_step(ssh, https, &Outcome::Answer, true), Step::Publish(https.into()));
        // GitLab's `.git` redirect: asked once more, and published when it answers.
        let dotgit = "https://gitlab.com/g/p.git";
        assert_eq!(next_step(ssh, https, &Outcome::Redirect(Some(dotgit.into())), true), Step::Probe(dotgit.into()));
        assert_eq!(next_step(ssh, dotgit, &Outcome::Answer, false), Step::Publish(dotgit.into()));
        // Asked once more and redirected again: refused, not a move.
        match next_step(ssh, dotgit, &Outcome::Redirect(Some(https.into())), false) {
            Step::Refuse(m) => assert!(m.contains("doesn't answer without credentials") && !m.contains("moved"), "{m}"),
            other => panic!("{other:?}"),
        }
        // A move, a sign-in page, nothing.
        let gh = "git@github.com:old/y";
        let moved = next_step(gh, "https://github.com/old/y", &Outcome::Redirect(Some("https://github.com/new/y".into())), true);
        assert!(matches!(&moved, Step::Refuse(m) if m.contains("has moved")), "{moved:?}");
        let login = next_step(ssh, https, &Outcome::Redirect(Some("https://gitlab.com/users/sign_in".into())), true);
        assert!(matches!(&login, Step::Refuse(m) if !m.contains("moved")), "{login:?}");
        assert!(matches!(next_step(ssh, https, &Outcome::Other, true), Step::Refuse(m) if m.contains("without credentials")));
    }
}
