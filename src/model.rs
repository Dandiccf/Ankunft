use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryStatus {
    Delivered,
    Frozen,
    InTransit,
    ReadyForPickup,
    OutForDelivery,
    NotFound,
    FailedAttempt,
    Exception,
    InformationReceived,
    Unknown(i32),
}

impl DeliveryStatus {
    pub fn from_api_code(code: i32) -> Self {
        match code {
            0 => Self::Delivered,
            1 => Self::Frozen,
            2 => Self::InTransit,
            3 => Self::ReadyForPickup,
            4 => Self::OutForDelivery,
            5 => Self::NotFound,
            6 => Self::FailedAttempt,
            7 => Self::Exception,
            8 => Self::InformationReceived,
            _ => Self::Unknown(code),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Delivered => "Zugestellt",
            Self::Frozen => "Keine Aktualisierung",
            Self::InTransit => "Unterwegs",
            Self::ReadyForPickup => "Abholbereit",
            Self::OutForDelivery => "In Zustellung",
            Self::NotFound => "Nicht gefunden",
            Self::FailedAttempt => "Zustellversuch fehlgeschlagen",
            Self::Exception => "Aufmerksamkeit erforderlich",
            Self::InformationReceived => "Elektronisch angekündigt",
            Self::Unknown(_) => "Unbekannter Status",
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Self::Delivered => "status-delivered",
            Self::ReadyForPickup => "status-pickup",
            Self::OutForDelivery => "status-today",
            Self::Exception | Self::FailedAttempt => "status-problem",
            Self::Frozen | Self::NotFound => "status-muted",
            Self::InTransit | Self::InformationReceived => "status-transit",
            Self::Unknown(_) => "status-muted",
        }
    }

    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Delivered => "emblem-ok-symbolic",
            Self::ReadyForPickup => "folder-download-symbolic",
            Self::OutForDelivery => "send-to-symbolic",
            Self::Exception | Self::FailedAttempt => "dialog-warning-symbolic",
            Self::Frozen | Self::NotFound => "dialog-question-symbolic",
            Self::InTransit => "go-next-symbolic",
            Self::InformationReceived => "mail-unread-symbolic",
            Self::Unknown(_) => "dialog-question-symbolic",
        }
    }

    pub fn progress(self) -> f64 {
        match self {
            Self::InformationReceived => 0.12,
            Self::InTransit => 0.58,
            Self::OutForDelivery => 0.88,
            Self::ReadyForPickup => 0.92,
            Self::Delivered => 1.0,
            Self::Frozen | Self::NotFound | Self::FailedAttempt | Self::Exception => 0.48,
            Self::Unknown(_) => 0.0,
        }
    }

    pub fn is_active(self) -> bool {
        !matches!(self, Self::Delivered | Self::Frozen)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryEvent {
    pub title: String,
    pub date: String,
    pub location: Option<String>,
    pub additional: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    pub carrier_code: String,
    pub carrier_name: String,
    pub description: String,
    pub tracking_number: String,
    pub status: DeliveryStatus,
    pub expected: Option<String>,
    pub expected_detail: Option<String>,
    pub expected_timestamp: Option<i64>,
    pub expected_end_timestamp: Option<i64>,
    pub extra_information: Option<String>,
    pub last_update: String,
    pub events: Vec<DeliveryEvent>,
}

impl Delivery {
    pub fn initials(&self) -> String {
        self.carrier_name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase()
    }

    pub fn matches_query(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.description.to_lowercase().contains(&query)
            || self.carrier_name.to_lowercase().contains(&query)
            || self.tracking_number.to_lowercase().contains(&query)
    }
}

pub fn demo_deliveries() -> Vec<Delivery> {
    vec![
        Delivery {
            carrier_code: "at".into(),
            carrier_name: "Österreichische Post".into(),
            description: "AirPods Zubehör".into(),
            tracking_number: "AT •••• 4821".into(),
            status: DeliveryStatus::OutForDelivery,
            expected: Some("Heute".into()),
            expected_detail: Some("zwischen 12:10 und 14:40 Uhr".into()),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: "Vor 18 Minuten".into(),
            events: vec![
                DeliveryEvent {
                    title: "Sendung ist in Zustellung".into(),
                    date: "Heute, 08:12".into(),
                    location: Some("Wien".into()),
                    additional: Some("Die Zustellung erfolgt voraussichtlich heute.".into()),
                },
                DeliveryEvent {
                    title: "Im Verteilzentrum bearbeitet".into(),
                    date: "Heute, 05:46".into(),
                    location: Some("Logistikzentrum Wien".into()),
                    additional: None,
                },
                DeliveryEvent {
                    title: "Sendung übernommen".into(),
                    date: "Gestern, 17:20".into(),
                    location: Some("Linz".into()),
                    additional: None,
                },
            ],
        },
        Delivery {
            carrier_code: "dpdat".into(),
            carrier_name: "DPD Austria".into(),
            description: "Kaffeebohnen".into(),
            tracking_number: "DPD •••• 7604".into(),
            status: DeliveryStatus::InTransit,
            expected: Some("Morgen".into()),
            expected_detail: Some("bis zum Ende des Tages".into()),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: "Vor 2 Stunden".into(),
            events: vec![
                DeliveryEvent {
                    title: "Unterwegs zum Zieldepot".into(),
                    date: "Heute, 06:32".into(),
                    location: Some("Hörsching".into()),
                    additional: None,
                },
                DeliveryEvent {
                    title: "Paket im DPD-System erfasst".into(),
                    date: "Gestern, 16:05".into(),
                    location: None,
                    additional: None,
                },
            ],
        },
        Delivery {
            carrier_code: "gls".into(),
            carrier_name: "GLS".into(),
            description: "Bücherbestellung".into(),
            tracking_number: "GLS •••• 1938".into(),
            status: DeliveryStatus::ReadyForPickup,
            expected: Some("Abholbereit".into()),
            expected_detail: Some("noch 4 Tage im PaketShop".into()),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: "Gestern".into(),
            events: vec![
                DeliveryEvent {
                    title: "Im GLS PaketShop abholbereit".into(),
                    date: "Gestern, 15:41".into(),
                    location: Some("Wien 7".into()),
                    additional: Some("Bitte Lichtbildausweis mitnehmen.".into()),
                },
                DeliveryEvent {
                    title: "Zustellung nicht möglich".into(),
                    date: "Gestern, 13:18".into(),
                    location: Some("Wien".into()),
                    additional: None,
                },
            ],
        },
        Delivery {
            carrier_code: "dhl".into(),
            carrier_name: "DHL Express".into(),
            description: "Entwicklerboard".into(),
            tracking_number: "DHL •••• 5092".into(),
            status: DeliveryStatus::InformationReceived,
            expected: Some("Freitag".into()),
            expected_detail: Some("Termin wird noch bestätigt".into()),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: "Gestern".into(),
            events: vec![DeliveryEvent {
                title: "Sendungsinformation eingegangen".into(),
                date: "Gestern, 11:24".into(),
                location: None,
                additional: Some("Die Sendung wurde noch nicht an DHL übergeben.".into()),
            }],
        },
        Delivery {
            carrier_code: "ups".into(),
            carrier_name: "UPS".into(),
            description: "Monitorarm".into(),
            tracking_number: "UPS •••• 0447".into(),
            status: DeliveryStatus::Delivered,
            expected: Some("Zugestellt".into()),
            expected_detail: Some("Montag um 10:17 Uhr".into()),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: "Montag".into(),
            events: vec![
                DeliveryEvent {
                    title: "Zugestellt".into(),
                    date: "Montag, 10:17".into(),
                    location: Some("Wien".into()),
                    additional: Some("Beim Empfänger abgegeben.".into()),
                },
                DeliveryEvent {
                    title: "In Zustellung".into(),
                    date: "Montag, 07:31".into(),
                    location: Some("Wien".into()),
                    additional: None,
                },
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_all_documented_status_codes() {
        for code in 0..=8 {
            assert!(!matches!(
                DeliveryStatus::from_api_code(code),
                DeliveryStatus::Unknown(_)
            ));
        }
        assert_eq!(
            DeliveryStatus::from_api_code(42),
            DeliveryStatus::Unknown(42)
        );
    }

    #[test]
    fn searches_description_carrier_and_tracking_number() {
        let delivery = &demo_deliveries()[0];
        assert!(delivery.matches_query("airpods"));
        assert!(delivery.matches_query("post"));
        assert!(delivery.matches_query("4821"));
        assert!(!delivery.matches_query("kaffeebohnen"));
    }
}
