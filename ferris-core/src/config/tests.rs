use super::*;
use crate::error::ConfigError;
use serde::Deserialize;

#[test]
fn connection_url_percent_encodes_credentials() {
    let cfg = DatabaseConfig {
        r#type: DatabaseType::Postgres,
        host: "db.internal".into(),
        port: 5432,
        name: "app/db".into(),
        user: "us@er".into(),
        password: "p:a#ss/word?x=y".into(),
        max_connections: 5,
        min_connections: 0,
        connect_timeout_secs: 5,
        idle_timeout_secs: 300,
        max_lifetime_secs: 1800,
        ssl_mode: SslMode::Prefer,
        schema: None,
    };
    assert_eq!(
        cfg.connection_url(),
        "postgres://us%40er:p%3Aa%23ss%2Fword%3Fx%3Dy@db.internal:5432/app%2Fdb"
    );
    // The password must never be able to smuggle in query parameters.
    assert!(!cfg.connection_url().contains("?"));
}

#[test]
fn connection_url_keeps_simple_credentials_as_is() {
    let cfg = DatabaseConfig {
        r#type: DatabaseType::MySQL,
        host: "db.internal".into(),
        port: 3306,
        name: "app".into(),
        user: "ferris".into(),
        password: "plain".into(),
        max_connections: 5,
        min_connections: 0,
        connect_timeout_secs: 5,
        idle_timeout_secs: 300,
        max_lifetime_secs: 1800,
        ssl_mode: SslMode::Prefer,
        schema: None,
    };
    assert_eq!(
        cfg.connection_url(),
        "mysql://ferris:plain@db.internal:3306/app"
    );
}

#[test]
fn sqlite_memory_uses_sqlx_special_form() {
    let cfg = DatabaseConfig {
        r#type: DatabaseType::SQLite,
        host: String::new(),
        port: 0,
        name: ":memory:".into(),
        user: String::new(),
        password: String::new(),
        max_connections: 1,
        min_connections: 0,
        connect_timeout_secs: 5,
        idle_timeout_secs: 300,
        max_lifetime_secs: 1800,
        ssl_mode: SslMode::Disable,
        schema: None,
    };
    assert_eq!(cfg.connection_url(), "sqlite::memory:");
}

#[test]
fn debug_output_redacts_secrets_but_serialization_keeps_them() {
    let cfg = DatabaseConfig {
        r#type: DatabaseType::Postgres,
        host: "db.internal".into(),
        port: 5432,
        name: "app".into(),
        user: "ferris".into(),
        password: "super-secret-password".into(),
        max_connections: 5,
        min_connections: 0,
        connect_timeout_secs: 5,
        idle_timeout_secs: 300,
        max_lifetime_secs: 1800,
        ssl_mode: SslMode::Prefer,
        schema: None,
    };

    let debug = format!("{cfg:?}");
    assert!(
        !debug.contains("super-secret-password"),
        "Debug must not contain the password: {debug}"
    );
    assert!(debug.contains("\"***\""), "Debug must show the placeholder");

    // Serialization is deliberately untouched: the CLI persists and redacts
    // the merged config at display time, so the real value must round-trip.
    let json: serde_json::Value = serde_json::to_value(&cfg).unwrap();
    assert_eq!(json["password"], "super-secret-password");
    assert_eq!(
        serde_json::from_value::<DatabaseConfig>(json)
            .unwrap()
            .password,
        "super-secret-password"
    );

    let auth = AuthConfig {
        jwt_secret: Some("jwt-secret-value".into()),
        jwt_expiry_hours: 8,
        refresh_expiry_days: 14,
        password_min_length: 8,
        password_require_uppercase: true,
        password_require_lowercase: true,
        password_require_numbers: true,
        password_require_symbols: false,
        max_login_attempts: 5,
        lockout_duration_minutes: 15,
        mfa_enabled: false,
        mfa_issuer: "ferris".into(),
        session_timeout_minutes: 60,
        max_sessions_per_user: 5,
        oauth: OAuthConfig::default(),
    };
    let auth_debug = format!("{auth:?}");
    assert!(!auth_debug.contains("jwt-secret-value"), "{auth_debug}");
}

/// Every `SslMode` round-trips through serde with the spellings libpq/MySQL
/// document.
#[test]
fn ssl_mode_round_trips_with_the_standard_spellings() {
    for (mode, wire) in [
        (SslMode::Disable, "disable"),
        (SslMode::Allow, "allow"),
        (SslMode::Prefer, "prefer"),
        (SslMode::Require, "require"),
        (SslMode::VerifyCa, "verify-ca"),
        (SslMode::VerifyFull, "verify-full"),
    ] {
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, format!("\"{wire}\""));
        assert_eq!(serde_json::from_str::<SslMode>(&json).unwrap(), mode);
    }
}

/// `load_from` merges TOML first, then YAML, so YAML wins.
#[test]
fn yaml_overrides_toml_in_load_from() {
    #[derive(Deserialize)]
    struct Probe {
        database: ProbeName,
    }
    #[derive(Deserialize)]
    struct ProbeName {
        name: String,
    }

    let dir = std::env::temp_dir().join(format!("ferris-config-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("ferris.toml"),
        "[database]\nname = \"from-toml\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("ferris.yaml"), "database:\n  name: from-yaml\n").unwrap();

    let probe: Probe = Config::figment_for(&dir).extract().unwrap();
    assert_eq!(probe.database.name, "from-yaml");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A missing config directory is `ConfigError::NotFound`, not the `extract()`
/// wall of "missing field" errors the `NotFound` variant used to be
/// unreachable behind.
#[test]
fn load_from_a_missing_directory_is_not_found() {
    let missing = std::env::temp_dir().join(format!("ferris-nope-{}", uuid::Uuid::new_v4()));
    match Config::load_from(&missing) {
        Err(crate::Error::Config(ConfigError::NotFound(what))) => {
            assert!(what.contains(&missing.display().to_string()), "{what}");
        }
        other => panic!("expected ConfigError::NotFound, got {other:?}"),
    }
}

/// A name that is already a sqlite URL is not wrapped a second time.
#[test]
fn sqlite_urls_are_not_double_schemed() {
    let mut cfg = DatabaseConfig {
        r#type: DatabaseType::SQLite,
        name: "sqlite://data/app.db".into(),
        ..DatabaseConfig::default()
    };
    assert_eq!(cfg.connection_url(), "sqlite://data/app.db");
    cfg.name = "file:/tmp/app.db".into();
    assert_eq!(cfg.connection_url(), "file:/tmp/app.db");
    cfg.name = "app.db".into();
    assert_eq!(cfg.connection_url(), "sqlite://app.db");
}

/// `[features]` is real configuration now: missing keys mean "enabled", which
/// is what a config written before the table existed means.
#[test]
fn feature_flags_default_to_true_and_a_partial_table_stays_enabled() {
    let flags = FeatureFlags::default();
    assert!(
        flags.realtime
            && flags.jobs
            && flags.scheduler
            && flags.email
            && flags.files
            && flags.print
            && flags.report
            && flags.search
            && flags.webhooks
    );

    let partial: FeatureFlags =
        serde_json::from_str(r#"{"realtime": false}"#).expect("a partial table parses");
    assert!(!partial.realtime);
    assert!(partial.jobs);
    assert!(partial.email);
    assert!(partial.files);
    assert!(partial.print);
    assert!(partial.report);
    assert!(partial.search);
    assert!(partial.webhooks);

    let round_trip: FeatureFlags =
        serde_json::from_str(&serde_json::to_string(&flags).unwrap()).unwrap();
    assert!(round_trip.scheduler);
}

#[test]
fn config_carries_the_feature_flags() {
    let config = Config::default();
    assert!(config.features.realtime);
    assert!(config.features.email);
    assert!(config.features.report);
}
