//! Recording/Static `ExternalDataProvider` test doubles (CORE-019A Phase 5,
//! AD-010).
//!
//! Same-contract principle as `effects.rs`'s `RecordingExecutor`: a real
//! implementation of the real production `ExternalDataProvider` trait, not a
//! look-alike. `StaticDataProvider` returns one canned [`DataResponse`] for
//! every `fetch`; `RecordingDataProvider` additionally records every
//! [`DataRequest`] it receives so a test can assert on what a handler
//! actually sent, then replies with its own canned response.

use std::sync::Mutex;

use async_trait::async_trait;
use ego_runtime::providers::ExternalDataProvider;
use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};

/// Always returns the same canned [`DataResponse`], regardless of the
/// request — the simplest possible deterministic double (AD-010: canned
/// response only, no mock framework).
pub struct StaticDataProvider {
    response: DataResponse,
}

impl StaticDataProvider {
    /// A double that always succeeds with `response`.
    pub fn new(response: DataResponse) -> Self {
        Self { response }
    }
}

#[async_trait]
impl ExternalDataProvider for StaticDataProvider {
    async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
        Ok(DataResponse {
            payload: self.response.payload.clone(),
            cache_hit: self.response.cache_hit,
        })
    }
}

/// Records every [`DataRequest`] it receives, then replies with a canned
/// [`DataResponse`] — call-recording, same principle as `RecordingExecutor`.
pub struct RecordingDataProvider {
    requests: Mutex<Vec<DataRequest>>,
    response: DataResponse,
}

impl RecordingDataProvider {
    /// A double that records every request and always succeeds with `response`.
    pub fn new(response: DataResponse) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response,
        }
    }

    /// Every request recorded so far, in call order.
    pub fn requests(&self) -> Vec<DataRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ExternalDataProvider for RecordingDataProvider {
    async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(DataResponse {
            payload: self.response.payload.clone(),
            cache_hit: self.response.cache_hit,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn request(key: &str) -> DataRequest {
        DataRequest {
            key: key.to_string(),
            payload: vec![1, 2, 3],
        }
    }

    #[tokio::test]
    async fn static_data_provider_returns_the_same_canned_response_every_call() {
        let provider = StaticDataProvider::new(DataResponse {
            payload: vec![9, 9, 9],
            cache_hit: true,
        });

        let first = provider.fetch(request("a")).await.unwrap();
        let second = provider.fetch(request("b")).await.unwrap();

        assert_eq!(first.payload, vec![9, 9, 9]);
        assert!(first.cache_hit);
        assert_eq!(second.payload, vec![9, 9, 9]);
        assert!(second.cache_hit);
    }

    #[tokio::test]
    async fn recording_data_provider_records_every_request_and_is_inspectable_after_the_fact() {
        let provider = Arc::new(RecordingDataProvider::new(DataResponse {
            payload: vec![7],
            cache_hit: false,
        }));

        provider.fetch(request("k1")).await.unwrap();
        provider.fetch(request("k2")).await.unwrap();

        let requests = provider.requests();
        assert_eq!(requests.len(), 2, "both fetch calls must be recorded");
        assert_eq!(requests[0].key, "k1");
        assert_eq!(requests[1].key, "k2");
    }
}
