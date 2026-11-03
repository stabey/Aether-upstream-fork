//! Legacy IDE and opt-in Hub identities have independent version histories.
use aether_ai_formats::client_profile::{validate_client_version, ClientProfileStore};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AntigravityClientProfile {
    pub version: String,
    pub user_agent: String,
    pub envelope_user_agent: String,
    pub goog_api_client: String,
}
impl AntigravityClientProfile {
    pub fn legacy() -> Self {
        Self {
            version: super::auth::ANTIGRAVITY_CLIENT_VERSION.into(),
            user_agent: super::auth::ANTIGRAVITY_REQUEST_USER_AGENT.into(),
            envelope_user_agent: super::auth::ANTIGRAVITY_REQUEST_USER_AGENT.into(),
            goog_api_client: "gl-node/18.18.2 fire/0.8.6 grpc/1.10.x".into(),
        }
    }
    pub fn hub(version: &str) -> Result<Self, &'static str> {
        let version = validate_client_version(version)?;
        Ok(Self {
            version: version.into(),
            user_agent: format!(
                "antigravity/hub/{version} (aidev_client; os_type=darwin; arch=arm64)"
            ),
            envelope_user_agent: "antigravity".into(),
            goog_api_client: "gl-node/22.21.1".into(),
        })
    }
}
pub fn hub_family_from(value: Option<&str>) -> bool {
    match value.map(str::trim) {
        None | Some("") | Some("legacy") => false,
        Some("hub") => true,
        Some(value) => {
            tracing::warn!(
                event_name = "antigravity_client_family_invalid",
                family = value,
                "unknown Antigravity client family; using legacy IDE profile"
            );
            false
        }
    }
}
pub fn antigravity_hub_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        hub_family_from(
            std::env::var("AETHER_ANTIGRAVITY_CLIENT_FAMILY")
                .ok()
                .as_deref(),
        )
    })
}

static HUB: OnceLock<ClientProfileStore<AntigravityClientProfile>> = OnceLock::new();
fn hub_store() -> &'static ClientProfileStore<AntigravityClientProfile> {
    HUB.get_or_init(|| {
        ClientProfileStore::new(AntigravityClientProfile::hub("2.28.2").expect("valid Hub builtin"))
    })
}
pub fn antigravity_client_profile() -> Arc<AntigravityClientProfile> {
    if antigravity_hub_enabled() {
        hub_store().snapshot()
    } else {
        Arc::new(AntigravityClientProfile::legacy())
    }
}
pub fn antigravity_hub_client_version() -> String {
    hub_store().snapshot().version.clone()
}
pub fn set_antigravity_hub_client_version(version: &str) -> Result<(), &'static str> {
    hub_store().publish(AntigravityClientProfile::hub(version)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn families_and_inflight_snapshots_are_independent() {
        assert!(AntigravityClientProfile::hub("2.28.2\nx: y").is_err());
        assert!(!hub_family_from(None));
        assert!(!hub_family_from(Some("unknown")));
        assert!(hub_family_from(Some("hub")));
        let store = ClientProfileStore::new(AntigravityClientProfile::hub("2.28.2").unwrap());
        let snapshot = store.snapshot();
        store.publish(AntigravityClientProfile::hub("2.29.0").unwrap());
        assert!(snapshot.user_agent.contains(&snapshot.version));
        assert_eq!(snapshot.version, "2.28.2");
        assert_eq!(AntigravityClientProfile::legacy().version, "4.3.0");
    }
}
