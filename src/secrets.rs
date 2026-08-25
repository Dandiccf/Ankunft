use std::collections::HashMap;

use gtk::glib::prelude::Cast;
use secrecy::{ExposeSecret, SecretString};
use secret::prelude::*;
use secret::{Schema, SchemaAttributeType, SchemaFlags};
use thiserror::Error;
use zeroize::Zeroize;

use crate::i18n::tr;

const SCHEMA_NAME: &str = "io.github.dandiccf.Ankunft.Credentials";
const SECRET_KIND: &str = "parcel-api-key";
const ITEM_LABEL: &str = "Ankunft – Parcel API-Schlüssel";

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("Der API-Schlüssel ist leer oder enthält ungültige Zeichen.")]
    InvalidKey,
    #[error("Der gespeicherte API-Schlüssel hat ein ungültiges Format.")]
    InvalidStoredKey,
    #[error("Der Zugriff auf den Schlüsselbund wurde abgebrochen.")]
    Cancelled,
    #[error("Der GNOME-Schlüsselbund ist gesperrt.")]
    Locked,
    #[error("Der Zugriff auf den GNOME-Schlüsselbund wurde verweigert.")]
    Denied(#[source] gtk::glib::Error),
    #[error("Der GNOME-Schlüsselbund ist nicht verfügbar.")]
    Unavailable(#[source] gtk::glib::Error),
    #[error("Der GNOME-Schlüsselbund hat einen nicht unterstützten Eintrag geliefert.")]
    UnsupportedItem,
    #[error("Der API-Schlüssel konnte nicht sicher verarbeitet werden.")]
    Backend(#[source] gtk::glib::Error),
}

impl SecretError {
    pub fn localized_message(&self) -> String {
        tr(match self {
            Self::InvalidKey => "Der API-Schlüssel ist leer oder enthält ungültige Zeichen.",
            Self::InvalidStoredKey => "Der gespeicherte API-Schlüssel hat ein ungültiges Format.",
            Self::Cancelled => "Der Zugriff auf den Schlüsselbund wurde abgebrochen.",
            Self::Locked => "Der GNOME-Schlüsselbund ist gesperrt.",
            Self::Denied(_) => "Der Zugriff auf den GNOME-Schlüsselbund wurde verweigert.",
            Self::Unavailable(_) => "Der GNOME-Schlüsselbund ist nicht verfügbar.",
            Self::UnsupportedItem => {
                "Der GNOME-Schlüsselbund hat einen nicht unterstützten Eintrag geliefert."
            }
            Self::Backend(_) => "Der API-Schlüssel konnte nicht sicher verarbeitet werden.",
        })
    }
}

pub fn parse_api_key(raw: &str) -> Result<SecretString, SecretError> {
    if raw.contains(['\0', '\r', '\n']) {
        return Err(SecretError::InvalidKey);
    }

    let api_key = raw.trim();
    if api_key.is_empty() {
        return Err(SecretError::InvalidKey);
    }

    Ok(SecretString::from(api_key))
}

pub fn parse_api_key_owned(mut raw: String) -> Result<SecretString, SecretError> {
    if raw.contains(['\0', '\r', '\n']) {
        raw.zeroize();
        return Err(SecretError::InvalidKey);
    }

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        raw.zeroize();
        return Err(SecretError::InvalidKey);
    }

    if trimmed.len() == raw.len() {
        Ok(SecretString::from(raw))
    } else {
        let normalized = trimmed.to_owned();
        raw.zeroize();
        Ok(SecretString::from(normalized))
    }
}

pub async fn load_api_key() -> Result<Option<SecretString>, SecretError> {
    let schema = schema();
    let attributes = attributes();
    let matches = secret::password_search_future(
        Some(&schema),
        attributes,
        secret::SearchFlags::ALL | secret::SearchFlags::UNLOCK | secret::SearchFlags::LOAD_SECRETS,
    )
    .await
    .map_err(classify_error)?;

    let Some(item) = matches.into_iter().max_by_key(|item| item.modified()) else {
        return Ok(None);
    };
    let Some(value) = item
        .retrieve_secret_future()
        .await
        .map_err(classify_error)?
    else {
        return Err(SecretError::InvalidStoredKey);
    };

    let mut bytes = value.get();
    let api_key = std::str::from_utf8(&bytes)
        .map_err(|_| SecretError::InvalidStoredKey)
        .and_then(|raw| parse_api_key(raw).map_err(|_| SecretError::InvalidStoredKey));
    bytes.zeroize();

    api_key.map(Some)
}

pub async fn store_api_key(api_key: &SecretString) -> Result<(), SecretError> {
    let schema = schema();
    let attributes = attributes();
    let item_label = tr(ITEM_LABEL);
    secret::password_store_future(
        Some(&schema),
        attributes,
        Some(secret::COLLECTION_DEFAULT.as_str()),
        &item_label,
        api_key.expose_secret(),
    )
    .await
    .map_err(classify_error)?;
    Ok(())
}

pub async fn clear_api_key() -> Result<(), SecretError> {
    let schema = schema();
    let attributes = attributes();
    let matches = secret::password_search_future(
        Some(&schema),
        attributes,
        secret::SearchFlags::ALL | secret::SearchFlags::UNLOCK,
    )
    .await
    .map_err(classify_error)?;

    let mut items = Vec::with_capacity(matches.len());
    for retrievable in matches {
        let item = retrievable
            .downcast::<secret::Item>()
            .map_err(|_| SecretError::UnsupportedItem)?;
        if item.is_locked() {
            return Err(SecretError::Locked);
        }
        items.push(item);
    }

    for item in items {
        item.delete_future().await.map_err(classify_error)?;
    }

    Ok(())
}

fn schema() -> Schema {
    Schema::new(
        SCHEMA_NAME,
        SchemaFlags::NONE,
        HashMap::from([("kind", SchemaAttributeType::String)]),
    )
}

fn attributes() -> HashMap<&'static str, &'static str> {
    HashMap::from([("kind", SECRET_KIND)])
}

fn classify_error(error: gtk::glib::Error) -> SecretError {
    use gtk::gio::{DBusError, IOErrorEnum};

    if error.matches(IOErrorEnum::Cancelled) {
        SecretError::Cancelled
    } else if error.matches(secret::Error::IsLocked) {
        SecretError::Locked
    } else if error.matches(IOErrorEnum::PermissionDenied)
        || error.matches(DBusError::AccessDenied)
        || error.matches(DBusError::AuthFailed)
    {
        SecretError::Denied(error)
    } else if error.matches(DBusError::ServiceUnknown)
        || error.matches(DBusError::NameHasNoOwner)
        || error.matches(DBusError::NoServer)
        || error.matches(DBusError::Disconnected)
        || error.matches(DBusError::SpawnServiceNotFound)
        || error.matches(IOErrorEnum::NotConnected)
    {
        SecretError::Unavailable(error)
    } else {
        SecretError::Backend(error)
    }
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn accepts_and_trims_an_api_key() {
        let key = parse_api_key("  secret-value  ").expect("valid key");
        assert_eq!(key.expose_secret(), "secret-value");
    }

    #[test]
    fn rejects_empty_and_multiline_values() {
        assert!(matches!(parse_api_key("   "), Err(SecretError::InvalidKey)));
        assert!(matches!(
            parse_api_key("first\nsecond"),
            Err(SecretError::InvalidKey)
        ));
    }

    #[test]
    fn owned_parser_moves_a_trimmed_value_into_secret_storage() {
        let key = parse_api_key_owned("  secret-value  ".to_owned()).expect("valid key");
        assert_eq!(key.expose_secret(), "secret-value");
    }
}
