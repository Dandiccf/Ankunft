//! Privacy-preserving GNOME notifications for important delivery transitions.
//!
//! This module intentionally derives notifications only from two complete
//! snapshots. A missing old snapshot, or a delivery that did not exist in the
//! old snapshot, establishes a baseline and never produces a notification.
//! Notification identifiers and action targets contain only a deterministic
//! hash; tracking numbers are never placed in notification content or IDs.

use std::collections::{HashMap, HashSet};

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib::prelude::IsA;

use crate::{
    app::APP_ID,
    i18n::{interpolate, tr, trn},
    model::{Delivery, DeliveryStatus},
    storage::DeliverySnapshot,
};

/// Name of the application action invoked when a notification is opened.
/// Register this without a parameter; it presents the existing app window.
pub const OPEN_NOTIFICATION_ACTION: &str = "open-notification";

const DETAILED_OPEN_NOTIFICATION_ACTION: &str = "app.open-notification";
const BUNDLED_NOTIFICATION_ID: &str = "ankunft-delivery-updates";

/// Result of one notification evaluation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotificationOutcome {
    /// Number of important delivery transitions represented by the result.
    pub changed_deliveries: usize,
    /// Number of native notifications sent (zero or one).
    pub notifications_sent: usize,
}

/// Detects important status transitions and sends at most one GNOME
/// notification.
///
/// The first import is silent. Newly appearing deliveries are also treated as
/// a baseline, because their earlier state is unknown. Only transitions *to*
/// `Delivered`, `ReadyForPickup`, `OutForDelivery`, `FailedAttempt`, or
/// `Exception` are reported. Multiple transitions are combined into one
/// notification.
///
/// The caller should persist `current` as the next snapshot after invoking
/// this function. Supplying that snapshot on the next refresh prevents an
/// unchanged status from being reported again.
pub fn notify_delivery_status_changes(
    application: &impl IsA<gio::Application>,
    previous: Option<&DeliverySnapshot>,
    current: &[Delivery],
) -> NotificationOutcome {
    let plan = build_notification_plan(previous, current);
    let Some(payload) = plan.payload.as_ref() else {
        return NotificationOutcome::default();
    };

    let notification = gio::Notification::new(&payload.title);
    notification.set_body(Some(&payload.body));
    notification.set_icon(&gio::ThemedIcon::new(APP_ID));
    notification.set_default_action(DETAILED_OPEN_NOTIFICATION_ACTION);

    application
        .as_ref()
        .send_notification(Some(&payload.id), &notification);

    NotificationOutcome {
        changed_deliveries: plan.changed_deliveries,
        notifications_sent: 1,
    }
}

struct NotificationPlan {
    changed_deliveries: usize,
    payload: Option<NotificationPayload>,
}

struct NotificationPayload {
    id: String,
    title: String,
    body: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct DeliveryKey {
    carrier_code: String,
    tracking_number: String,
}

impl DeliveryKey {
    fn from_delivery(delivery: &Delivery) -> Self {
        Self {
            carrier_code: delivery.carrier_code.trim().to_owned(),
            tracking_number: delivery.tracking_number.trim().to_owned(),
        }
    }

    fn opaque_token(&self) -> String {
        // FNV-1a is implemented locally instead of using DefaultHasher so the
        // token remains stable across Rust releases. This token is an opaque
        // identifier, not a cryptographic authentication mechanism.
        const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;

        fn feed(mut hash: u64, bytes: &[u8]) -> u64 {
            const PRIME: u64 = 0x0000_0100_0000_01b3;
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(PRIME);
            }
            hash
        }

        let carrier = self.carrier_code.as_bytes();
        let tracking = self.tracking_number.as_bytes();
        let mut hash = feed(OFFSET_BASIS, b"io.github.dandiccf.Ankunft/delivery/v1");
        hash = feed(hash, &(carrier.len() as u64).to_le_bytes());
        hash = feed(hash, carrier);
        hash = feed(hash, &(tracking.len() as u64).to_le_bytes());
        hash = feed(hash, tracking);

        // Mix in the standard FNV prime once more after the length-delimited
        // input. Formatting only this value guarantees that cleartext never
        // reaches a notification identifier or action target.
        format!("{:016x}", hash.wrapping_mul(PRIME))
    }
}

struct StatusChange {
    key: DeliveryKey,
    status: DeliveryStatus,
}

fn build_notification_plan(
    previous: Option<&DeliverySnapshot>,
    current: &[Delivery],
) -> NotificationPlan {
    let Some(previous) = previous else {
        return empty_plan();
    };

    let old_statuses = previous
        .deliveries
        .iter()
        .map(|delivery| (DeliveryKey::from_delivery(delivery), delivery.status))
        .collect::<HashMap<_, _>>();

    let mut seen_current = HashSet::with_capacity(current.len());
    let mut changes = Vec::new();

    for delivery in current {
        let key = DeliveryKey::from_delivery(delivery);
        if !seen_current.insert(key.clone()) {
            continue;
        }

        let Some(old_status) = old_statuses.get(&key) else {
            // The delivery is new to this snapshot. Without a known previous
            // status, notifying could create a noisy first-import experience.
            continue;
        };

        if *old_status != delivery.status && is_notifiable(delivery.status) {
            changes.push(StatusChange {
                key,
                status: delivery.status,
            });
        }
    }

    match changes.as_slice() {
        [] => empty_plan(),
        [change] => NotificationPlan {
            changed_deliveries: 1,
            payload: Some(single_payload(change)),
        },
        _ => NotificationPlan {
            changed_deliveries: changes.len(),
            payload: Some(NotificationPayload {
                id: BUNDLED_NOTIFICATION_ID.to_owned(),
                title: interpolate(
                    trn(
                        "{} wichtige Sendungsänderung",
                        "{} wichtige Sendungsänderungen",
                        changes.len() as u64,
                    ),
                    &[("", &changes.len().to_string())],
                ),
                body: tr("Öffne Ankunft, um die Änderungen anzusehen."),
            }),
        },
    }
}

fn empty_plan() -> NotificationPlan {
    NotificationPlan {
        changed_deliveries: 0,
        payload: None,
    }
}

fn is_notifiable(status: DeliveryStatus) -> bool {
    matches!(
        status,
        DeliveryStatus::Delivered
            | DeliveryStatus::ReadyForPickup
            | DeliveryStatus::OutForDelivery
            | DeliveryStatus::FailedAttempt
            | DeliveryStatus::Exception
    )
}

fn single_payload(change: &StatusChange) -> NotificationPayload {
    let (title, body) = match change.status {
        DeliveryStatus::Delivered => (
            tr("Sendung zugestellt"),
            tr("Eine Sendung wurde erfolgreich zugestellt."),
        ),
        DeliveryStatus::ReadyForPickup => (
            tr("Sendung abholbereit"),
            tr("Eine Sendung wartet auf ihre Abholung."),
        ),
        DeliveryStatus::OutForDelivery => (
            tr("Sendung in Zustellung"),
            tr("Eine Sendung ist auf dem Weg zur Zustelladresse."),
        ),
        DeliveryStatus::FailedAttempt => (
            tr("Zustellversuch fehlgeschlagen"),
            tr("Eine Sendung konnte nicht zugestellt werden."),
        ),
        DeliveryStatus::Exception => (
            tr("Problem bei einer Sendung"),
            tr("Eine Sendung benötigt möglicherweise Aufmerksamkeit."),
        ),
        _ => unreachable!("single_payload is only called for notifiable statuses"),
    };

    let token = change.key.opaque_token();
    NotificationPayload {
        id: format!("ankunft-delivery-{token}"),
        title,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeliveryEvent;

    const SECRET_TRACKING_NUMBER: &str = "SECRET-TRACKING-123456";

    fn delivery(status: DeliveryStatus) -> Delivery {
        Delivery {
            carrier_code: "at".to_owned(),
            carrier_name: "Österreichische Post".to_owned(),
            description: format!("Bestellung {SECRET_TRACKING_NUMBER}"),
            tracking_number: SECRET_TRACKING_NUMBER.to_owned(),
            status,
            expected: None,
            expected_detail: None,
            expected_timestamp: None,
            expected_end_timestamp: None,
            extra_information: None,
            last_update: "Gerade eben".to_owned(),
            events: Vec::<DeliveryEvent>::new(),
        }
    }

    fn snapshot(deliveries: Vec<Delivery>) -> DeliverySnapshot {
        DeliverySnapshot {
            fetched_at_unix_secs: 1,
            deliveries,
        }
    }

    #[test]
    fn first_import_is_silent() {
        let plan = build_notification_plan(None, &[delivery(DeliveryStatus::Delivered)]);

        assert_eq!(plan.changed_deliveries, 0);
        assert!(plan.payload.is_none());
    }

    #[test]
    fn newly_appearing_delivery_is_a_silent_baseline() {
        let previous = snapshot(Vec::new());
        let plan =
            build_notification_plan(Some(&previous), &[delivery(DeliveryStatus::OutForDelivery)]);

        assert_eq!(plan.changed_deliveries, 0);
        assert!(plan.payload.is_none());
    }

    #[test]
    fn unchanged_status_is_not_reported_again() {
        let previous = snapshot(vec![delivery(DeliveryStatus::Delivered)]);
        let plan = build_notification_plan(Some(&previous), &[delivery(DeliveryStatus::Delivered)]);

        assert_eq!(plan.changed_deliveries, 0);
        assert!(plan.payload.is_none());
    }

    #[test]
    fn only_selected_destination_statuses_are_notifiable() {
        let important = [
            DeliveryStatus::Delivered,
            DeliveryStatus::ReadyForPickup,
            DeliveryStatus::OutForDelivery,
            DeliveryStatus::FailedAttempt,
            DeliveryStatus::Exception,
        ];
        let ignored = [
            DeliveryStatus::Frozen,
            DeliveryStatus::InTransit,
            DeliveryStatus::NotFound,
            DeliveryStatus::InformationReceived,
            DeliveryStatus::Unknown(42),
        ];

        for status in important {
            let previous = snapshot(vec![delivery(DeliveryStatus::InTransit)]);
            let plan = build_notification_plan(Some(&previous), &[delivery(status)]);
            assert_eq!(plan.changed_deliveries, 1, "status: {status:?}");
            assert!(plan.payload.is_some(), "status: {status:?}");
        }

        for status in ignored {
            let previous = snapshot(vec![delivery(DeliveryStatus::OutForDelivery)]);
            let plan = build_notification_plan(Some(&previous), &[delivery(status)]);
            assert_eq!(plan.changed_deliveries, 0, "status: {status:?}");
            assert!(plan.payload.is_none(), "status: {status:?}");
        }
    }

    #[test]
    fn single_notification_contains_no_tracking_number() {
        let previous = snapshot(vec![delivery(DeliveryStatus::InTransit)]);
        let plan =
            build_notification_plan(Some(&previous), &[delivery(DeliveryStatus::OutForDelivery)]);
        let payload = plan.payload.expect("one transition should be reported");

        for value in [
            payload.id.as_str(),
            payload.title.as_str(),
            payload.body.as_str(),
        ] {
            assert!(!value.contains(SECRET_TRACKING_NUMBER));
        }
    }

    #[test]
    fn opaque_identifier_is_stable_and_scoped_to_carrier_and_tracking() {
        let original = DeliveryKey::from_delivery(&delivery(DeliveryStatus::InTransit));
        let same = DeliveryKey::from_delivery(&delivery(DeliveryStatus::Delivered));
        assert_eq!(original.opaque_token(), same.opaque_token());

        let mut other_carrier = delivery(DeliveryStatus::Delivered);
        other_carrier.carrier_code = "dhl".to_owned();
        let other_carrier = DeliveryKey::from_delivery(&other_carrier);
        assert_ne!(original.opaque_token(), other_carrier.opaque_token());

        let mut other_tracking = delivery(DeliveryStatus::Delivered);
        other_tracking.tracking_number = "DIFFERENT-TRACKING".to_owned();
        let other_tracking = DeliveryKey::from_delivery(&other_tracking);
        assert_ne!(original.opaque_token(), other_tracking.opaque_token());
    }

    #[test]
    fn duplicate_current_rows_produce_only_one_change() {
        let previous = snapshot(vec![delivery(DeliveryStatus::InTransit)]);
        let current = delivery(DeliveryStatus::Delivered);
        let plan = build_notification_plan(Some(&previous), &[current.clone(), current]);

        assert_eq!(plan.changed_deliveries, 1);
        assert!(plan.payload.is_some());
    }

    #[test]
    fn multiple_changes_are_bundled_without_delivery_details() {
        let first_old = delivery(DeliveryStatus::InTransit);
        let mut second_old = delivery(DeliveryStatus::InTransit);
        second_old.carrier_code = "dhl".to_owned();
        second_old.tracking_number = "SECOND-PRIVATE-NUMBER".to_owned();

        let mut first_new = first_old.clone();
        first_new.status = DeliveryStatus::Delivered;
        let mut second_new = second_old.clone();
        second_new.status = DeliveryStatus::Exception;

        let previous = snapshot(vec![first_old, second_old]);
        let plan = build_notification_plan(Some(&previous), &[first_new, second_new]);
        let payload = plan.payload.expect("changes should be bundled");

        assert_eq!(plan.changed_deliveries, 2);
        assert_eq!(payload.id, BUNDLED_NOTIFICATION_ID);
        assert!(payload.title.starts_with('2'));
        assert!(!payload.body.contains(SECRET_TRACKING_NUMBER));
        assert!(!payload.body.contains("SECOND-PRIVATE-NUMBER"));
    }
}
