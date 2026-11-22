//! Email accounts and queue settings.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::redact::Redacted;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    pub default_account: String,
    pub accounts: Vec<EmailAccountConfig>,
    pub queue_concurrency: usize,
    pub retry_attempts: u32,
    pub retry_delay_secs: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct EmailAccountConfig {
    pub name: String,
    pub email: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_tls: bool,
    /// Upgrade the SMTP submission with STARTTLS instead of implicit TLS.
    /// Mutually exclusive with `smtp_tls` in practice (implicit TLS already
    /// encrypts from the first byte); set this for the common 587 submission
    /// port. Defaults to `false` so existing configs keep working.
    #[serde(default)]
    pub smtp_starttls: bool,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub imap_username: Option<String>,
    pub imap_password: Option<String>,
    pub imap_tls: bool,
    /// Upgrade the IMAP connection with STARTTLS instead of implicit TLS
    /// (typically port 143). Defaults to `false` for backwards compatibility.
    #[serde(default)]
    pub imap_starttls: bool,
}

impl fmt::Debug for EmailAccountConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmailAccountConfig")
            .field("name", &self.name)
            .field("email", &self.email)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("smtp_username", &self.smtp_username)
            .field("smtp_password", &Redacted)
            .field("smtp_tls", &self.smtp_tls)
            .field("smtp_starttls", &self.smtp_starttls)
            .field("imap_host", &self.imap_host)
            .field("imap_port", &self.imap_port)
            .field("imap_username", &self.imap_username)
            .field(
                "imap_password",
                &self.imap_password.as_ref().map(|_| Redacted),
            )
            .field("imap_tls", &self.imap_tls)
            .field("imap_starttls", &self.imap_starttls)
            .finish()
    }
}
