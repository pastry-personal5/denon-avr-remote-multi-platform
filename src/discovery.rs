//! Compatibility façade for receiver discovery.
//!
//! This module re-exports from the canonical layered implementation.

pub use denon_avr_platform::SsdpDiscovery;
pub use denon_avr_platform::DEFAULT_DISCOVERY_TIMEOUT;

/// Legacy discovered receiver type.
#[allow(dead_code)]
pub struct DiscoveredReceiver {
    pub address: std::net::SocketAddr,
    pub location: Option<String>,
    pub server: Option<String>,
    pub model: Option<String>,
    pub search_target: Option<String>,
    pub unique_service_name: Option<String>,
}

/// Legacy discovery function.
#[allow(deprecated)]
pub fn discover(timeout: std::time::Duration) -> Result<Vec<DiscoveredReceiver>, String> {
    use denon_avr_core::ReceiverDiscovery;
    let discovery = SsdpDiscovery;
    discovery
        .discover(timeout)
        .map_err(|e| e.to_string())
        .map(|receivers| {
            receivers
                .into_iter()
                .map(|r| DiscoveredReceiver {
                    address: r.address,
                    location: r.location,
                    server: r.server,
                    model: r.model,
                    search_target: r.search_target,
                    unique_service_name: r.unique_service_name,
                })
                .collect()
        })
}
