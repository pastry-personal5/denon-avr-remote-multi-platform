//! Bounded HTTP reader for receiver-owned Quick Select display names.

use crate::AppCommandHttpClient;
use denon_avr_domain::{
    Freshness, QuickSelectName, QuickSelectNameObservation, QuickSelectNameResponseEvidence,
    ReceiverEndpoint,
};
use denon_avr_protocol::parse_quick_select_names;
use std::io;
use std::time::{Duration, SystemTime};

const REQUEST_XML: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<tx>\n<cmd id=\"1\">GetQuickSelectName</cmd>\n</tx>";

pub struct QuickSelectNamesHttpClient {
    client: AppCommandHttpClient,
}

impl QuickSelectNamesHttpClient {
    pub fn new(endpoint: ReceiverEndpoint, timeout: Duration) -> io::Result<Self> {
        Ok(Self {
            client: AppCommandHttpClient::new(endpoint, timeout)?,
        })
    }

    pub fn read(&self, generation: u64) -> io::Result<QuickSelectNameObservation> {
        let response = self
            .client
            .execute_xml_at("/goform/AppCommand.xml", REQUEST_XML)?;
        let parsed = parse_quick_select_names(&response.body)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        let mut names = [None, None, None, None];
        for (index, value) in parsed.names.into_iter().enumerate() {
            names[index] = value
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .map(QuickSelectName::new)
                .transpose()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        }
        Ok(QuickSelectNameObservation {
            names,
            sources: parsed
                .sources
                .map(|value| value.map(|value| value.trim().to_owned())),
            freshness: if parsed.complete {
                Freshness::Live
            } else {
                Freshness::Partial
            },
            generation,
            observed_at: Some(SystemTime::now()),
            error: None,
            raw_response: response.body,
            response_evidence: if parsed.complete {
                QuickSelectNameResponseEvidence::Complete
            } else {
                QuickSelectNameResponseEvidence::Partial
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_endpoint_before_network_access() {
        assert!(QuickSelectNamesHttpClient::new(
            ReceiverEndpoint {
                host: "\n".into(),
                port: 8080
            },
            Duration::from_secs(1),
        )
        .is_err());
    }
}
