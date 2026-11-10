//! Cactus parameter files, as cactup reads them: which bytes are comments
//! (the `par.peg` rule), and which substitution tokens a parfile uses —
//! `@NAME@` variables, `@KNOB(name)@` / `@KNOB-OPTIONAL(name, default)@`
//! knobs and `@ENV(NAME)@` / `@ENV-OPTIONAL(…)@` environment variables —
//! lexed exactly as cactup's substitution lexes them (its `template.rs`),
//! so the registry flags precisely what `cactup sim export` declares.
//!
//! Rules: a comment is copied through untouched, so nothing in one is a
//! token; `@@` is a literal `@`, never a token; anything else starting with
//! `@` must be a well-formed token, or it is an error ("stray @").

use std::ops::Range;

use anyhow::{Context, Result, anyhow, bail};

/// What a parfile uses: each name once, in order of first use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct References {
    /// `@NAME@` variables (`UPPER_SNAKE`).
    pub vars: Vec<String>,
    /// `@KNOB(…)@` knobs.
    pub knobs: Vec<Reference>,
    /// `@ENV(…)@` environment variables.
    pub env: Vec<Reference>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    /// Every use carries a default (the `-OPTIONAL` form): the parfile works
    /// without a value. One bare use makes it required.
    pub optional: bool,
}

/// The variables and tokens `text` uses, comments skipped. A malformed token
/// or a stray `@` is an error naming its line.
pub fn references(text: &str) -> Result<References> {
    let mut refs = References::default();
    let mut done = 0;
    for span in comment_spans(text) {
        scan(text, done..span.start, &mut refs)?;
        done = span.end;
    }
    scan(text, done..text.len(), &mut refs)?;
    Ok(refs)
}

/// The byte ranges of `text` that are comments, in order, each ending before
/// its line's newline. `#` outside a `"…"` string starts a comment to the end
/// of the line. Inside a string, `#` on any line after the string's first
/// also starts one (the grammar's `stringcomment`, how `ActiveThorns` lists
/// are annotated), ending at the end of the line or at the closing quote,
/// whichever comes first. A backslash escapes the next character in a string.
pub fn comment_spans(text: &str) -> Vec<Range<usize>> {
    enum St {
        Code,
        /// Inside `"…"`; `continued` once a newline has passed.
        Str { continued: bool },
        StrComment { start: usize },
    }
    let b = text.as_bytes();
    let mut spans = Vec::new();
    let mut st = St::Code;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match st {
            St::Code => match c {
                b'#' => {
                    let end = end_of_line(b, i);
                    spans.push(i..end);
                    i = end;
                }
                b'"' => {
                    st = St::Str { continued: false };
                    i += 1;
                }
                _ => i += 1,
            },
            St::Str { continued } => match c {
                b'\\' => i += 2,
                b'"' => {
                    st = St::Code;
                    i += 1;
                }
                b'\n' => {
                    st = St::Str { continued: true };
                    i += 1;
                }
                b'#' if continued => {
                    st = St::StrComment { start: i };
                    i += 1;
                }
                _ => i += 1,
            },
            St::StrComment { start } => match c {
                b'\\' => i += 2,
                b'"' => {
                    spans.push(start..i);
                    st = St::Code;
                    i += 1;
                }
                b'\n' => {
                    spans.push(start..i);
                    st = St::Str { continued: true };
                    i += 1;
                }
                _ => i += 1,
            },
        }
    }
    if let St::StrComment { start } = st {
        spans.push(start..b.len());
    }
    spans
}

fn end_of_line(b: &[u8], i: usize) -> usize {
    b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |n| i + n)
}

fn line_of(text: &str, pos: usize) -> usize {
    text[..pos].matches('\n').count() + 1
}

fn stray_at(text: &str, pos: usize) -> anyhow::Error {
    anyhow!(
        "Stray '@' on line {}: not an '@@' escape nor a well-formed @NAME@ token (write a literal '@' as '@@')",
        line_of(text, pos)
    )
}

fn is_name_byte(b: u8) -> bool {
    b == b'_' || b.is_ascii_uppercase() || b.is_ascii_digit()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Env,
    Knob,
}

impl Source {
    fn func(self) -> &'static str {
        match self {
            Source::Env => "ENV",
            Source::Knob => "KNOB",
        }
    }

    fn is_arg_byte(self, b: u8) -> bool {
        match self {
            Source::Env => is_name_byte(b),
            Source::Knob => b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-',
        }
    }

    /// cactup's knob-name rule: lowercase letters, digits after the first
    /// character, dashes anywhere but first or last. (cactup also reserves
    /// the names of its `cactup knob` subcommands; that is a CLI matter, not
    /// a parfile one, and is left out here.)
    fn check_arg(self, arg: &str, at: usize, form: &str) -> Result<()> {
        match self {
            Source::Env if arg.is_empty() => {
                bail!("Malformed @{form}(NAME)@ token on line {at} (NAME must be UPPER_SNAKE)")
            }
            Source::Env => Ok(()),
            Source::Knob => {
                let b = arg.as_bytes();
                let ok = !b.is_empty()
                    && b[0].is_ascii_lowercase()
                    && b[b.len() - 1] != b'-'
                    && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-');
                if !ok {
                    bail!(
                        "Malformed @{form}(name)@ token on line {at}: \"{arg}\" is not a valid knob name"
                    );
                }
                Ok(())
            }
        }
    }
}

struct Cursor<'a> {
    text: &'a str,
    pos: usize,
    end: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.pos..self.end]
    }
    fn peek(&self) -> Option<u8> {
        (self.pos < self.end).then(|| self.text.as_bytes()[self.pos])
    }
    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }
    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn eat_str(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.pos += 1;
        }
    }
    fn take_while(&mut self, f: impl Fn(u8) -> bool) -> &'a str {
        let start = self.pos;
        while self.peek().is_some_and(&f) {
            self.pos += 1;
        }
        &self.text[start..self.pos]
    }
}

/// Scan `text[range]`, a comment-free stretch, recording what it uses.
fn scan(text: &str, range: Range<usize>, refs: &mut References) -> Result<()> {
    let mut cur = Cursor { text, pos: range.start, end: range.end };
    while let Some(off) = cur.rest().find('@') {
        cur.pos += off;
        let pos = cur.pos;
        let at = line_of(text, pos);
        cur.bump();
        if cur.eat(b'@') {
            continue; // `@@`: a literal `@`
        }
        let name = cur.take_while(is_name_byte);
        if cur.eat(b'@') {
            if name.is_empty() {
                return Err(stray_at(text, pos));
            }
            if !refs.vars.iter().any(|v| v == name) {
                refs.vars.push(name.to_owned());
            }
            continue;
        }
        let source = match name {
            "ENV" => Some(Source::Env),
            "KNOB" => Some(Source::Knob),
            _ => None,
        };
        if let Some(source) = source
            && matches!(cur.peek(), Some(b'(') | Some(b'-'))
        {
            computed_token(&mut cur, at, source, refs)?;
            continue;
        }
        return Err(stray_at(text, pos));
    }
    Ok(())
}

/// One `@ENV(…)@`/`@KNOB(…)@` token; `cur` sits right after the word.
fn computed_token(cur: &mut Cursor<'_>, at: usize, source: Source, refs: &mut References) -> Result<()> {
    let func = source.func();
    let optional = cur.eat(b'-');
    if optional && !cur.eat_str("OPTIONAL") {
        bail!("Malformed @{func}-…@ token on line {at}: the only suffix is -OPTIONAL (@{func}-OPTIONAL(…)@)");
    }
    let form = if optional { format!("{func}-OPTIONAL") } else { func.to_owned() };
    if !cur.eat(b'(') {
        bail!("Malformed @{form}(…)@ token on line {at}: expected '(' after {form}");
    }
    cur.skip_ws();
    let arg = cur.take_while(|b| source.is_arg_byte(b));
    source.check_arg(arg, at, &form)?;
    cur.skip_ws();
    if cur.eat(b',') {
        if !optional {
            bail!("@{func}({arg})@ on line {at} takes no default value; use @{func}-OPTIONAL({arg}, …)@ for a default");
        }
        cur.skip_ws();
        parse_default(cur, at, &form, arg)?;
        cur.skip_ws();
    }
    if !cur.eat(b')') || !cur.eat(b'@') {
        bail!("Malformed @{form}({arg}…)@ token on line {at}: expected ')@' to close it");
    }
    let list = match source {
        Source::Env => &mut refs.env,
        Source::Knob => &mut refs.knobs,
    };
    match list.iter_mut().find(|r| r.name == arg) {
        Some(r) => r.optional &= optional,
        None => list.push(Reference { name: arg.to_owned(), optional }),
    }
    Ok(())
}

/// A `-OPTIONAL` token's default: `"…"`/`'…'` (escapes `\"`, `\'`, `\\`) or
/// a bare run of letters and digits.
fn parse_default(cur: &mut Cursor<'_>, at: usize, form: &str, arg: &str) -> Result<String> {
    match cur.peek() {
        Some(quote @ (b'"' | b'\'')) => {
            cur.bump();
            let mut bytes = Vec::new();
            loop {
                match cur.bump() {
                    None => bail!("Unterminated quoted default value in @{form}({arg}, …)@ on line {at}"),
                    Some(b'\\') => match cur.bump() {
                        Some(c @ (b'"' | b'\'' | b'\\')) => bytes.push(c),
                        Some(c) => {
                            bytes.push(b'\\');
                            bytes.push(c);
                        }
                        None => bail!("Unterminated quoted default value in @{form}({arg}, …)@ on line {at}"),
                    },
                    Some(b) if b == quote => break,
                    Some(b) => bytes.push(b),
                }
            }
            String::from_utf8(bytes).context("Default value is not valid UTF-8")
        }
        _ => {
            let bare = cur.take_while(|b| b.is_ascii_alphanumeric());
            let next = cur.rest().chars().next();
            let closes = match next {
                Some(')') => true,
                Some(' ' | '\t') => {
                    let mut probe = Cursor { text: cur.text, pos: cur.pos, end: cur.end };
                    probe.skip_ws();
                    probe.peek() == Some(b')')
                }
                _ => false,
            };
            if !closes {
                match next {
                    None => bail!("Malformed @{form}({arg}, …)@ token on line {at}: expected ')@' to close it"),
                    Some(c) => bail!(
                        "Unquoted default value in @{form}({arg}, …)@ on line {at} contains '{c}'; only letters \
                         and digits may go unquoted, so quote it: @{form}({arg}, \"…\")@"
                    ),
                }
            }
            if bare.is_empty() {
                bail!("Empty default value in @{form}({arg}, )@ on line {at}: write \"\" for an empty default, or drop the comma");
            }
            Ok(bare.to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of<'a>(t: &'a str, spans: &[Range<usize>]) -> Vec<&'a str> {
        spans.iter().map(|s| &t[s.clone()]).collect()
    }

    #[test]
    fn comments_follow_par_peg() {
        let t = "# header me@host @@\nIO::out_dir = \"@QUEUE@#@NODES@\" # n=@NODES@ @\n";
        assert_eq!(text_of(t, &comment_spans(t)), ["# header me@host @@", "# n=@NODES@ @"]);
        // An escaped quote keeps the string open.
        let t = "x = \"a\\\"# @NODES@\" # @\n";
        assert_eq!(text_of(t, &comment_spans(t)), ["# @"]);
        // In a multi-line string, `#` on the first line is text; on later
        // lines a comment ending at the newline or the closing quote.
        let t = "ActiveThorns = \"# @NODES@\n  Carpet # driver me@host\n  IOUtil # last @\"\n# @\n";
        assert_eq!(text_of(t, &comment_spans(t)), ["# driver me@host", "# last @", "# @"]);
    }

    /// Exactly what cactup's export declares: comments and `@@` skipped, each
    /// name once in order of first use, a knob optional only when every use
    /// has a default.
    #[test]
    fn references_lists_what_a_parfile_uses() {
        let par = "# @NOT_ME@ me@host\n\
                   IO::out_dir = \"@SIMULATION_NAME@/@@x\"\n\
                   K::f = \"@KNOB(kadath-initial-data)@\" # @KNOB(commented)@\n\
                   A::b = \"@KNOB-OPTIONAL(mail, none)@ @SIMULATION_NAME@ @WALLTIME_HOURS@\"\n\
                   A::c = \"@KNOB-OPTIONAL(kadath-initial-data, x)@ @ENV(HOME)@\"\n";
        let refs = references(par).unwrap();
        assert_eq!(refs.vars, ["SIMULATION_NAME", "WALLTIME_HOURS"]);
        let knobs: Vec<(&str, bool)> = refs.knobs.iter().map(|r| (r.name.as_str(), r.optional)).collect();
        assert_eq!(knobs, [("kadath-initial-data", false), ("mail", true)]);
        assert_eq!(refs.env, [Reference { name: "HOME".into(), optional: false }]);
    }

    /// A real parfile's case: `@KNOB(name)@` written in comments to document
    /// the syntax is no use of a knob called `name`.
    #[test]
    fn documentation_in_comments_is_not_a_use() {
        let par = "#  expansion, @KNOB(name)@ is replaced by the value of the cactup \"knob\"\n\
                   #   * @KNOB(name)@ is the REQUIRED form\n\
                   kadathimporter::filename = \"@KNOB(kadath-initial-data)@\"\n";
        let refs = references(par).unwrap();
        let knobs: Vec<&str> = refs.knobs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(knobs, ["kadath-initial-data"]);
    }

    #[test]
    fn malformed_tokens_and_stray_ats_are_errors_with_their_line() {
        let err = references("# fine @\nx = a@b\n").unwrap_err().to_string();
        assert!(err.contains("line 2"), "{err}");
        assert!(references("x = \"@KNOB(Bad)@\"\n").is_err());
        assert!(references("x = \"@KNOB(a, b)@\"\n").is_err(), "a default needs -OPTIONAL");
        assert!(references("x = \"@KNOB-OPTIONAL(a, two words)@\"\n").is_err(), "an unquoted default with a blank");
        assert_eq!(references("x = \"@KNOB-OPTIONAL(a, 'q w')@\"\n").unwrap().knobs.len(), 1, "a quoted default may hold a blank");
    }
}
