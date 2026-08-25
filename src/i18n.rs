//! Small gettext runtime used by Ankunft.
//!
//! Catalogs compiled by `build.rs` are embedded as a reliable development and
//! portable-build fallback. Installed catalogs take precedence, so packaged
//! translations can be updated without rebuilding the binary.

use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const DOMAIN: &str = "ankunft";
const SOURCE_LOCALE: &str = "de";
const MAX_CATALOG_BYTES: u64 = 8 * 1024 * 1024;

include!(concat!(env!("OUT_DIR"), "/ankunft_i18n_catalogs.rs"));

static CATALOG: OnceLock<Option<Catalog>> = OnceLock::new();

/// Loads the best catalog and returns its locale. Calling this explicitly at
/// application startup is optional; all translation functions initialize on
/// first use.
pub fn initialize() -> &'static str {
    active_catalog()
        .map(|catalog| catalog.locale.as_str())
        .unwrap_or(SOURCE_LOCALE)
}

/// Translates one message, falling back to the German source text.
pub fn tr(message: &str) -> String {
    active_catalog()
        .and_then(|catalog| catalog.messages.get(message))
        .filter(|translation| !translation.is_empty())
        .cloned()
        .unwrap_or_else(|| message.to_owned())
}

/// Translates a singular/plural pair and selects the form for `count`.
pub fn trn(singular: &str, plural: &str, count: u64) -> String {
    let source = if count == 1 { singular } else { plural };
    let Some(catalog) = active_catalog() else {
        return source.to_owned();
    };

    let key = format!("{singular}\0{plural}");
    let Some(forms) = catalog.messages.get(&key) else {
        return source.to_owned();
    };
    let index = plural_index(&catalog.locale, count);
    forms
        .split('\0')
        .nth(index)
        .filter(|translation| !translation.is_empty())
        .unwrap_or(source)
        .to_owned()
}

/// Replaces named placeholders such as `{error}` after translation.
/// Passing an empty placeholder name replaces the first `{}` occurrence.
pub fn interpolate(message: impl Into<String>, values: &[(&str, &str)]) -> String {
    let mut rendered = message.into();
    for (name, value) in values {
        if name.is_empty() {
            rendered = rendered.replacen("{}", value, 1);
        } else {
            rendered = rendered.replace(&format!("{{{name}}}"), value);
        }
    }
    rendered
}

fn active_catalog() -> Option<&'static Catalog> {
    CATALOG.get_or_init(load_catalog).as_ref()
}

fn load_catalog() -> Option<Catalog> {
    for locale in requested_locales() {
        if let Some(bytes) = read_installed_catalog(&locale)
            && let Some(catalog) = Catalog::from_mo(locale.clone(), &bytes)
        {
            return Some(catalog);
        }

        if let Some((_, bytes)) = EMBEDDED_CATALOGS
            .iter()
            .find(|(embedded_locale, _)| *embedded_locale == locale)
            && let Some(catalog) = Catalog::from_mo(locale, bytes)
        {
            return Some(catalog);
        }
    }
    None
}

fn requested_locales() -> Vec<String> {
    let effective = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| env::var(name).ok().filter(|value| !value.trim().is_empty()));

    let mut raw = Vec::new();
    if effective
        .as_deref()
        .is_none_or(|locale| !is_untranslated_locale(locale))
        && let Ok(language) = env::var("LANGUAGE")
    {
        raw.extend(language.split(':').map(str::to_owned));
    }
    if let Some(effective) = effective {
        raw.push(effective);
    }

    let mut locales = Vec::new();
    let mut seen = HashSet::new();
    for candidate in raw {
        let Some(normalized) = normalize_locale(&candidate) else {
            continue;
        };
        if seen.insert(normalized.clone()) {
            locales.push(normalized.clone());
        }
        if let Some((language, _)) = normalized.split_once('_') {
            let language = language.to_owned();
            if seen.insert(language.clone()) {
                locales.push(language);
            }
        }
    }
    locales
}

fn normalize_locale(locale: &str) -> Option<String> {
    let locale = locale.trim();
    if locale.is_empty() || is_untranslated_locale(locale) {
        return None;
    }

    let locale = locale
        .split(['.', '@'])
        .next()
        .unwrap_or(locale)
        .replace('-', "_");
    let mut parts = locale.split('_');
    let language = parts.next()?.to_ascii_lowercase();
    if language.len() < 2 || !language.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }

    match parts.next() {
        Some(region)
            if !region.is_empty()
                && region.len() <= 3
                && region.bytes().all(|byte| byte.is_ascii_alphanumeric()) =>
        {
            Some(format!("{language}_{}", region.to_ascii_uppercase()))
        }
        _ => Some(language),
    }
}

fn is_untranslated_locale(locale: &str) -> bool {
    matches!(
        locale.trim().split(['.', '@']).next().unwrap_or_default(),
        "C" | "POSIX"
    )
}

fn read_installed_catalog(locale: &str) -> Option<Vec<u8>> {
    for root in locale_roots() {
        let path = root
            .join(locale)
            .join("LC_MESSAGES")
            .join(format!("{DOMAIN}.mo"));
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_CATALOG_BYTES {
            continue;
        }
        if let Ok(bytes) = fs::read(path) {
            return Some(bytes);
        }
    }
    None
}

fn locale_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = env::var_os("ANKUNFT_LOCALE_DIR") {
        roots.push(PathBuf::from(path));
    }
    if let Some(path) = option_env!("ANKUNFT_LOCALEDIR") {
        roots.push(PathBuf::from(path));
    }
    roots.push(PathBuf::from(env!("ANKUNFT_BUILD_LOCALE_DIR")));

    if let Ok(executable) = env::current_exe()
        && let Some(prefix) = executable.parent().and_then(Path::parent)
    {
        roots.push(prefix.join("share/locale"));
    }
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        roots.push(PathBuf::from(data_home).join("locale"));
    } else if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/locale"));
    }
    if let Some(data_dirs) = env::var_os("XDG_DATA_DIRS") {
        roots.extend(env::split_paths(&data_dirs).map(|path| path.join("locale")));
    }
    roots.push(PathBuf::from("/usr/local/share/locale"));
    roots.push(PathBuf::from("/usr/share/locale"));

    let mut seen = HashSet::new();
    roots
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn plural_index(locale: &str, count: u64) -> usize {
    let language = locale.split('_').next().unwrap_or(locale);
    match language {
        // GNU plural rules for the supported French and Brazilian Portuguese
        // catalogs use the singular form for zero and one.
        "fr" | "pt" => usize::from(count > 1),
        _ => usize::from(count != 1),
    }
}

struct Catalog {
    locale: String,
    messages: HashMap<String, String>,
}

impl Catalog {
    fn from_mo(locale: String, bytes: &[u8]) -> Option<Self> {
        let endian = match bytes.get(0..4)? {
            [0xde, 0x12, 0x04, 0x95] => Endian::Little,
            [0x95, 0x04, 0x12, 0xde] => Endian::Big,
            _ => return None,
        };
        let count = read_u32(bytes, 8, endian)? as usize;
        let original_table = read_u32(bytes, 12, endian)? as usize;
        let translated_table = read_u32(bytes, 16, endian)? as usize;
        if count > 100_000 {
            return None;
        }

        let mut messages = HashMap::with_capacity(count.saturating_sub(1));
        for index in 0..count {
            let original = table_string(bytes, original_table, index, endian)?;
            let translated = table_string(bytes, translated_table, index, endian)?;
            if original.is_empty() || translated.is_empty() {
                continue;
            }
            messages.insert(original.to_owned(), translated.to_owned());
        }
        Some(Self { locale, messages })
    }
}

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}

fn read_u32(bytes: &[u8], offset: usize, endian: Endian) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(match endian {
        Endian::Little => u32::from_le_bytes(raw),
        Endian::Big => u32::from_be_bytes(raw),
    })
}

fn table_string(bytes: &[u8], table_offset: usize, index: usize, endian: Endian) -> Option<&str> {
    let entry = table_offset.checked_add(index.checked_mul(8)?)?;
    let length = read_u32(bytes, entry, endian)? as usize;
    let offset = read_u32(bytes, entry.checked_add(4)?, endian)? as usize;
    let end = offset.checked_add(length)?;
    std::str::from_utf8(bytes.get(offset..end)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_desktop_locales() {
        assert_eq!(normalize_locale("de_AT.UTF-8"), Some("de_AT".into()));
        assert_eq!(normalize_locale("pt-br.UTF-8"), Some("pt_BR".into()));
        assert_eq!(normalize_locale("C.UTF-8"), None);
    }

    #[test]
    fn interpolates_named_and_positional_values() {
        assert_eq!(
            interpolate("Fehler: {error} ({})", &[("error", "Netz"), ("", "2")]),
            "Fehler: Netz (2)"
        );
    }

    #[test]
    fn supported_plural_rules_are_selected() {
        assert_eq!(plural_index("de", 0), 1);
        assert_eq!(plural_index("fr", 0), 0);
        assert_eq!(plural_index("pt_BR", 2), 1);
    }

    #[test]
    fn embedded_catalogs_are_valid_and_translated() {
        assert_eq!(EMBEDDED_CATALOGS.len(), 6);
        for (locale, bytes) in EMBEDDED_CATALOGS {
            let catalog = Catalog::from_mo((*locale).to_owned(), bytes)
                .unwrap_or_else(|| panic!("invalid embedded catalog for {locale}"));
            assert_eq!(catalog.locale, *locale);
            assert!(catalog.messages.len() >= 209);
            let delivered = match *locale {
                "de" => "Zugestellt",
                "en" => "Delivered",
                "fr" => "Livré",
                "es" => "Entregado",
                "it" => "Consegnata",
                "pt_BR" => "Entregue",
                other => panic!("unexpected embedded locale {other}"),
            };
            assert_eq!(
                catalog.messages.get("Zugestellt").map(String::as_str),
                Some(delivered)
            );
            assert!(
                catalog
                    .messages
                    .contains_key("{} wichtige Sendungsänderung\0{} wichtige Sendungsänderungen")
            );
            let add_delivery = catalog
                .messages
                .get("Neue Sendung hinzufügen")
                .unwrap_or_else(|| panic!("missing add-delivery translation for {locale}"));
            assert!(!add_delivery.is_empty());
            if *locale != "de" {
                assert_ne!(add_delivery, "Neue Sendung hinzufügen");
            }
            let carrier_suggestion = catalog
                .messages
                .get("Aus der Sendungsnummer vorgeschlagen – bitte prüfen.")
                .unwrap_or_else(|| panic!("missing carrier-suggestion translation for {locale}"));
            assert!(!carrier_suggestion.is_empty());
            if *locale != "de" {
                assert_ne!(
                    carrier_suggestion,
                    "Aus der Sendungsnummer vorgeschlagen – bitte prüfen."
                );
            }
            assert!(
                catalog
                    .messages
                    .get("Sendung konnte nicht hinzugefügt werden: {error}")
                    .is_some_and(|translation| translation.contains("{error}"))
            );
            assert!(
                catalog
                    .messages
                    .get("Nur in Ankunft auf diesem Gerät · Parcel-Status: {status}")
                    .is_some_and(|translation| translation.contains("{status}"))
            );
            assert!(
                catalog
                    .messages
                    .get("Die lokale Markierung konnte nicht gespeichert werden: {error}")
                    .is_some_and(|translation| translation.contains("{error}"))
            );
            assert!(
                catalog
                    .messages
                    .get(
                        "Der lokale Sendungsspeicher wurde aktualisiert, aber die dauerhafte Speicherung konnte nicht bestätigt werden: {0}",
                    )
                    .is_some_and(|translation| translation.contains("{0}"))
            );
        }
    }
}
