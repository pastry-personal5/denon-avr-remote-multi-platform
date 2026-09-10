//! Policy for capability-gated receiver source catalogs.

use crate::ports::{OperationError, OperationErrorKind};
use denon_avr_domain::{
    CatalogResponseEvidence, Freshness, ModelCapabilities, SourceCatalog, SourceCatalogObservation,
};

pub(crate) fn ensure_supported(capabilities: &ModelCapabilities) -> Result<(), OperationError> {
    if capabilities.source_catalog_read {
        Ok(())
    } else {
        Err(OperationError::new(
            OperationErrorKind::Unsupported,
            "source catalog",
            "selected receiver has no validated source catalog capability",
        ))
    }
}

pub(crate) fn merge_refresh(
    previous: SourceCatalog,
    generation: u64,
    result: Result<SourceCatalogObservation, OperationError>,
) -> (
    SourceCatalog,
    SourceCatalogObservation,
    Result<(), OperationError>,
) {
    match result {
        Ok(mut observation)
            if observation.response_evidence == CatalogResponseEvidence::Unsupported
                && previous.generation == generation =>
        {
            let mut catalog = previous;
            catalog.freshness = Freshness::Partial;
            catalog.error = Some("receiver did not provide a source catalog response".into());
            observation.catalog = catalog.clone();
            (catalog, observation, Ok(()))
        }
        Ok(mut observation) => {
            observation.catalog.generation = generation;
            let catalog = observation.catalog.clone();
            (catalog, observation, Ok(()))
        }
        Err(error) => {
            let catalog = if previous.generation == generation {
                let mut catalog = previous;
                catalog.freshness = Freshness::Partial;
                catalog.error = Some(error.to_string());
                catalog
            } else {
                SourceCatalog {
                    freshness: Freshness::Unknown,
                    generation,
                    error: Some(error.to_string()),
                    ..SourceCatalog::default()
                }
            };
            let observation = SourceCatalogObservation {
                catalog: catalog.clone(),
                raw_response: String::new(),
                response_evidence: error_evidence(&error),
            };
            (catalog, observation, Err(error))
        }
    }
}

fn error_evidence(error: &OperationError) -> CatalogResponseEvidence {
    match error.kind {
        OperationErrorKind::Malformed => CatalogResponseEvidence::Malformed,
        OperationErrorKind::Timeout => CatalogResponseEvidence::Timeout,
        OperationErrorKind::Disconnected | OperationErrorKind::Connection => {
            CatalogResponseEvidence::Disconnected
        }
        _ => CatalogResponseEvidence::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_response_preserves_same_generation_catalog() {
        let previous = SourceCatalog {
            generation: 4,
            freshness: Freshness::Live,
            ..SourceCatalog::default()
        };
        let response = SourceCatalogObservation {
            catalog: SourceCatalog::default(),
            raw_response: String::new(),
            response_evidence: CatalogResponseEvidence::Unsupported,
        };
        let (catalog, observation, result) = merge_refresh(previous, 4, Ok(response));
        assert_eq!(catalog.freshness, Freshness::Partial);
        assert_eq!(observation.catalog, catalog);
        assert_eq!(result, Ok(()));
    }
}
