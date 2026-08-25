use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::i18n::tr;

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

    pub fn label(self) -> String {
        tr(match self {
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
        })
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

const MAX_RECENT_DELIVERIES: usize = 200;

/// Combines a freshly fetched Recent view with delivered entries retained in
/// the previous local snapshot.
///
/// Fresh API entries always win. Their order is preserved, duplicate API
/// entries are removed, and only previously delivered entries that are absent
/// from the API response are appended. The retained archive never grows the
/// result beyond 200 entries, unless the unique API response itself is larger.
pub(crate) fn merge_delivered_history(
    current: Vec<Delivery>,
    previous: &[Delivery],
) -> Vec<Delivery> {
    let mut seen = HashSet::with_capacity(current.len().saturating_add(previous.len()));
    let mut merged = Vec::with_capacity(current.len().min(MAX_RECENT_DELIVERIES));

    for delivery in current {
        if seen.insert(delivery_key(&delivery)) {
            merged.push(delivery);
        }
    }

    for delivery in previous
        .iter()
        .filter(|delivery| delivery.status == DeliveryStatus::Delivered)
    {
        if merged.len() >= MAX_RECENT_DELIVERIES {
            break;
        }

        if seen.insert(delivery_key(delivery)) {
            merged.push(delivery.clone());
        }
    }

    merged
}

fn delivery_key(delivery: &Delivery) -> (String, String) {
    (
        delivery.carrier_code.trim().to_owned(),
        delivery.tracking_number.trim().to_owned(),
    )
}

pub fn demo_deliveries() -> Vec<Delivery> {
    vec![
        Delivery {
            carrier_code: "at".into(),
            carrier_name: tr("Österreichische Post"),
            description: tr("AirPods Zubehör"),
            tracking_number: "AT •••• 4821".into(),
            status: DeliveryStatus::OutForDelivery,
            expected: Some(tr("Heute")),
            expected_detail: Some(tr("zwischen 12:10 und 14:40 Uhr")),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: tr("Vor 18 Minuten"),
            events: vec![
                DeliveryEvent {
                    title: tr("Sendung ist in Zustellung"),
                    date: tr("Heute, 08:12"),
                    location: Some(tr("Wien")),
                    additional: Some(tr("Die Zustellung erfolgt voraussichtlich heute.")),
                },
                DeliveryEvent {
                    title: tr("Im Verteilzentrum bearbeitet"),
                    date: tr("Heute, 05:46"),
                    location: Some(tr("Logistikzentrum Wien")),
                    additional: None,
                },
                DeliveryEvent {
                    title: tr("Sendung übernommen"),
                    date: tr("Gestern, 17:20"),
                    location: Some(tr("Linz")),
                    additional: None,
                },
            ],
        },
        Delivery {
            carrier_code: "dpdat".into(),
            carrier_name: "DPD Austria".into(),
            description: tr("Kaffeebohnen"),
            tracking_number: "DPD •••• 7604".into(),
            status: DeliveryStatus::InTransit,
            expected: Some(tr("Morgen")),
            expected_detail: Some(tr("bis zum Ende des Tages")),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: tr("Vor 2 Stunden"),
            events: vec![
                DeliveryEvent {
                    title: tr("Unterwegs zum Zieldepot"),
                    date: tr("Heute, 06:32"),
                    location: Some(tr("Hörsching")),
                    additional: None,
                },
                DeliveryEvent {
                    title: tr("Paket im DPD-System erfasst"),
                    date: tr("Gestern, 16:05"),
                    location: None,
                    additional: None,
                },
            ],
        },
        Delivery {
            carrier_code: "gls".into(),
            carrier_name: "GLS".into(),
            description: tr("Bücherbestellung"),
            tracking_number: "GLS •••• 1938".into(),
            status: DeliveryStatus::ReadyForPickup,
            expected: Some(tr("Abholbereit")),
            expected_detail: Some(tr("noch 4 Tage im PaketShop")),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: tr("Gestern"),
            events: vec![
                DeliveryEvent {
                    title: tr("Im GLS PaketShop abholbereit"),
                    date: tr("Gestern, 15:41"),
                    location: Some(tr("Wien 7")),
                    additional: Some(tr("Bitte Lichtbildausweis mitnehmen.")),
                },
                DeliveryEvent {
                    title: tr("Zustellung nicht möglich"),
                    date: tr("Gestern, 13:18"),
                    location: Some(tr("Wien")),
                    additional: None,
                },
            ],
        },
        Delivery {
            carrier_code: "dhl".into(),
            carrier_name: "DHL Express".into(),
            description: tr("Entwicklerboard"),
            tracking_number: "DHL •••• 5092".into(),
            status: DeliveryStatus::InformationReceived,
            expected: Some(tr("Freitag")),
            expected_detail: Some(tr("Termin wird noch bestätigt")),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: tr("Gestern"),
            events: vec![DeliveryEvent {
                title: tr("Sendungsinformation eingegangen"),
                date: tr("Gestern, 11:24"),
                location: None,
                additional: Some(tr("Die Sendung wurde noch nicht an DHL übergeben.")),
            }],
        },
        Delivery {
            carrier_code: "ups".into(),
            carrier_name: "UPS".into(),
            description: tr("Monitorarm"),
            tracking_number: "UPS •••• 0447".into(),
            status: DeliveryStatus::Delivered,
            expected: Some(tr("Zugestellt")),
            expected_detail: Some(tr("Montag um 10:17 Uhr")),
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: tr("Montag"),
            events: vec![
                DeliveryEvent {
                    title: tr("Zugestellt"),
                    date: tr("Montag, 10:17"),
                    location: Some(tr("Wien")),
                    additional: Some(tr("Beim Empfänger abgegeben.")),
                },
                DeliveryEvent {
                    title: tr("In Zustellung"),
                    date: tr("Montag, 07:31"),
                    location: Some(tr("Wien")),
                    additional: None,
                },
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(
        carrier: &str,
        tracking: &str,
        status: DeliveryStatus,
        description: &str,
    ) -> Delivery {
        Delivery {
            carrier_code: carrier.into(),
            carrier_name: carrier.into(),
            description: description.into(),
            tracking_number: tracking.into(),
            status,
            expected: None,
            expected_detail: None,
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: String::new(),
            events: Vec::new(),
        }
    }

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
        assert!(delivery.matches_query(&tr("Österreichische Post")));
        assert!(delivery.matches_query("4821"));
        assert!(!delivery.matches_query("definitely-not-a-delivery"));
    }

    #[test]
    fn merges_only_missing_delivered_history_and_current_data_wins() {
        let current = vec![delivery("post", "same", DeliveryStatus::InTransit, "fresh")];
        let previous = vec![
            delivery("post", "same", DeliveryStatus::Delivered, "stale"),
            delivery("dhl", "archive", DeliveryStatus::Delivered, "retained"),
            delivery("ups", "active", DeliveryStatus::InTransit, "not retained"),
        ];

        let merged = merge_delivered_history(current, &previous);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].description, "fresh");
        assert_eq!(merged[0].status, DeliveryStatus::InTransit);
        assert_eq!(merged[1].tracking_number, "archive");
    }

    #[test]
    fn removes_duplicate_current_and_archived_entries_by_delivery_key() {
        let current = vec![
            delivery("post", "one", DeliveryStatus::Delivered, "first"),
            delivery("post", "one", DeliveryStatus::Delivered, "duplicate"),
        ];
        let previous = vec![
            delivery("post", "one", DeliveryStatus::Delivered, "old"),
            delivery(" dhl ", " two ", DeliveryStatus::Delivered, "archive"),
            delivery("dhl", "two", DeliveryStatus::Delivered, "archive duplicate"),
        ];

        let merged = merge_delivered_history(current, &previous);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].description, "first");
        assert_eq!(merged[1].description, "archive");
    }

    #[test]
    fn caps_only_the_appended_archive_at_two_hundred_entries() {
        let current = (0..198)
            .map(|index| {
                delivery(
                    "current",
                    &format!("current-{index}"),
                    DeliveryStatus::InTransit,
                    "current",
                )
            })
            .collect();
        let previous = (0..10)
            .map(|index| {
                delivery(
                    "archive",
                    &format!("archive-{index}"),
                    DeliveryStatus::Delivered,
                    "archive",
                )
            })
            .collect::<Vec<_>>();

        let merged = merge_delivered_history(current, &previous);
        assert_eq!(merged.len(), MAX_RECENT_DELIVERIES);
        assert_eq!(merged[199].tracking_number, "archive-1");

        let oversized_current = (0..205)
            .map(|index| {
                delivery(
                    "current",
                    &format!("oversized-{index}"),
                    DeliveryStatus::InTransit,
                    "current",
                )
            })
            .collect();
        let merged = merge_delivered_history(oversized_current, &previous);
        assert_eq!(merged.len(), 205);
    }
}
