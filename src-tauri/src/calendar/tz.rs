//! Time zone names found in calendar feeds (`DTSTART;TZID=...`).

use chrono_tz::Tz;

/// Windows time zone ids, as Outlook and Exchange write them in ICS feeds,
/// mapped to IANA zones. From the Unicode CLDR `windowsZones.xml` table
/// (territory "001", the default for each Windows zone).
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("Dateline Standard Time", "Etc/GMT+12"),
    ("UTC-11", "Etc/GMT+11"),
    ("Aleutian Standard Time", "America/Adak"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Marquesas Standard Time", "Pacific/Marquesas"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("UTC-09", "Etc/GMT+9"),
    ("Pacific Standard Time (Mexico)", "America/Tijuana"),
    ("UTC-08", "Etc/GMT+8"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Mountain Standard Time (Mexico)", "America/Mazatlan"),
    ("Mountain Standard Time", "America/Denver"),
    ("Yukon Standard Time", "America/Whitehorse"),
    ("Central America Standard Time", "America/Guatemala"),
    ("Central Standard Time", "America/Chicago"),
    ("Easter Island Standard Time", "Pacific/Easter"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Canada Central Standard Time", "America/Regina"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("Eastern Standard Time (Mexico)", "America/Cancun"),
    ("Eastern Standard Time", "America/New_York"),
    ("Haiti Standard Time", "America/Port-au-Prince"),
    ("Cuba Standard Time", "America/Havana"),
    ("US Eastern Standard Time", "America/Indiana/Indianapolis"),
    ("Turks And Caicos Standard Time", "America/Grand_Turk"),
    ("Paraguay Standard Time", "America/Asuncion"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Venezuela Standard Time", "America/Caracas"),
    ("Central Brazilian Standard Time", "America/Cuiaba"),
    ("SA Western Standard Time", "America/La_Paz"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("Tocantins Standard Time", "America/Araguaina"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("SA Eastern Standard Time", "America/Cayenne"),
    ("Argentina Standard Time", "America/Argentina/Buenos_Aires"),
    ("Greenland Standard Time", "America/Nuuk"),
    ("Montevideo Standard Time", "America/Montevideo"),
    ("Magallanes Standard Time", "America/Punta_Arenas"),
    ("Saint Pierre Standard Time", "America/Miquelon"),
    ("Bahia Standard Time", "America/Bahia"),
    ("UTC-02", "Etc/GMT+2"),
    ("Mid-Atlantic Standard Time", "Etc/GMT+2"),
    ("Azores Standard Time", "Atlantic/Azores"),
    ("Cape Verde Standard Time", "Atlantic/Cape_Verde"),
    ("UTC", "Etc/UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("Sao Tome Standard Time", "Africa/Sao_Tome"),
    ("Morocco Standard Time", "Africa/Casablanca"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("W. Central Africa Standard Time", "Africa/Lagos"),
    ("Jordan Standard Time", "Asia/Amman"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("Middle East Standard Time", "Asia/Beirut"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("Syria Standard Time", "Asia/Damascus"),
    ("West Bank Standard Time", "Asia/Hebron"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("FLE Standard Time", "Europe/Kyiv"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("South Sudan Standard Time", "Africa/Juba"),
    ("Kaliningrad Standard Time", "Europe/Kaliningrad"),
    ("Sudan Standard Time", "Africa/Khartoum"),
    ("Libya Standard Time", "Africa/Tripoli"),
    ("Namibia Standard Time", "Africa/Windhoek"),
    ("Arabic Standard Time", "Asia/Baghdad"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Belarus Standard Time", "Europe/Minsk"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("E. Africa Standard Time", "Africa/Nairobi"),
    ("Volgograd Standard Time", "Europe/Volgograd"),
    ("Iran Standard Time", "Asia/Tehran"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Astrakhan Standard Time", "Europe/Astrakhan"),
    ("Azerbaijan Standard Time", "Asia/Baku"),
    ("Russia Time Zone 3", "Europe/Samara"),
    ("Mauritius Standard Time", "Indian/Mauritius"),
    ("Saratov Standard Time", "Europe/Saratov"),
    ("Georgian Standard Time", "Asia/Tbilisi"),
    ("Caucasus Standard Time", "Asia/Yerevan"),
    ("Afghanistan Standard Time", "Asia/Kabul"),
    ("West Asia Standard Time", "Asia/Tashkent"),
    ("Ekaterinburg Standard Time", "Asia/Yekaterinburg"),
    ("Pakistan Standard Time", "Asia/Karachi"),
    ("Qyzylorda Standard Time", "Asia/Qyzylorda"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Sri Lanka Standard Time", "Asia/Colombo"),
    ("Nepal Standard Time", "Asia/Kathmandu"),
    ("Central Asia Standard Time", "Asia/Bishkek"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("Omsk Standard Time", "Asia/Omsk"),
    ("Myanmar Standard Time", "Asia/Yangon"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("Altai Standard Time", "Asia/Barnaul"),
    ("W. Mongolia Standard Time", "Asia/Hovd"),
    ("North Asia Standard Time", "Asia/Krasnoyarsk"),
    ("N. Central Asia Standard Time", "Asia/Novosibirsk"),
    ("Tomsk Standard Time", "Asia/Tomsk"),
    ("China Standard Time", "Asia/Shanghai"),
    ("North Asia East Standard Time", "Asia/Irkutsk"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("Ulaanbaatar Standard Time", "Asia/Ulaanbaatar"),
    ("Aus Central W. Standard Time", "Australia/Eucla"),
    ("Transbaikal Standard Time", "Asia/Chita"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("North Korea Standard Time", "Asia/Pyongyang"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("Yakutsk Standard Time", "Asia/Yakutsk"),
    ("Cen. Australia Standard Time", "Australia/Adelaide"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("West Pacific Standard Time", "Pacific/Port_Moresby"),
    ("Tasmania Standard Time", "Australia/Hobart"),
    ("Vladivostok Standard Time", "Asia/Vladivostok"),
    ("Lord Howe Standard Time", "Australia/Lord_Howe"),
    ("Bougainville Standard Time", "Pacific/Bougainville"),
    ("Russia Time Zone 10", "Asia/Srednekolymsk"),
    ("Magadan Standard Time", "Asia/Magadan"),
    ("Norfolk Standard Time", "Pacific/Norfolk"),
    ("Sakhalin Standard Time", "Asia/Sakhalin"),
    ("Central Pacific Standard Time", "Pacific/Guadalcanal"),
    ("Russia Time Zone 11", "Asia/Kamchatka"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("UTC+12", "Etc/GMT-12"),
    ("Fiji Standard Time", "Pacific/Fiji"),
    ("Chatham Islands Standard Time", "Pacific/Chatham"),
    ("UTC+13", "Etc/GMT-13"),
    ("Tonga Standard Time", "Pacific/Tongatapu"),
    ("Samoa Standard Time", "Pacific/Apia"),
    ("Line Islands Standard Time", "Pacific/Kiritimati"),
];

/// Windows time zone display names, as some Outlook exports write them in
/// TZID (`(UTC-05:00) Eastern Time (US & Canada)`), without the `(UTC...)`
/// prefix, mapped to the same IANA zones as [`WINDOWS_ZONES`]. Older and
/// newer Windows spellings are both listed.
const WINDOWS_DISPLAY_NAMES: &[(&str, &str)] = &[
    ("International Date Line West", "Etc/GMT+12"),
    ("Coordinated Universal Time-11", "Etc/GMT+11"),
    ("Aleutian Islands", "America/Adak"),
    ("Hawaii", "Pacific/Honolulu"),
    ("Marquesas Islands", "Pacific/Marquesas"),
    ("Alaska", "America/Anchorage"),
    ("Coordinated Universal Time-09", "Etc/GMT+9"),
    ("Baja California", "America/Tijuana"),
    ("Coordinated Universal Time-08", "Etc/GMT+8"),
    ("Pacific Time (US & Canada)", "America/Los_Angeles"),
    ("Arizona", "America/Phoenix"),
    ("Chihuahua, La Paz, Mazatlan", "America/Mazatlan"),
    ("La Paz, Mazatlan", "America/Mazatlan"),
    ("Mountain Time (US & Canada)", "America/Denver"),
    ("Yukon", "America/Whitehorse"),
    ("Central America", "America/Guatemala"),
    ("Central Time (US & Canada)", "America/Chicago"),
    ("Easter Island", "Pacific/Easter"),
    ("Guadalajara, Mexico City, Monterrey", "America/Mexico_City"),
    ("Saskatchewan", "America/Regina"),
    ("Bogota, Lima, Quito, Rio Branco", "America/Bogota"),
    ("Bogota, Lima, Quito", "America/Bogota"),
    ("Chetumal", "America/Cancun"),
    ("Eastern Time (US & Canada)", "America/New_York"),
    ("Haiti", "America/Port-au-Prince"),
    ("Havana", "America/Havana"),
    ("Indiana (East)", "America/Indiana/Indianapolis"),
    ("Turks and Caicos", "America/Grand_Turk"),
    ("Asuncion", "America/Asuncion"),
    ("Atlantic Time (Canada)", "America/Halifax"),
    ("Caracas", "America/Caracas"),
    ("Cuiaba", "America/Cuiaba"),
    ("Georgetown, La Paz, Manaus, San Juan", "America/La_Paz"),
    ("Santiago", "America/Santiago"),
    ("Newfoundland", "America/St_Johns"),
    ("Araguaina", "America/Araguaina"),
    ("Brasilia", "America/Sao_Paulo"),
    ("Cayenne, Fortaleza", "America/Cayenne"),
    ("City of Buenos Aires", "America/Argentina/Buenos_Aires"),
    ("Buenos Aires", "America/Argentina/Buenos_Aires"),
    ("Greenland", "America/Nuuk"),
    ("Montevideo", "America/Montevideo"),
    ("Punta Arenas", "America/Punta_Arenas"),
    ("Saint Pierre and Miquelon", "America/Miquelon"),
    ("Salvador", "America/Bahia"),
    ("Coordinated Universal Time-02", "Etc/GMT+2"),
    ("Mid-Atlantic - Old", "Etc/GMT+2"),
    ("Azores", "Atlantic/Azores"),
    ("Cabo Verde Is.", "Atlantic/Cape_Verde"),
    ("Cape Verde Is.", "Atlantic/Cape_Verde"),
    ("Coordinated Universal Time", "Etc/UTC"),
    ("Dublin, Edinburgh, Lisbon, London", "Europe/London"),
    ("Monrovia, Reykjavik", "Atlantic/Reykjavik"),
    ("Sao Tome", "Africa/Sao_Tome"),
    ("Casablanca", "Africa/Casablanca"),
    (
        "Amsterdam, Berlin, Bern, Rome, Stockholm, Vienna",
        "Europe/Berlin",
    ),
    (
        "Belgrade, Bratislava, Budapest, Ljubljana, Prague",
        "Europe/Budapest",
    ),
    ("Brussels, Copenhagen, Madrid, Paris", "Europe/Paris"),
    ("Sarajevo, Skopje, Warsaw, Zagreb", "Europe/Warsaw"),
    ("West Central Africa", "Africa/Lagos"),
    ("Amman", "Asia/Amman"),
    ("Athens, Bucharest", "Europe/Bucharest"),
    ("Beirut", "Asia/Beirut"),
    ("Cairo", "Africa/Cairo"),
    ("Chisinau", "Europe/Chisinau"),
    ("Damascus", "Asia/Damascus"),
    ("Gaza, Hebron", "Asia/Hebron"),
    ("Harare, Pretoria", "Africa/Johannesburg"),
    (
        "Helsinki, Kyiv, Riga, Sofia, Tallinn, Vilnius",
        "Europe/Kyiv",
    ),
    (
        "Helsinki, Kiev, Riga, Sofia, Tallinn, Vilnius",
        "Europe/Kyiv",
    ),
    ("Jerusalem", "Asia/Jerusalem"),
    ("Juba", "Africa/Juba"),
    ("Kaliningrad", "Europe/Kaliningrad"),
    ("Khartoum", "Africa/Khartoum"),
    ("Tripoli", "Africa/Tripoli"),
    ("Windhoek", "Africa/Windhoek"),
    ("Baghdad", "Asia/Baghdad"),
    ("Istanbul", "Europe/Istanbul"),
    ("Kuwait, Riyadh", "Asia/Riyadh"),
    ("Minsk", "Europe/Minsk"),
    ("Moscow, St. Petersburg", "Europe/Moscow"),
    ("Moscow, St. Petersburg, Volgograd", "Europe/Moscow"),
    ("Nairobi", "Africa/Nairobi"),
    ("Volgograd", "Europe/Volgograd"),
    ("Tehran", "Asia/Tehran"),
    ("Abu Dhabi, Muscat", "Asia/Dubai"),
    ("Astrakhan, Ulyanovsk", "Europe/Astrakhan"),
    ("Baku", "Asia/Baku"),
    ("Izhevsk, Samara", "Europe/Samara"),
    ("Port Louis", "Indian/Mauritius"),
    ("Saratov", "Europe/Saratov"),
    ("Tbilisi", "Asia/Tbilisi"),
    ("Yerevan", "Asia/Yerevan"),
    ("Kabul", "Asia/Kabul"),
    ("Ashgabat, Tashkent", "Asia/Tashkent"),
    ("Ekaterinburg", "Asia/Yekaterinburg"),
    ("Islamabad, Karachi", "Asia/Karachi"),
    ("Qyzylorda", "Asia/Qyzylorda"),
    ("Chennai, Kolkata, Mumbai, New Delhi", "Asia/Kolkata"),
    ("Sri Jayawardenepura", "Asia/Colombo"),
    ("Kathmandu", "Asia/Kathmandu"),
    ("Astana", "Asia/Bishkek"),
    ("Bishkek", "Asia/Bishkek"),
    ("Dhaka", "Asia/Dhaka"),
    ("Omsk", "Asia/Omsk"),
    ("Yangon (Rangoon)", "Asia/Yangon"),
    ("Bangkok, Hanoi, Jakarta", "Asia/Bangkok"),
    ("Barnaul, Gorno-Altaysk", "Asia/Barnaul"),
    ("Hovd", "Asia/Hovd"),
    ("Krasnoyarsk", "Asia/Krasnoyarsk"),
    ("Novosibirsk", "Asia/Novosibirsk"),
    ("Tomsk", "Asia/Tomsk"),
    ("Beijing, Chongqing, Hong Kong, Urumqi", "Asia/Shanghai"),
    ("Irkutsk", "Asia/Irkutsk"),
    ("Kuala Lumpur, Singapore", "Asia/Singapore"),
    ("Perth", "Australia/Perth"),
    ("Taipei", "Asia/Taipei"),
    ("Ulaanbaatar", "Asia/Ulaanbaatar"),
    ("Eucla", "Australia/Eucla"),
    ("Chita", "Asia/Chita"),
    ("Osaka, Sapporo, Tokyo", "Asia/Tokyo"),
    ("Pyongyang", "Asia/Pyongyang"),
    ("Seoul", "Asia/Seoul"),
    ("Yakutsk", "Asia/Yakutsk"),
    ("Adelaide", "Australia/Adelaide"),
    ("Darwin", "Australia/Darwin"),
    ("Brisbane", "Australia/Brisbane"),
    ("Canberra, Melbourne, Sydney", "Australia/Sydney"),
    ("Guam, Port Moresby", "Pacific/Port_Moresby"),
    ("Hobart", "Australia/Hobart"),
    ("Vladivostok", "Asia/Vladivostok"),
    ("Lord Howe Island", "Australia/Lord_Howe"),
    ("Bougainville Island", "Pacific/Bougainville"),
    ("Chokurdakh", "Asia/Srednekolymsk"),
    ("Magadan", "Asia/Magadan"),
    ("Norfolk Island", "Pacific/Norfolk"),
    ("Sakhalin", "Asia/Sakhalin"),
    ("Solomon Is., New Caledonia", "Pacific/Guadalcanal"),
    ("Anadyr, Petropavlovsk-Kamchatsky", "Asia/Kamchatka"),
    ("Auckland, Wellington", "Pacific/Auckland"),
    ("Coordinated Universal Time+12", "Etc/GMT-12"),
    ("Fiji", "Pacific/Fiji"),
    ("Chatham Islands", "Pacific/Chatham"),
    ("Coordinated Universal Time+13", "Etc/GMT-13"),
    ("Nuku'alofa", "Pacific/Tongatapu"),
    ("Samoa", "Pacific/Apia"),
    ("Kiritimati Island", "Pacific/Kiritimati"),
];

/// The name part of a Windows display name: `(UTC-05:00) Eastern Time (US &
/// Canada)` gives `Eastern Time (US & Canada)`. Also accepts the older
/// `(GMT-05:00)` form and a bare `(UTC)`.
fn strip_display_offset(name: &str) -> Option<&str> {
    let rest = name.strip_prefix('(')?;
    let (offset, after) = rest.split_once(')')?;
    let offset = offset.trim();
    let sign = offset
        .strip_prefix("UTC")
        .or_else(|| offset.strip_prefix("GMT"))?;
    let looks_like_offset = sign.is_empty()
        || (sign.starts_with(['+', '-'])
            && sign[1..].chars().all(|c| c.is_ascii_digit() || c == ':'));
    looks_like_offset.then(|| after.trim())
}

/// Resolves a `TZID` parameter to a zone. Accepts IANA names
/// (`America/New_York`), Windows names (`Eastern Standard Time`), Windows
/// display names (`(UTC-05:00) Eastern Time (US & Canada)`, B-16), and
/// prefixed ids some apps write (`/mozilla.org/20050126_1/America/New_York`).
/// `None` for an unknown name; the caller then treats the time as local.
pub fn resolve_tzid(raw: &str) -> Option<Tz> {
    let name = raw.trim().trim_matches('"').trim();
    if name.is_empty() {
        return None;
    }
    if let Ok(tz) = name.parse::<Tz>() {
        return Some(tz);
    }
    if let Some((_, iana)) = WINDOWS_ZONES
        .iter()
        .find(|(win, _)| win.eq_ignore_ascii_case(name))
    {
        return iana.parse().ok();
    }
    if let Some(tz) = chrono_tz::TZ_VARIANTS
        .iter()
        .find(|tz| tz.name().eq_ignore_ascii_case(name))
    {
        return Some(*tz);
    }
    if let Some(display) = strip_display_offset(name) {
        return WINDOWS_DISPLAY_NAMES
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(display))
            .and_then(|(_, iana)| iana.parse().ok());
    }
    // A vendor prefix before an IANA name: try each "/"-separated suffix.
    name.match_indices('/')
        .find_map(|(i, _)| name[i + 1..].parse::<Tz>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_windows_zone_maps_to_a_real_iana_zone() {
        for (win, iana) in WINDOWS_ZONES {
            assert!(iana.parse::<Tz>().is_ok(), "{win} -> {iana} is not a zone");
            // Names that are already IANA ids ("UTC") resolve to themselves.
            let expected = win.parse::<Tz>().map(|t| t.name()).unwrap_or(iana);
            assert_eq!(resolve_tzid(win).map(|t| t.name()), Some(expected));
        }
    }

    #[test]
    fn resolves_common_tzid_forms() {
        assert_eq!(
            resolve_tzid("America/New_York"),
            Some(Tz::America__New_York)
        );
        assert_eq!(resolve_tzid("\"Europe/Berlin\""), Some(Tz::Europe__Berlin));
        assert_eq!(
            resolve_tzid("Eastern Standard Time"),
            Some(Tz::America__New_York)
        );
        assert_eq!(
            resolve_tzid("w. europe standard time"),
            Some(Tz::Europe__Berlin)
        );
        assert_eq!(
            resolve_tzid("/mozilla.org/20050126_1/America/Los_Angeles"),
            Some(Tz::America__Los_Angeles)
        );
        assert_eq!(resolve_tzid("america/chicago"), Some(Tz::America__Chicago));
        assert_eq!(resolve_tzid("Not A Zone"), None);
        assert_eq!(resolve_tzid(""), None);
    }

    #[test]
    fn every_display_name_maps_to_a_real_iana_zone() {
        for (name, iana) in WINDOWS_DISPLAY_NAMES {
            assert!(iana.parse::<Tz>().is_ok(), "{name} -> {iana} is not a zone");
        }
    }

    #[test]
    fn resolves_outlook_display_names() {
        assert_eq!(
            resolve_tzid("(UTC-05:00) Eastern Time (US & Canada)"),
            Some(Tz::America__New_York)
        );
        assert_eq!(
            resolve_tzid("\"(UTC-08:00) Pacific Time (US & Canada)\""),
            Some(Tz::America__Los_Angeles)
        );
        assert_eq!(
            resolve_tzid("(UTC+01:00) Amsterdam, Berlin, Bern, Rome, Stockholm, Vienna"),
            Some(Tz::Europe__Berlin)
        );
        assert_eq!(
            resolve_tzid("(GMT+05:30) Chennai, Kolkata, Mumbai, New Delhi"),
            Some(Tz::Asia__Kolkata)
        );
        assert_eq!(
            resolve_tzid("(UTC) Coordinated Universal Time").map(|t| t.name()),
            Some("Etc/UTC")
        );
        // Unknown display names fall back to local time (None).
        assert_eq!(resolve_tzid("(UTC-05:00) Somewhere New"), None);
        assert_eq!(
            resolve_tzid("(Not an offset) Eastern Time (US & Canada)"),
            None
        );
    }
}
