//! The single authority on how SQL is spelled per backend.
//!
//! Every portability decision in the framework funnels through [`Dialect`].
//! `sqlx`'s `Any` driver hides the concrete backend and rewrites `?`
//! placeholders per driver, but it does *not* rewrite dialect syntax: an
//! `ON CONFLICT` upsert, a double-quoted identifier or `CREATE INDEX IF NOT
//! EXISTS` is Postgres/SQLite-only SQL that MySQL rejects outright.
//!
//! The dialect travels with the connection ([`crate::Db`]) so no call site has
//! to remember which backend it is talking to.

use ferris_core::{DatabaseType, Error, Result, ValidationError, config::SslMode};

/// Concrete SQL backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    #[default]
    Postgres,
    MySQL,
    SQLite,
}

impl Dialect {
    /// Dialect implied by a configured database type.
    pub fn from_database_type(database_type: DatabaseType) -> Self {
        match database_type {
            DatabaseType::Postgres => Self::Postgres,
            DatabaseType::MySQL => Self::MySQL,
            DatabaseType::SQLite => Self::SQLite,
        }
    }

    /// Dialect implied by a sqlx connection-URL scheme.
    ///
    /// Anything unrecognised falls back to `Postgres` because that is the
    /// framework default; the alternative is failing to open a pool at all.
    pub fn from_url(url: &str) -> Self {
        if url.starts_with("mysql") || url.starts_with("mariadb") {
            Self::MySQL
        } else if url.starts_with("sqlite") {
            Self::SQLite
        } else {
            Self::Postgres
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::MySQL => "mysql",
            Self::SQLite => "sqlite",
        }
    }

    /// Postgres has a native case-insensitive `ILIKE`; the others emulate it
    /// with `LOWER(...) LIKE LOWER(...)`.
    pub fn supports_ilike(&self) -> bool {
        matches!(self, Self::Postgres)
    }

    /// `FULL [OUTER] JOIN` exists on Postgres only; MySQL has no `FULL JOIN`
    /// and SQLite added it in 3.39, which bundled `libsqlite` cannot be relied
    /// upon, so reject it rather than emit invalid SQL.
    pub fn supports_full_join(&self) -> bool {
        matches!(self, Self::Postgres)
    }

    /// Whether `SNAPSHOT` isolation is available (Postgres only).
    pub fn supports_snapshot_isolation(&self) -> bool {
        matches!(self, Self::Postgres)
    }

    /// MySQL has no `ON CONFLICT`; uniqueness is resolved with
    /// `ON DUPLICATE KEY UPDATE` and no conflict target is named.
    pub fn supports_on_conflict(&self) -> bool {
        !matches!(self, Self::MySQL)
    }

    /// Oracle MySQL has no `CREATE INDEX IF NOT EXISTS` (MariaDB does). The
    /// portable spelling is to attempt the index and ignore a duplicate-name
    /// error, which [`crate::sql::ensure_index`] does.
    pub fn supports_create_index_if_not_exists(&self) -> bool {
        matches!(self, Self::Postgres | Self::SQLite)
    }

    /// MySQL rejects a `DEFAULT` on a `TEXT`/`BLOB` column
    /// (`ER_BLOB_CANT_HAVE_DEFAULT`), so text columns carrying a default must
    /// become `VARCHAR`.
    pub fn allows_default_on_text(&self) -> bool {
        !matches!(self, Self::MySQL)
    }

    fn quote_char(self) -> char {
        match self {
            // MySQL's ANSI_QUOTES mode would accept `"`, but it is off by
            // default, where `"user"` lexes as a *string literal*. Backticks
            // are correct on every MySQL configuration.
            Self::MySQL => '`',
            Self::Postgres | Self::SQLite => '"',
        }
    }

    /// Quote an SQL identifier (table, column, index, alias).
    ///
    /// The name is restricted to `[A-Za-z0-9_]`, so quoting is a matter of
    /// correctness — reserved words such as `user`, `key` and `default`
    /// otherwise parse as literals on MySQL — rather than injection defence.
    pub fn quote_ident(&self, ident: &str) -> Result<String> {
        if !is_plain_ident(ident) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "Invalid identifier: {ident}"
            ))));
        }
        let q = self.quote_char();
        Ok(format!("{q}{ident}{q}"))
    }

    /// Quote an identifier without failing.
    ///
    /// For the rare input that is not a plain identifier this returns `None`
    /// rather than a silently-quoted string, so the caller is forced to decide
    /// whether to reject or to escape. It never falls back to the raw name:
    /// emitting an unvalidated identifier is exactly the bug
    /// [`Self::quote_ident`] exists to prevent.
    pub fn try_quote_ident(&self, ident: &str) -> Option<String> {
        is_plain_ident(ident).then(|| {
            let q = self.quote_char();
            format!("{q}{ident}{q}")
        })
    }

    /// Escape a string so it can be embedded as a `'...'` SQL literal.
    ///
    /// Backslashes are doubled on MySQL because it treats `\` as an escape
    /// character inside string literals by default; Postgres uses the standard
    /// `''` doubling (and only interprets backslashes when
    /// `standard_conforming_strings` is off, which is not the default).
    pub fn escape_string_literal(&self, value: &str) -> String {
        let mut out = String::with_capacity(value.len() + 2);
        for c in value.chars() {
            match c {
                '\'' => out.push_str("''"),
                '\\' if matches!(self, Self::MySQL | Self::SQLite) => out.push_str("\\\\"),
                _ => out.push(c),
            }
        }
        out
    }

    /// Query-string fragment applying an [`SslMode`] for this dialect.
    pub fn ssl_param(&self, mode: SslMode) -> Option<&'static str> {
        let value = match self {
            Self::Postgres => match mode {
                SslMode::Disable => "disable",
                SslMode::Allow => "allow",
                SslMode::Prefer => "prefer",
                SslMode::Require => "require",
                SslMode::VerifyCa => "verify-ca",
                SslMode::VerifyFull => "verify-full",
            },
            Self::MySQL => match mode {
                SslMode::Disable => "DISABLED",
                SslMode::Allow | SslMode::Prefer => "PREFERRED",
                SslMode::Require => "REQUIRED",
                SslMode::VerifyCa => "VERIFY_CA",
                SslMode::VerifyFull => "VERIFY_IDENTITY",
            },
            Self::SQLite => return None,
        };
        Some(value)
    }

    /// Key the SSL mode is passed under: `ssl-mode` on MySQL, `sslmode`
    /// elsewhere.
    pub fn ssl_param_key(&self) -> &'static str {
        if matches!(self, Self::MySQL) {
            "ssl-mode"
        } else {
            "sslmode"
        }
    }
}

/// Whether `s` is a bare SQL identifier: `[A-Za-z0-9_]+`.
pub fn is_plain_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `s` is a plain or qualified identifier (`name`, `tab.name`).
pub fn is_qualified_ident(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(is_plain_ident) && !s.starts_with('.') && !s.ends_with('.')
}

#[cfg(test)]
mod tests;
