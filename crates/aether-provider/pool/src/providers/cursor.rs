use crate::capability::ProviderPoolCapabilities;
use crate::provider::ProviderPoolAdapter;

/// Pool adapter for the Cursor sidecar gateway provider.
///
/// Quota refresh against Cursor's dashboard RPC is intentionally out of scope
/// for the first integration; run the external SDK gateway (cursor-sdk2api /
/// Cursor2API) for account health and rely on Aether's generic key scheduling.
#[derive(Debug, Clone, Default)]
pub struct CursorProviderPoolAdapter;

impl ProviderPoolAdapter for CursorProviderPoolAdapter {
    fn provider_type(&self) -> &'static str {
        aether_provider_transport::cursor::CURSOR_PROVIDER_TYPE
    }

    fn capabilities(&self) -> ProviderPoolCapabilities {
        ProviderPoolCapabilities::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_pool_adapter_reports_provider_type() {
        let adapter = CursorProviderPoolAdapter;
        assert_eq!(adapter.provider_type(), "cursor");
        assert!(!adapter.capabilities().quota_refresh);
    }
}
