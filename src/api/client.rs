use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::Serialize;
use thiserror::Error;

use crate::model::Delivery;

use super::types::{DeliveriesResponse, SupportedCarriersResponse};

const API_ROOT: &str = "https://api.parcel.app/external";
const MAX_READ_REQUESTS_PER_HOUR: usize = 20;
const MAX_ADD_REQUESTS_PER_DAY: usize = 20;

#[derive(Debug, Clone, Copy)]
pub enum FilterMode {
    Active,
    Recent,
}

impl FilterMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Recent => "recent",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NewDelivery {
    pub tracking_number: String,
    pub carrier_code: String,
    pub description: String,
    pub language: String,
    pub send_push_confirmation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postcode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Das lokale Abruflimit ist erreicht. Bitte später erneut versuchen.")]
    LocalRateLimit,
    #[error("Parcel hat die Anfrage abgelehnt: {0}")]
    Parcel(String),
    #[error("Netzwerkfehler: {0}")]
    Network(#[from] reqwest::Error),
}

pub struct ParcelClient {
    http: Client,
    api_root: String,
    api_key: String,
    read_requests: Mutex<VecDeque<Instant>>,
    add_requests: Mutex<VecDeque<Instant>>,
    carrier_cache: Mutex<Option<SupportedCarriersResponse>>,
}

impl ParcelClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self, ApiError> {
        Self::with_api_root(api_key, API_ROOT)
    }

    fn with_api_root(
        api_key: impl Into<String>,
        api_root: impl Into<String>,
    ) -> Result<Self, ApiError> {
        let http = Client::builder()
            .user_agent(concat!("Ankunft/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .redirect(Policy::none())
            .build()?;

        Ok(Self {
            http,
            api_root: api_root.into(),
            api_key: api_key.into(),
            read_requests: Mutex::new(VecDeque::new()),
            add_requests: Mutex::new(VecDeque::new()),
            carrier_cache: Mutex::new(None),
        })
    }

    pub fn deliveries(&self, filter: FilterMode) -> Result<Vec<Delivery>, ApiError> {
        let carriers = self.supported_carriers()?;
        self.reserve_read_request()?;
        let response: DeliveriesResponse = self
            .http
            .get(format!("{}/deliveries/", self.api_root))
            .query(&[("filter_mode", filter.as_str())])
            .header("api-key", &self.api_key)
            .send()?
            .error_for_status()?
            .json()?;

        if !response.success {
            return Err(ApiError::Parcel(
                response
                    .error_message
                    .unwrap_or_else(|| "Unbekannter API-Fehler".into()),
            ));
        }

        Ok(response
            .deliveries
            .into_iter()
            .filter_map(|delivery| delivery.into_domain(&carriers))
            .collect())
    }

    pub fn supported_carriers(&self) -> Result<SupportedCarriersResponse, ApiError> {
        if let Some(carriers) = self
            .carrier_cache
            .lock()
            .expect("carrier cache mutex poisoned")
            .clone()
        {
            return Ok(carriers);
        }

        let carriers: SupportedCarriersResponse = self
            .http
            .get(format!("{}/supported_carriers.json", self.api_root))
            .send()?
            .error_for_status()?
            .json()?;

        *self
            .carrier_cache
            .lock()
            .expect("carrier cache mutex poisoned") = Some(carriers.clone());
        Ok(carriers)
    }

    pub fn add_delivery(&self, delivery: &NewDelivery) -> Result<(), ApiError> {
        self.reserve_add_request()?;
        let response: serde_json::Value = self
            .http
            .post(format!("{}/add-delivery/", self.api_root))
            .header("api-key", &self.api_key)
            .json(delivery)
            .send()?
            .error_for_status()?
            .json()?;

        if response.get("success").and_then(|value| value.as_bool()) != Some(true) {
            let message = response
                .get("error_message")
                .and_then(|value| value.as_str())
                .unwrap_or("Unbekannter API-Fehler");
            return Err(ApiError::Parcel(message.into()));
        }

        Ok(())
    }

    fn reserve_read_request(&self) -> Result<(), ApiError> {
        reserve_request(
            &self.read_requests,
            MAX_READ_REQUESTS_PER_HOUR,
            Duration::from_secs(3600),
        )
    }

    fn reserve_add_request(&self) -> Result<(), ApiError> {
        reserve_request(
            &self.add_requests,
            MAX_ADD_REQUESTS_PER_DAY,
            Duration::from_secs(24 * 3600),
        )
    }
}

fn reserve_request(
    ledger: &Mutex<VecDeque<Instant>>,
    limit: usize,
    window: Duration,
) -> Result<(), ApiError> {
    let now = Instant::now();
    let mut requests = ledger.lock().expect("rate limit mutex poisoned");
    while requests
        .front()
        .is_some_and(|timestamp| now.duration_since(*timestamp) >= window)
    {
        requests.pop_front();
    }

    if requests.len() >= limit {
        return Err(ApiError::LocalRateLimit);
    }

    requests.push_back(now);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_mode_matches_the_documented_query_values() {
        assert_eq!(FilterMode::Active.as_str(), "active");
        assert_eq!(FilterMode::Recent.as_str(), "recent");
    }

    #[test]
    fn blocks_the_twenty_first_read_request() {
        let client = ParcelClient::new("test-key").expect("client");
        for _ in 0..20 {
            assert!(client.reserve_read_request().is_ok());
        }
        assert!(matches!(
            client.reserve_read_request(),
            Err(ApiError::LocalRateLimit)
        ));
    }
}
