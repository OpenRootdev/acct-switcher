//! When a saved sign-in stops working: a date the app stores next to its tokens, or an idle
//! limit after which the provider signs out accounts that are not used. acswi only ever reads
//! the date out of a sign-in file, never a token.

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::paths::expand;

/// Accounts that stop working within this many days are shown as expiring soon.
pub const WARN_DAYS: i64 = 7;

/// `[expiry]` in a definition. Either or both of a stored date and an idle limit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpirySpec {
    /// JSON file the app keeps its sign-in in (read while the account is signed in).
    #[serde(default)]
    pub file: Option<String>,
    /// JSON pointer to the date in that file: seconds or milliseconds since 1970, or RFC 3339.
    #[serde(default)]
    pub pointer: Option<String>,
    /// The provider signs out an account that is not used for this many days.
    #[serde(default)]
    pub idle_days: Option<u32>,
}

impl ExpirySpec {
    pub fn validate(&self) -> Result<(), String> {
        match (&self.file, &self.pointer, self.idle_days) {
            (Some(_), None, _) | (None, Some(_), _) => Err("[expiry] needs both `file` and `pointer`".into()),
            (None, None, None) => Err("[expiry] needs `file` and `pointer`, or `idle_days`".into()),
            (_, _, Some(0)) => Err("[expiry] idle_days must be more than 0".into()),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ok,
    Soon,
    Expired,
}

#[derive(Debug, Clone, Serialize)]
pub struct Expiry {
    pub at: DateTime<Utc>,
    /// Worked out from the idle limit and when the account was last used, not a date the
    /// app stored, so the real date may be later.
    pub estimated: bool,
    /// Compared with `WARN_DAYS`.
    pub level: Level,
}

impl Expiry {
    pub fn new(at: DateTime<Utc>, estimated: bool) -> Expiry {
        Expiry { at, estimated, level: level(at, WARN_DAYS) }
    }

    /// Whole days left (negative once expired).
    pub fn days_left(&self) -> i64 {
        (self.at - Utc::now()).num_days()
    }
}

pub fn level(at: DateTime<Utc>, warn_days: i64) -> Level {
    let now = Utc::now();
    if at <= now {
        Level::Expired
    } else if at - now <= Duration::days(warn_days) {
        Level::Soon
    } else {
        Level::Ok
    }
}

/// The date stored with the live sign-in, if the definition names one.
pub fn read_live(spec: &ExpirySpec) -> Option<DateTime<Utc>> {
    let (file, pointer) = (spec.file.as_ref()?, spec.pointer.as_ref()?);
    let bytes = std::fs::read(expand(file)).ok()?;
    let doc: Value = serde_json::from_slice(strip_bom(&bytes)).ok()?;
    parse_date(doc.pointer(pointer)?)
}

fn strip_bom(b: &[u8]) -> &[u8] {
    b.strip_prefix(b"\xef\xbb\xbf").unwrap_or(b)
}

pub fn parse_date(v: &Value) -> Option<DateTime<Utc>> {
    match v {
        Value::Number(n) => from_epoch(n.as_f64()?),
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.with_timezone(&Utc))
            .or_else(|| s.trim().parse::<f64>().ok().and_then(from_epoch)),
        _ => None,
    }
}

fn from_epoch(n: f64) -> Option<DateTime<Utc>> {
    // Seconds would only pass 1e11 in the year 5138, so larger numbers are milliseconds.
    let secs = if n > 1e11 { n / 1000.0 } else { n };
    Utc.timestamp_opt(secs as i64, 0).single()
}

/// The earlier of the stored date and the idle limit counted from `idle_since`.
pub fn combine(spec: &ExpirySpec, stored: Option<DateTime<Utc>>, idle_since: Option<DateTime<Utc>>) -> Option<Expiry> {
    let idle = spec.idle_days.zip(idle_since).map(|(d, since)| since + Duration::days(d.into()));
    match (stored, idle) {
        (Some(s), Some(i)) if i < s => Some(Expiry::new(i, true)),
        (Some(s), _) => Some(Expiry::new(s, false)),
        (None, Some(i)) => Some(Expiry::new(i, true)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dates() {
        let want = Utc.with_ymd_and_hms(2026, 11, 23, 18, 54, 0).unwrap();
        assert_eq!(parse_date(&serde_json::json!(want.timestamp_millis())), Some(want));
        assert_eq!(parse_date(&serde_json::json!(want.timestamp())), Some(want));
        assert_eq!(parse_date(&serde_json::json!("2026-11-23T18:54:00Z")), Some(want));
        assert_eq!(parse_date(&serde_json::json!("2026-11-23T20:54:00+02:00")), Some(want));
        assert_eq!(parse_date(&serde_json::json!(true)), None);
    }

    #[test]
    fn reads_only_the_date_from_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("creds.json");
        std::fs::write(&file, br#"{"oauth":{"token":"x","refreshExpiresAt":1795460040000}}"#).unwrap();
        let spec = ExpirySpec {
            file: Some(file.to_string_lossy().into()),
            pointer: Some("/oauth/refreshExpiresAt".into()),
            idle_days: None,
        };
        assert_eq!(read_live(&spec), Some(Utc.timestamp_opt(1795460040, 0).unwrap()));
    }

    #[test]
    fn earlier_limit_wins() {
        let spec = ExpirySpec { idle_days: Some(90), ..Default::default() };
        let now = Utc::now();
        let e = combine(&spec, Some(now + Duration::days(200)), Some(now - Duration::days(85))).unwrap();
        assert!(e.estimated);
        assert_eq!(e.level, Level::Soon);
        let e = combine(&spec, Some(now + Duration::days(30)), Some(now)).unwrap();
        assert!(!e.estimated);
        assert_eq!(e.level, Level::Ok);
        assert_eq!(combine(&spec, None, Some(now - Duration::days(91))).unwrap().level, Level::Expired);
        assert!(combine(&spec, None, None).is_none());
        assert!(ExpirySpec::default().validate().is_err());
    }
}
