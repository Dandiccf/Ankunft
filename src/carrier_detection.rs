//! Privacy-preserving carrier suggestions based on tracking-number formats.
//!
//! Parcel's public carrier list contains names and internal codes, but no
//! detection rules.  These helpers deliberately work offline, never log the
//! tracking number, and only cover formats that can be recognized with a
//! useful degree of confidence.  The UI always leaves the suggestion editable.

/// Returns Parcel carrier codes in preference order for a tracking number.
///
/// The first code that exists in Parcel's current carrier list is used.  Some
/// carriers have country-specific codes, so the locale is used only as a hint
/// for ordering those variants.
pub fn suggest_carrier_codes(tracking_number: &str, locale: &str) -> Vec<&'static str> {
    let Some(number) = normalized_tracking_number(tracking_number) else {
        return Vec::new();
    };

    if is_ups(&number) {
        return vec!["ups"];
    }

    if is_amazon_logistics(&number) {
        return preferred_amazon_codes(locale);
    }

    // Parcel documents four numeric characters as its placeholder format.
    if number.len() == 4 && number.bytes().all(|byte| byte.is_ascii_digit()) {
        return vec!["pholder"];
    }

    if let Some(code) = s10_postal_carrier(&number) {
        return vec![code];
    }

    if is_usps_numeric(&number) {
        return vec!["usps"];
    }

    if is_dhl_alphanumeric(&number) || is_dhl_express_numeric(&number) {
        return vec!["dhl"];
    }

    if is_fedex_express(&number) {
        return vec!["fedex"];
    }

    // DPD Austria and several other DPD regions document a 14-digit parcel
    // number.  The locale only ranks the matching regional Parcel code; the UI
    // explicitly labels the result as a suggestion that can be overridden.
    if number.len() == 14 && number.bytes().all(|byte| byte.is_ascii_digit()) {
        return preferred_dpd_codes(locale);
    }

    // Austrian Post publicly shows a 22-digit domestic example.  Restrict this
    // weaker format to Austrian locales and keep it below USPS recognition.
    if locale_region(locale) == Some("AT")
        && number.len() == 22
        && number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return vec!["at"];
    }

    Vec::new()
}

fn normalized_tracking_number(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value
            .chars()
            .any(|character| !character.is_ascii_alphanumeric() && !matches!(character, ' ' | '-'))
    {
        return None;
    }

    let normalized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_uppercase())
        .collect::<String>();
    (!normalized.is_empty()).then_some(normalized)
}

fn is_ups(number: &str) -> bool {
    let bytes = number.as_bytes();
    if bytes.len() != 18
        || !number.starts_with("1Z")
        || !bytes[2..17].iter().all(u8::is_ascii_alphanumeric)
        || !bytes[17].is_ascii_digit()
    {
        return false;
    }

    let sum = bytes[2..17]
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            let value = if byte.is_ascii_digit() {
                u32::from(byte - b'0')
            } else {
                u32::from((byte - 3) % 10)
            };
            value * if index % 2 == 0 { 1 } else { 2 }
        })
        .sum::<u32>();
    (10 - (sum % 10)) % 10 == u32::from(bytes[17] - b'0')
}

fn is_amazon_logistics(number: &str) -> bool {
    number.len() == 15
        && matches!(&number[..3], "TBA" | "TBC" | "TBM")
        && number[3..].bytes().all(|byte| byte.is_ascii_digit())
}

fn preferred_amazon_codes(locale: &str) -> Vec<&'static str> {
    let primary = match locale_region(locale) {
        Some("AU") => "amzlau",
        Some("BE") => "amzlbe",
        Some("BR") => "amzlbr",
        Some("CA") => "amzlca",
        Some("EG") => "amzleg",
        Some("ES") => "amzles",
        Some("FR") => "amzlfr",
        Some("GB" | "UK") => "amzluk",
        Some("IE") => "amzlie",
        Some("IN") => "amzlin",
        Some("IT") => "amzlit",
        Some("JP") => "amzljp",
        Some("MX") => "amzlmx",
        Some("NL") => "amzlnl",
        Some("PL") => "amzlpl",
        Some("SA") => "amzlsa",
        Some("SE") => "amzlse",
        Some("SG") => "amzlsg",
        Some("TR") => "amzltr",
        Some("AE") => "amzlae",
        Some("US") => "amzlus",
        // Amazon Germany also serves Austria, while language-only German is a
        // useful fallback when a region is unavailable.
        Some("AT" | "DE") => "amzlde",
        None if locale_language(locale) == "de" => "amzlde",
        _ => "amzlus",
    };
    vec![primary]
}

fn s10_postal_carrier(number: &str) -> Option<&'static str> {
    let bytes = number.as_bytes();
    if !has_s10_shape(number) {
        return None;
    }

    let weights = [8_u32, 6, 4, 2, 3, 5, 9, 7];
    let sum = bytes[2..10]
        .iter()
        .zip(weights)
        .map(|(digit, weight)| u32::from(digit - b'0') * weight)
        .sum::<u32>();
    let expected = match 11 - (sum % 11) {
        11 => 5,
        10 => 0,
        value => value,
    };
    if expected != u32::from(bytes[10] - b'0') {
        return None;
    }

    match &number[11..] {
        "AT" => Some("at"),
        "AU" => Some("au"),
        "AZ" => Some("azer"),
        "BE" => Some("bpost"),
        "BG" => Some("bolg"),
        "BR" => Some("corbra"),
        "BY" => Some("blp"),
        "CA" => Some("cp"),
        "CH" => Some("swiss"),
        "CN" => Some("china"),
        "CY" => Some("cypr"),
        "CZ" => Some("ceska"),
        "DE" => Some("dp"),
        "DK" => Some("dk"),
        "EE" => Some("ee"),
        "ES" => Some("cor"),
        "FI" => Some("posti"),
        "FR" => Some("lp"),
        "GB" => Some("rm"),
        "GR" => Some("elta"),
        "HK" => Some("hk"),
        "HR" => Some("hr"),
        "HU" => Some("hung"),
        "ID" => Some("indon"),
        "IE" => Some("anpost"),
        "IL" => Some("il"),
        "IN" => Some("in"),
        "IT" => Some("it"),
        "JP" => Some("jp"),
        "KR" => Some("kor"),
        "KZ" => Some("kz"),
        "LT" => Some("litva"),
        "LU" => Some("ptl"),
        "LV" => Some("lv"),
        "MD" => Some("moldov"),
        "MT" => Some("malta"),
        "MX" => Some("corm"),
        "MY" => Some("malpos"),
        "NL" => Some("tntp"),
        "NO" => Some("nor"),
        "NZ" => Some("coup"),
        "PK" => Some("pk"),
        "PL" => Some("poland"),
        "PT" => Some("ctt"),
        "RU" => Some("rp"),
        "SA" => Some("saudi"),
        "SE" => Some("se"),
        "SG" => Some("sing"),
        "SI" => Some("slv"),
        "SK" => Some("slovak"),
        "TH" => Some("thai"),
        "TR" => Some("turk"),
        "UA" => Some("ukr"),
        "US" => Some("usps"),
        "ZA" => Some("safr"),
        _ => None,
    }
}

fn has_s10_shape(number: &str) -> bool {
    let bytes = number.as_bytes();
    bytes.len() == 13
        && bytes[..2].iter().all(u8::is_ascii_uppercase)
        && bytes[2..11].iter().all(u8::is_ascii_digit)
        && bytes[11..].iter().all(u8::is_ascii_uppercase)
}

fn is_usps_numeric(number: &str) -> bool {
    matches!(number.len(), 20..=22)
        && number.bytes().all(|byte| byte.is_ascii_digit())
        && matches!(&number[..2], "92" | "93" | "94" | "95")
}

fn is_dhl_alphanumeric(number: &str) -> bool {
    // A mistyped international S10 postal number must not fall through into
    // DHL merely because service prefixes such as LX or RX overlap.
    if has_s10_shape(number) {
        return false;
    }

    let dhl_piece = (number.starts_with("JJD") && matches!(number.len(), 12..=13))
        || (number.starts_with("JVGL") && matches!(number.len(), 13..=14));
    if dhl_piece {
        let prefix_len = if number.starts_with("JVGL") { 4 } else { 3 };
        return number[prefix_len..]
            .bytes()
            .all(|byte| byte.is_ascii_digit());
    }

    let ecommerce_prefix = ["GM", "LX", "RX", "UV", "CN", "SG", "TH", "IN", "HK"]
        .into_iter()
        .any(|prefix| number.starts_with(prefix));
    ecommerce_prefix
        && matches!(number.len(), 12..=41)
        && number.bytes().any(|byte| byte.is_ascii_digit())
        && number.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_dhl_express_numeric(number: &str) -> bool {
    if !matches!(number.len(), 10 | 11) || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let (serial, check_digit) = number.split_at(number.len() - 1);
    serial
        .parse::<u64>()
        .is_ok_and(|serial| serial % 7 == check_digit.parse::<u64>().unwrap_or(10))
}

fn is_fedex_express(number: &str) -> bool {
    let bytes = number.as_bytes();
    if bytes.len() != 12 || !bytes.iter().all(u8::is_ascii_digit) {
        return false;
    }

    let weights = [3_u32, 1, 7, 3, 1, 7, 3, 1, 7, 3, 1];
    let sum = bytes[..11]
        .iter()
        .zip(weights)
        .map(|(digit, weight)| u32::from(digit - b'0') * weight)
        .sum::<u32>();
    (sum % 11) % 10 == u32::from(bytes[11] - b'0')
}

fn preferred_dpd_codes(locale: &str) -> Vec<&'static str> {
    let regional = match locale_region(locale) {
        Some("AT") => "dpdat",
        Some("DE") => "dpdpcode",
        Some("FR") => "dpdfrpcode",
        Some("GB" | "UK") => "dpduk",
        Some("IE") => "dpdie",
        Some("IT") => "dpditpcode",
        Some("PL") => "dpdpoland",
        _ => "dpdgpcode",
    };
    if regional == "dpdgpcode" {
        vec![regional]
    } else {
        vec![regional, "dpdgpcode"]
    }
}

fn locale_language(locale: &str) -> &str {
    locale.split(['_', '-', '.', '@']).next().unwrap_or(locale)
}

fn locale_region(locale: &str) -> Option<&str> {
    let locale = locale.split(['.', '@']).next().unwrap_or(locale);
    locale.split(['_', '-']).nth(1).map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_ups_with_checksum_and_ignores_near_match() {
        assert_eq!(
            suggest_carrier_codes("1Z5R89390357567127", "de_AT"),
            vec!["ups"]
        );
        assert!(suggest_carrier_codes("1Z5R89390357567128", "de_AT").is_empty());
    }

    #[test]
    fn recognizes_s10_and_uses_issuing_postal_operator() {
        assert_eq!(suggest_carrier_codes("CA482156820DE", "de_AT"), vec!["dp"]);
        assert!(suggest_carrier_codes("CA482156821DE", "de_AT").is_empty());
        assert!(suggest_carrier_codes("LX123456789CN", "de_AT").is_empty());
    }

    #[test]
    fn recognizes_common_offline_formats() {
        assert_eq!(
            suggest_carrier_codes("TBA000000000000", "de_AT.UTF-8"),
            vec!["amzlde"]
        );
        assert_eq!(
            suggest_carrier_codes("01234567890123", "de_AT.UTF-8"),
            vec!["dpdat", "dpdgpcode"]
        );
        assert_eq!(suggest_carrier_codes("3318810025", "de_AT"), vec!["dhl"]);
        assert_eq!(
            suggest_carrier_codes("986578788855", "de_AT"),
            vec!["fedex"]
        );
        assert_eq!(suggest_carrier_codes("0000", "de_AT"), vec!["pholder"]);
    }

    #[test]
    fn accepts_human_separators_but_not_urls() {
        assert_eq!(
            suggest_carrier_codes("1Z 5R89-3903 5756 7127", "en_US"),
            vec!["ups"]
        );
        assert!(suggest_carrier_codes("https://example.test/1Z", "en_US").is_empty());
    }

    #[test]
    fn ranks_country_specific_dpd_codes() {
        assert_eq!(
            suggest_carrier_codes("01234567890123", "de_DE"),
            vec!["dpdpcode", "dpdgpcode"]
        );
        assert_eq!(
            suggest_carrier_codes("01234567890123", "fr_FR"),
            vec!["dpdfrpcode", "dpdgpcode"]
        );
    }
}
