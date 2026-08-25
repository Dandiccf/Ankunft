use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};

use crate::{
    i18n::tr,
    model::{Delivery, DeliveryEvent, DeliveryStatus},
};

#[derive(Debug, Deserialize)]
pub(crate) struct DeliveriesResponse {
    pub success: bool,
    pub error_message: Option<String>,
    #[serde(default)]
    pub deliveries: Vec<ApiDelivery>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiDelivery {
    pub carrier_code: String,
    pub description: String,
    pub status_code: i32,
    pub tracking_number: String,
    #[serde(default)]
    pub events: Vec<ApiEvent>,
    pub date_expected: Option<String>,
    pub date_expected_end: Option<String>,
    pub timestamp_expected: Option<i64>,
    pub timestamp_expected_end: Option<i64>,
    pub extra_information: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiEvent {
    pub event: String,
    pub date: String,
    pub location: Option<String>,
    pub additional: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SupportedCarrier {
    pub name: String,
    #[serde(default)]
    pub name_variations: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "deserialize_optional_integer")]
    pub extra_required: Option<u16>,
}

pub(crate) type SupportedCarriersResponse = BTreeMap<String, SupportedCarrier>;

impl ApiDelivery {
    pub(crate) fn into_domain(self, carriers: &SupportedCarriersResponse) -> Option<Delivery> {
        let status = DeliveryStatus::from_api_code(self.status_code);
        let carrier_name = carriers
            .get(&self.carrier_code)
            .map(|carrier| carrier.name.clone())
            .unwrap_or_else(|| self.carrier_code.to_uppercase());
        let last_update = self
            .events
            .first()
            .map(|event| event.date.clone())
            .unwrap_or_else(|| tr("Noch keine Aktualisierung"));

        Some(Delivery {
            carrier_code: self.carrier_code,
            carrier_name,
            description: self.description,
            tracking_number: self.tracking_number,
            status,
            expected: self.date_expected,
            expected_detail: self.date_expected_end,
            expected_timestamp: self.timestamp_expected,
            expected_end_timestamp: self.timestamp_expected_end,
            extra_information: self.extra_information,
            last_update,
            events: self
                .events
                .into_iter()
                .map(|event| DeliveryEvent {
                    title: event.event,
                    date: event.date,
                    location: event.location,
                    additional: event.additional,
                })
                .collect(),
        })
    }
}

fn deserialize_optional_integer<'de, D>(deserializer: D) -> Result<Option<u16>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntegerValue {
        Number(u16),
        Text(String),
    }

    let value = Option::<IntegerValue>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(IntegerValue::Number(value)) => Ok(Some(value)),
        Some(IntegerValue::Text(value)) => value
            .parse::<u16>()
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_delivery_payload_and_keeps_unknown_status() {
        let response: DeliveriesResponse = serde_json::from_str(
            r#"{
                "success": true,
                "deliveries": [{
                    "carrier_code": "at",
                    "description": "Testpaket",
                    "status_code": 42,
                    "tracking_number": "TEST123",
                    "events": [{
                        "event": "Unterwegs",
                        "date": "Heute, 10:00",
                        "location": "Wien"
                    }],
                    "date_expected": "Morgen",
                    "timestamp_expected": 1787652000,
                    "extra_information": "1010"
                }]
            }"#,
        )
        .expect("valid response");

        let carriers = BTreeMap::from([(
            "at".to_string(),
            SupportedCarrier {
                name: "Austrian Post".into(),
                name_variations: BTreeMap::new(),
                extra_required: None,
            },
        )]);
        let delivery = response.deliveries.into_iter().next().unwrap();
        let delivery = delivery.into_domain(&carriers).expect("domain delivery");

        assert_eq!(delivery.status, DeliveryStatus::Unknown(42));
        assert_eq!(delivery.expected_timestamp, Some(1787652000));
        assert_eq!(delivery.extra_information.as_deref(), Some("1010"));
        assert_eq!(delivery.events[0].location.as_deref(), Some("Wien"));
    }

    #[test]
    fn accepts_numeric_and_text_carrier_requirements() {
        let numeric: SupportedCarrier =
            serde_json::from_str(r#"{"name":"DPD","extra_required":1}"#).unwrap();
        let text: SupportedCarrier =
            serde_json::from_str(r#"{"name":"DPD","extra_required":"5"}"#).unwrap();

        assert_eq!(numeric.extra_required, Some(1));
        assert_eq!(text.extra_required, Some(5));
    }
}
