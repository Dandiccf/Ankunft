//! Obtain background permission from the desktop before hiding a Flatpak window.
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use glib::variant::ToVariant;
use gtk::{gio, glib};

use crate::i18n::tr;

pub async fn request_permission() -> bool {
    if !std::path::Path::new("/.flatpak-info").exists() {
        return true;
    }
    let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
        return false;
    };
    let Some(sender) = bus.unique_name() else {
        return false;
    };
    let token = format!("ankunft_{}", std::process::id());
    let path = format!(
        "/org/freedesktop/portal/desktop/request/{}/{}",
        sender.trim_start_matches(':').replace('.', "_"),
        token,
    );
    let (send, receive) = futures_channel::oneshot::channel();
    let send = Rc::new(RefCell::new(Some(send)));
    let response_send = send.clone();
    let _subscription = bus.subscribe_to_signal(
        Some("org.freedesktop.portal.Desktop"),
        Some("org.freedesktop.portal.Request"),
        Some("Response"),
        Some(&path),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let allowed = permission_granted(signal.parameters);
            if let Some(send) = response_send.borrow_mut().take() {
                let _ = send.send(allowed);
            }
        },
    );
    let options = HashMap::from([
        ("handle_token", token.to_variant()),
        (
            "reason",
            tr("Sendungen prüfen und über wichtige Änderungen informieren.").to_variant(),
        ),
        ("autostart", false.to_variant()),
    ]);
    let request = bus
        .call_future(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Background",
            "RequestBackground",
            Some(&("", options).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            30_000,
        )
        .await;
    if request.is_err() {
        return false;
    }
    let timeout_send = send.clone();
    let timeout = glib::timeout_add_seconds_local_once(120, move || {
        if let Some(send) = timeout_send.borrow_mut().take() {
            let _ = send.send(false);
        }
    });
    let allowed = receive.await.unwrap_or(false);
    if glib::MainContext::default()
        .find_source_by_id(&timeout)
        .is_some()
    {
        timeout.remove();
    }
    if !allowed {
        let _ = bus
            .call_future(
                Some("org.freedesktop.portal.Desktop"),
                &path,
                "org.freedesktop.portal.Request",
                "Close",
                None,
                None,
                gio::DBusCallFlags::NONE,
                5_000,
            )
            .await;
    }
    allowed
}

fn permission_granted(response: &glib::Variant) -> bool {
    response
        .get::<(u32, HashMap<String, glib::Variant>)>()
        .is_some_and(|(code, results)| {
            code == 0
                && results
                    .get("background")
                    .and_then(|value| value.get::<bool>())
                    == Some(true)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_requires_explicit_successful_permission() {
        let granted = HashMap::from([("background", true.to_variant())]);
        assert!(permission_granted(&(0_u32, granted.clone()).to_variant()));
        assert!(!permission_granted(&(1_u32, granted).to_variant()));
        assert!(!permission_granted(
            &(0_u32, HashMap::<String, glib::Variant>::new()).to_variant()
        ));
        assert!(!permission_granted(&"malformed".to_variant()));
        assert!(!permission_granted(
            &(0_u32, HashMap::from([("background", false.to_variant())])).to_variant()
        ));
    }
}
