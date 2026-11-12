use crate::{Error, Result};

/// Reads the top-level `format` key before full deserialization, so a file from a newer generation
/// gets an "upgrade" message rather than a confusing unknown-key error. Returns the generation
/// found, which is anything from `oldest` to `newest`: every older generation a format still
/// reads is a subset of the newest (fields only ever added, as optional), so the newest types
/// deserialize it as it is, and the caller checks that its content fits the generation it claims.
pub(crate) fn check(text: &str, what: &'static str, oldest: u32, newest: u32) -> Result<u32> {
    let table: toml::Table = toml::from_str(text).map_err(|source| Error::Toml { what, source })?;
    let found = match table.get("format") {
        Some(toml::Value::Integer(n)) if *n >= 1 && *n <= u32::MAX as i64 => *n as u32,
        Some(_) | None => return Err(Error::MissingFormat { what, supported: newest }),
    };
    if found > newest {
        return Err(Error::NewerFormat { what, found, supported: newest });
    }
    if found < oldest {
        return Err(Error::invalid(format!("{what} uses unknown format {found}")));
    }
    Ok(found)
}

/// The generations a format reads, for a message: `1` while there is only one, else `1 to N`.
pub(crate) fn readable(newest: u32) -> String {
    if newest <= 1 { "1".to_string() } else { format!("1 to {newest}") }
}
