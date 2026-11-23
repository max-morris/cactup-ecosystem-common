//! The anonymous probe, decided here so the Cactarium and cactup decide the same way. Every git
//! remote is probed before it is published: an https or http URL must answer a stranger itself,
//! and an ssh URL is published as its https equivalent ([`crate::url::https_equivalent`]) only if
//! that answers. (`git://` is refused by [`crate::url::shareability`]: no HTTP to probe.) This
//! module is pure; each side does only the I/O.
//!
//! # Running it
//!
//! 0. [`start`]`(url)` refuses a URL by its form, or names the `candidate` to probe and publish.
//! 1. Ask `GET` [`probe_url`]`(asked)` (first `asked` = `candidate`) with no credentials of any kind: no credential helper, no
//!    `.netrc`, no cookies, no extra headers, no `insteadOf` rewriting, no prompts, no keys. Send a
//!    `User-Agent` starting with [`USER_AGENT_PREFIX`] (`git/`), since forges answer git clients
//!    and browsers differently. Follow no redirect. Connect only to an address
//!    [`is_public_address`] accepts, and to the address you checked (resolve once, then pin it).
//! 2. Turn the response into an [`Outcome`] with [`outcome`]`(request, status, Content-Type,
//!    Location)`. When there was no response at all, or the address was refused, report it with
//!    [`crate::url::unreachable_refusal`] or [`crate::url::private_address_refusal`] instead.
//! 3. [`next_step`] says what to do: publish the candidate, ask once more at the target of a
//!    redirect to the same repo (GitLab's `.git` form), or refuse with a message to show.
//!
//! Every probe connects only to addresses [`is_public_address`] accepts, http as well as https.

use crate::url::{Shareability, busy_refusal, moved_refusal, redirect_is_same_repo, shareability, unanswered_refusal, unquote};
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
/// site-local (fec0::/10, deprecated), documentation, Teredo (2001::/32), the local-use NAT64
/// prefix 64:ff9b:1::/48 (where the IPv4 address sits depends on the operator's prefix length, so
/// it can't be judged), and v4-mapped or v4-compatible forms of a refused IPv4 address.
///
/// An address that embeds an IPv4 address at a fixed place is judged by that address: the
/// well-known NAT64 prefix 64:ff9b::/96 (what DNS64 synthesizes for an IPv4-only host such as
/// github.com, in its last 32 bits) and 6to4 2002::/16 (in bits 16 to 48).
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
                || (s[0] & 0xffc0) == 0xfec0
                || s[0] == 0x2001 && s[1] == 0x0db8
                || s[0] == 0x2001 && s[1] == 0x0000
                || s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0x0001
                || s[0] == 0x0064 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] && private(IpAddr::V4(v4(s[6], s[7])))
                || s[0] == 0x0064 && s[1] == 0xff9b && s[2] != 0x0001 && s[2..6] != [0, 0, 0, 0]
                || s[0] == 0x2002 && private(IpAddr::V4(v4(s[1], s[2])))
                || v.to_ipv4_mapped().is_some_and(|m| private(IpAddr::V4(m)))
                || v.to_ipv4().is_some_and(|m| private(IpAddr::V4(m)))
        }
    }
}

/// The IPv4 address in two 16-bit segments.
fn v4(hi: u16, lo: u16) -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8)
}

/// Whether a status means the host is busy or failing rather than refusing a stranger: 408, 429
/// or any 5xx. Nothing is learned about the repo; ask again later.
pub fn is_busy(status: u16) -> bool {
    status == 408 || status == 429 || (500..=599).contains(&status)
}

/// The [`Outcome`] of an HTTP response to `request_url` (a [`probe_url`]): [`is_answer`] →
/// [`Outcome::Answer`]; a 3xx → [`Outcome::Redirect`] with [`redirect_target`] of `location`;
/// [`is_busy`] → [`Outcome::Busy`]; anything else → [`Outcome::Other`].
pub fn outcome(request_url: &str, status: u16, content_type: Option<&str>, location: Option<&str>) -> Outcome {
    if is_answer(status, content_type) {
        Outcome::Answer
    } else if (300..=399).contains(&status) {
        Outcome::Redirect(location.and_then(|l| redirect_target(request_url, l)))
    } else if is_busy(status) {
        Outcome::Busy(status)
    } else {
        Outcome::Other
    }
}

/// What one probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// [`is_answer`].
    Answer,
    /// A 3xx, with [`redirect_target`] of its `Location` when it had one.
    Redirect(Option<String>),
    /// A status saying the host is busy or failing ([`is_busy`]: 408, 429, 5xx), with it.
    Busy(u16),
    /// Anything else the host answered: an error status (401, 403, 404, …), a 200 that isn't the
    /// advertisement. (No answer at all, or a refused address, is the caller's to report:
    /// [`crate::url::unreachable_refusal`], [`crate::url::private_address_refusal`].)
    Other,
}

/// How checking a git remote begins ([`start`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// Probe this URL (the remote itself when https or http, an ssh remote's https equivalent);
    /// it is what gets published if it answers.
    Probe { candidate: String },
    /// Refused by its form ([`shareability`]): credentials, a local path, git://, an ssh URL with
    /// no https equivalent, anything else. The message (URL redacted) to show.
    Refuse(String),
}

/// How checking the git remote `url` begins, by its form ([`shareability`]).
pub fn start(url: &str) -> Start {
    match shareability(url) {
        Shareability::Shareable => Start::Probe { candidate: unquote(url).to_owned() },
        Shareability::Ssh { https: Some(https) } => Start::Probe { candidate: https },
        s => Start::Refuse(s.refusal(url).unwrap_or_default()),
    }
}

/// What to do next ([`next_step`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Publish this URL (the candidate) for the remote.
    Publish(String),
    /// Probe this URL (a redirect's target at the same repo) once more, then call [`next_step`]
    /// again with it as `asked`.
    Probe(String),
    /// Refuse, showing this message (URLs redacted). A caller may append its own detail.
    Refuse(String),
}

/// The decision after probing `asked` for the git remote `url`, whose [`start`] named `candidate`.
/// The first probe asks the candidate itself; a [`Step::Probe`] asks once more elsewhere.
///
/// - An answer publishes the candidate, also when it came from asking once more: the target of a
///   same-repo redirect has the candidate's repo key, and git follows that redirect itself.
/// - On the first probe, a redirect to the same repo ([`redirect_is_same_repo`]) is asked once more.
/// - Any other redirect is refused by [`moved_refusal`] (a real move says so; a sign-in page or
///   another host is [`unanswered_refusal`]).
/// - A busy or failing host ([`Outcome::Busy`]) is [`busy_refusal`]: try again later.
/// - Anything else is [`unanswered_refusal`].
pub fn next_step(url: &str, candidate: &str, asked: &str, outcome: &Outcome) -> Step {
    let first = asked == candidate;
    match outcome {
        Outcome::Answer => Step::Publish(candidate.to_owned()),
        Outcome::Redirect(Some(to)) if first && redirect_is_same_repo(candidate, to) => Step::Probe(to.clone()),
        Outcome::Redirect(to) => Step::Refuse(moved_refusal(url, candidate, to.as_deref())),
        Outcome::Busy(status) => Step::Refuse(busy_refusal(url, candidate, *status)),
        Outcome::Other => Step::Refuse(unanswered_refusal(url, candidate)),
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
    fn starts() {
        assert_eq!(start("https://github.com/x/y"), Start::Probe { candidate: "https://github.com/x/y".into() });
        assert_eq!(start(r#""https://github.com/x/y""#), Start::Probe { candidate: "https://github.com/x/y".into() });
        assert_eq!(start("git@github.com:x/y.git"), Start::Probe { candidate: "https://github.com/x/y.git".into() });
        for (url, says) in [
            ("https://t@github.com/x/y", "remove it"),
            ("git://github.com/x/y", "git://"),
            ("/srv/x", "local path"),
            ("user@host:~u/x", "no https"),
        ] {
            assert!(matches!(start(url), Start::Refuse(m) if m.contains(says)), "{url}");
        }
    }

    #[test]
    fn steps() {
        // An ssh remote, probed as its https equivalent.
        let ssh = "git@gitlab.com:g/p";
        let https = "https://gitlab.com/g/p";
        assert_eq!(next_step(ssh, https, https, &Outcome::Answer), Step::Publish(https.into()));
        // GitLab's `.git` redirect: asked once more; the candidate is published when it answers.
        let dotgit = "https://gitlab.com/g/p.git";
        assert_eq!(next_step(ssh, https, https, &Outcome::Redirect(Some(dotgit.into()))), Step::Probe(dotgit.into()));
        assert_eq!(next_step(ssh, https, dotgit, &Outcome::Answer), Step::Publish(https.into()));
        // Asked once more and redirected again: refused, not a move.
        match next_step(ssh, https, dotgit, &Outcome::Redirect(Some(https.into()))) {
            Step::Refuse(m) => assert!(m.contains("doesn't answer without credentials") && !m.contains("moved"), "{m}"),
            other => panic!("{other:?}"),
        }
        let gh = "git@github.com:old/y";
        let moved = next_step(gh, "https://github.com/old/y", "https://github.com/old/y", &Outcome::Redirect(Some("https://github.com/new/y".into())));
        assert!(matches!(&moved, Step::Refuse(m) if m.contains("is an ssh URL") && m.contains("has moved")), "{moved:?}");
        let login = next_step(ssh, https, https, &Outcome::Redirect(Some("https://gitlab.com/users/sign_in".into())));
        assert!(matches!(&login, Step::Refuse(m) if !m.contains("moved")), "{login:?}");
        assert!(matches!(next_step(ssh, https, https, &Outcome::Other), Step::Refuse(m) if m.contains("without credentials")));
    }

    #[test]
    fn https_remotes_are_probed_too() {
        let url = "https://github.com/me/private.git";
        assert_eq!(next_step(url, url, url, &Outcome::Answer), Step::Publish(url.into()));
        match next_step(url, url, url, &Outcome::Other) {
            Step::Refuse(m) => assert!(m.contains("doesn't answer git without credentials") && !m.contains("ssh"), "{m}"),
            other => panic!("{other:?}"),
        }
        let moved = next_step(url, url, url, &Outcome::Redirect(Some("https://github.com/them/private.git".into())));
        assert!(matches!(&moved, Step::Refuse(m) if m.contains("redirects to https://github.com/them/private.git") && !m.contains("ssh")), "{moved:?}");
        // GitLab's `.git` form, then an answer: the URL as written is published.
        let gl = "https://gitlab.com/g/p";
        assert_eq!(next_step(gl, gl, gl, &Outcome::Redirect(Some("https://gitlab.com/g/p.git".into()))), Step::Probe("https://gitlab.com/g/p.git".into()));
        assert_eq!(next_step(gl, gl, "https://gitlab.com/g/p.git", &Outcome::Answer), Step::Publish(gl.into()));
        // http:// redirecting to its https form is the same repo, asked once more.
        let http = "http://github.com/x/y";
        assert_eq!(next_step(http, http, http, &Outcome::Redirect(Some("https://github.com/x/y".into()))), Step::Probe("https://github.com/x/y".into()));
    }

    #[test]
    fn embedded_addresses() {
        let public = |a: &str| is_public_address(a.parse().unwrap());
        // DNS64's synthesized address for an IPv4-only host is judged by that host.
        assert!(public("64:ff9b::8c52:7004"), "github.com behind NAT64");
        assert!(!public("64:ff9b::a00:1"), "10.0.0.1 behind NAT64");
        assert!(!public("64:ff9b::7f00:1"), "127.0.0.1 behind NAT64");
        assert!(!public("64:ff9b:1::8c52:7004"), "local-use NAT64: can't be judged");
        assert!(!public("64:ff9b:5::8c52:7004"), "elsewhere in 64:ff9b::/32");
        // 6to4.
        assert!(public("2002:8c52:7004::1"));
        assert!(!public("2002:c0a8:101::1"), "192.168.1.1 in 6to4");
        assert!(!public("fec0::1"), "site-local");
        assert!(!public("2001:0:4136:e378::1"), "Teredo");
        assert!(public("2001:4860:4860::8888"));
    }

    #[test]
    fn busy_hosts() {
        let req = "https://bitbucket.org/o/r/info/refs?service=git-upload-pack";
        assert_eq!(outcome(req, 429, None, None), Outcome::Busy(429));
        assert_eq!(outcome(req, 503, Some("text/html"), None), Outcome::Busy(503));
        assert_eq!(outcome(req, 401, None, None), Outcome::Other);
        assert_eq!(outcome(req, 200, Some(ADVERTISEMENT), None), Outcome::Answer);
        assert_eq!(
            outcome(req, 301, None, Some("/o/s/info/refs?service=git-upload-pack")),
            Outcome::Redirect(Some("https://bitbucket.org/o/s".into()))
        );
        let url = "https://bitbucket.org/o/r";
        match next_step(url, url, url, &Outcome::Busy(429)) {
            Step::Refuse(m) => assert!(m.contains("429") && m.contains("try again later") && !m.contains("private"), "{m}"),
            other => panic!("{other:?}"),
        }
        let ssh = "git@bitbucket.org:o/r";
        match next_step(ssh, url, url, &Outcome::Busy(502)) {
            Step::Refuse(m) => assert!(m.contains("ssh URL") && m.contains("502") && m.contains("try again later"), "{m}"),
            other => panic!("{other:?}"),
        }
    }
}
