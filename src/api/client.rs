use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::blocking::Client;
use reqwest::header::HeaderValue;
use reqwest::redirect::Policy;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use thiserror::Error;

use crate::{
    i18n::{interpolate, tr},
    model::Delivery,
    rate_limit::{RateLimitError, RateLimiter, RequestKind},
};

use super::types::{DeliveriesResponse, SupportedCarriersResponse};

const API_ROOT: &str = "https://api.parcel.app/external";
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
    #[error("Der lokale Schutz des Parcel-API-Limits ist nicht verfügbar: {0}")]
    RateLimitState(#[source] RateLimitError),
    #[error("Der API-Schlüssel hat kein gültiges HTTP-Headerformat.")]
    InvalidApiKeyHeader,
    #[error("Parcel hat die Anfrage abgelehnt: {0}")]
    Parcel(String),
    #[error("Netzwerkfehler: {0}")]
    Network(#[from] reqwest::Error),
}

impl ApiError {
    pub fn localized_message(&self) -> String {
        match self {
            Self::LocalRateLimit => {
                tr("Das lokale Abruflimit ist erreicht. Bitte später erneut versuchen.")
            }
            Self::RateLimitState(error) => {
                let error = error.localized_message();
                interpolate(
                    tr("Der lokale Schutz des Parcel-API-Limits ist nicht verfügbar: {0}"),
                    &[("0", &error)],
                )
            }
            Self::InvalidApiKeyHeader => {
                tr("Der API-Schlüssel hat kein gültiges HTTP-Headerformat.")
            }
            Self::Parcel(error) => {
                interpolate(tr("Parcel hat die Anfrage abgelehnt: {0}"), &[("0", error)])
            }
            Self::Network(error) => {
                let error = error.to_string();
                interpolate(tr("Netzwerkfehler: {0}"), &[("0", &error)])
            }
        }
    }
}

pub struct ParcelClient {
    http: Client,
    api_root: String,
    api_key: SecretString,
    rate_limiter: Arc<RateLimiter>,
    carrier_cache: Mutex<Option<SupportedCarriersResponse>>,
}

impl ParcelClient {
    pub fn new(api_key: SecretString) -> Result<Self, ApiError> {
        Self::with_api_root(api_key, API_ROOT)
    }

    fn with_api_root(api_key: SecretString, api_root: impl Into<String>) -> Result<Self, ApiError> {
        let rate_limiter = RateLimiter::shared_default().map_err(ApiError::RateLimitState)?;
        let _ = rate_limiter
            .status(RequestKind::Read)
            .map_err(ApiError::RateLimitState)?;
        let http = Client::builder()
            .user_agent(concat!("Ankunft/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .redirect(Policy::none())
            .build()?;

        Ok(Self {
            http,
            api_root: api_root.into(),
            api_key,
            rate_limiter,
            carrier_cache: Mutex::new(None),
        })
    }

    pub fn deliveries(&self, filter: FilterMode) -> Result<Vec<Delivery>, ApiError> {
        let api_key_header = self.sensitive_api_key_header()?;
        let carriers = self.supported_carriers()?;
        self.reserve_read_request()?;
        let response: DeliveriesResponse = self
            .http
            .get(format!("{}/deliveries/", self.api_root))
            .query(&[("filter_mode", filter.as_str())])
            .header("api-key", api_key_header)
            .send()?
            .error_for_status()?
            .json()?;

        if !response.success {
            return Err(ApiError::Parcel(
                response
                    .error_message
                    .unwrap_or_else(|| tr("Unbekannter API-Fehler")),
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
        let api_key_header = self.sensitive_api_key_header()?;
        self.reserve_add_request()?;
        let response: serde_json::Value = self
            .http
            .post(format!("{}/add-delivery/", self.api_root))
            .header("api-key", api_key_header)
            .json(delivery)
            .send()?
            .error_for_status()?
            .json()?;

        if response.get("success").and_then(|value| value.as_bool()) != Some(true) {
            let message = response
                .get("error_message")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| tr("Unbekannter API-Fehler"));
            return Err(ApiError::Parcel(message));
        }

        Ok(())
    }

    fn reserve_read_request(&self) -> Result<(), ApiError> {
        self.reserve_request(RequestKind::Read)
    }

    fn reserve_add_request(&self) -> Result<(), ApiError> {
        self.reserve_request(RequestKind::Add)
    }

    fn reserve_request(&self, kind: RequestKind) -> Result<(), ApiError> {
        match self.rate_limiter.reserve(kind) {
            Ok(_) => Ok(()),
            Err(RateLimitError::LimitReached { .. }) => Err(ApiError::LocalRateLimit),
            Err(error) => Err(ApiError::RateLimitState(error)),
        }
    }

    fn sensitive_api_key_header(&self) -> Result<HeaderValue, ApiError> {
        let mut value = HeaderValue::from_str(self.api_key.expose_secret())
            .map_err(|_| ApiError::InvalidApiKeyHeader)?;
        value.set_sensitive(true);
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_mode_matches_the_documented_query_values() {
        assert_eq!(FilterMode::Active.as_str(), "active");
        assert_eq!(FilterMode::Recent.as_str(), "recent");
    }
}
