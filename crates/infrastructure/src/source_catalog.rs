//! Bounded infrastructure adapter for the evidence-gated source catalog read.

use crate::AppCommandHttpClient;
use denon_avr_domain::{
    CatalogResponseEvidence, Freshness, ReceiverEndpoint, SourceCatalog, SourceCatalogObservation,
};
use denon_avr_protocol::source_catalog::{
    parse_source_catalog_response, source_catalog_request_xml,
};
use std::io;
use std::time::{Duration, SystemTime};

pub struct SourceCatalogHttpClient {
    client: AppCommandHttpClient,
}

impl SourceCatalogHttpClient {
    pub fn new(endpoint: ReceiverEndpoint, timeout: Duration) -> io::Result<Self> {
        Ok(Self {
            client: AppCommandHttpClient::new(endpoint, timeout)?,
        })
    }

    pub fn read(&self, generation: u64) -> io::Result<SourceCatalogObservation> {
        let response = self
            .client
            .execute_xml_at("/goform/AppCommand.xml", source_catalog_request_xml())?;
        let parsed = parse_source_catalog_response(&response.body)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        let freshness = match parsed.evidence {
            CatalogResponseEvidence::Complete => Freshness::Live,
            CatalogResponseEvidence::Partial => Freshness::Partial,
            CatalogResponseEvidence::Unsupported => Freshness::Unknown,
            _ => unreachable!("protocol parser emits only response-shape evidence"),
        };
        Ok(SourceCatalogObservation {
            catalog: SourceCatalog {
                entries: parsed.entries,
                freshness,
                generation,
                observed_at: Some(SystemTime::now()),
                error: None,
            },
            raw_response: response.body,
            response_evidence: parsed.evidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_endpoint_before_network_access() {
        assert!(SourceCatalogHttpClient::new(
            ReceiverEndpoint {
                host: "\n".into(),
                port: 80,
            },
            Duration::from_secs(1),
        )
        .is_err());
    }
}
