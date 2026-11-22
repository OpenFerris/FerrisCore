//! Runtime detection of the Cargo features the host binary was built with.
//!
//! Cargo features are decided at **compile** time, but two consumers only learn
//! about the decision at run time:
//!
//! * the WIT side — `platform::features()` must tell a guest which subsystems
//!   this host can actually serve, so the guest hides a surface the host did not compile
//!   instead of calling an import the host answers `unsupported` to;
//! * `ferris doctor` — it prints the compiled set next to the runtime
//!   `[features]` gates, and a mismatch is a configuration error.
//!
//! Both must read the **same** value, and `ferris-wasm` cannot read it from
//! `ferris-server` (that crate depends on this one, and the dependency cannot be
//! inverted). So the value lives here, in the bottom crate: a plain set of names
//! with a process-wide slot. `ferris-server` fills it at boot from
//! `cfg!(feature = …)`; everyone else only reads it.
//!
//! The slot is intentionally not a `static mut` and not a `Mutex`: install
//! happens once, before the listener accepts, and readers only need an immutable
//! reference afterwards. A second install is refused (returns `false`) so a
//! reload cannot silently advertise a different set than the code that is
//! running — the same first-install-wins rule as the realtime handler and the
//! PKI provider.

use std::collections::BTreeSet;
use std::sync::OnceLock;

/// Sorted names of the host features this binary compiled.
///
/// `BTreeSet` gives a deterministic order, which is what the WIT call and the
/// doctor output both need to be stable across runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostFeatures {
    names: BTreeSet<String>,
}

impl HostFeatures {
    /// Build a set from any iterator of names. Duplicates collapse; order does
    /// not matter.
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// Whether `name` is compiled in.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// The names in deterministic (sorted) order.
    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.names.iter().map(String::as_str)
    }

    /// The names as owned strings, for callers that hand them to a wire format.
    #[must_use]
    pub fn to_vec(&self) -> Vec<String> {
        self.names.iter().cloned().collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

impl std::fmt::Display for HostFeatures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.names.iter().cloned().collect::<Vec<_>>().join(","))
    }
}

/// The installed set. `None` before boot (unit tests, embedding) — readers fall
/// back to what they know instead of guessing.
static INSTALLED: OnceLock<HostFeatures> = OnceLock::new();

/// Install the compiled feature set for this process.
///
/// First install wins: returns `false` when a set was already installed, which
/// the caller reports (a second boot in the same process is a programming error,
/// not a reason to panic).
pub fn install(features: HostFeatures) -> bool {
    INSTALLED.set(features).is_ok()
}

/// The installed set, or `None` when nothing was installed yet.
#[must_use]
pub fn installed() -> Option<&'static HostFeatures> {
    INSTALLED.get()
}

/// Whether the installed set contains `name`.
///
/// A process that never installed one answers `false`: "not known to be
/// compiled" is the safe reading for a capability question.
#[must_use]
pub fn is_enabled(name: &str) -> bool {
    installed().is_some_and(|features| features.contains(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_is_sorted_and_deduplicated() {
        let features = HostFeatures::new(["web", "api", "web", "search"]);
        assert_eq!(features.to_vec(), vec!["api", "search", "web"]);
        assert_eq!(features.len(), 3);
        assert!(!features.is_empty());
        assert!(features.contains("search"));
        assert!(!features.contains("realtime"));
        assert_eq!(features.to_string(), "api,search,web");
    }

    #[test]
    fn an_empty_set_is_honest_about_it() {
        let features = HostFeatures::default();
        assert!(features.is_empty());
        assert_eq!(features.len(), 0);
        assert!(!features.contains("api"));
    }

    #[test]
    fn an_uninstalled_process_does_not_claim_any_capability() {
        // `install` is process-global and other tests in this binary may have
        // installed a set, so this only pins the fallback logic itself.
        if installed().is_none() {
            assert!(!is_enabled("api"));
            assert!(!is_enabled("search"));
        }
    }
}
