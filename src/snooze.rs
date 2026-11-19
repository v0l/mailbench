use crate::backend::MessageSummary;
use crate::store::{Store, StoreError};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const KIND: &str = "snooze";

pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

pub fn key_of(account: &str, message_id: Option<&str>, date: i64, subject: &str) -> String {
    use sha2::Digest;
    let identity = match message_id.filter(|m| !m.is_empty()) {
        Some(id) => id.to_lowercase(),
        None => format!("{date}\0{subject}"),
    };
    let digest = sha2::Sha256::digest(identity.as_bytes());
    format!("{account}\0{}", digest[..16].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snooze {
    pub account: String,
    pub message_id: Option<String>,
    pub subject: String,
    pub date: i64,
    pub wake_at: i64,
}

impl Snooze {
    pub fn key(&self) -> String {
        key_of(&self.account, self.message_id.as_deref(), self.date, &self.subject)
    }

    pub fn matches(&self, m: &MessageSummary) -> bool {
        match (&self.message_id, &m.message_id) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => m.date.unwrap_or(0) == self.date && m.subject.as_deref().unwrap_or("") == self.subject,
        }
    }
}

pub struct SnoozeTable<'a> {
    store: &'a Store,
    clock: &'a dyn Clock,
}

impl<'a> SnoozeTable<'a> {
    pub fn new(store: &'a Store, clock: &'a dyn Clock) -> Self {
        SnoozeTable { store, clock }
    }

    pub fn add(&self, snooze: &Snooze) -> Result<(), StoreError> {
        let raw = serde_json::to_vec(snooze).map_err(|e| StoreError::Db(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?;
        self.store.put(KIND, &snooze.key(), &raw)
    }

    pub fn remove(&self, snooze: &Snooze) -> Result<(), StoreError> {
        self.store.delete(KIND, &snooze.key())
    }

    pub fn all(&self, account: &str) -> Vec<Snooze> {
        let mut out: Vec<Snooze> = self
            .store
            .list_prefix(KIND, &format!("{account}\0"))
            .unwrap_or_default()
            .iter()
            .filter_map(|k| self.store.get(KIND, k).ok().flatten())
            .filter_map(|raw| serde_json::from_slice(&raw).ok())
            .collect();
        out.sort_by_key(|s| s.wake_at);
        out
    }

    pub fn due(&self, account: &str) -> Vec<Snooze> {
        let now = self.clock.now();
        self.all(account).into_iter().filter(|s| s.wake_at <= now).collect()
    }

    pub fn next_due(&self, account: &str) -> Option<i64> {
        self.all(account).first().map(|s| s.wake_at)
    }

    pub fn wake_times(&self, account: &str) -> HashMap<String, i64> {
        self.all(account).into_iter().map(|s| (s.key(), s.wake_at)).collect()
    }
}

pub fn describe(at: i64) -> String {
    match chrono::Local.timestamp_opt(at, 0).single() {
        Some(t) => t.format("%a %-d %b %H:%M").to_string(),
        None => String::new(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub label: &'static str,
    pub at: i64,
}

fn at_time<Tz: TimeZone>(tz: &Tz, date: NaiveDate, h: u32, m: u32) -> Option<DateTime<Tz>> {
    tz.from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(h, m, 0)?)).earliest()
}

pub fn presets<Tz: TimeZone>(now: &DateTime<Tz>) -> Vec<Preset> {
    let tz = now.timezone();
    let today = now.date_naive();
    let later = (now.clone() + Duration::hours(3)).with_minute(0).and_then(|d| d.with_second(0)).unwrap_or_else(|| now.clone());
    let tomorrow = today + Duration::days(1);
    let week = today + Duration::days(7 - today.weekday().num_days_from_monday() as i64);
    let mut out = vec![Preset { label: "later today", at: later.timestamp() }];
    if let Some(t) = at_time(&tz, tomorrow, 8, 0) {
        out.push(Preset { label: "tomorrow 08:00", at: t.timestamp() });
    }
    if let Some(t) = at_time(&tz, week, 8, 0) {
        out.push(Preset { label: "next week", at: t.timestamp() });
    }
    out
}

pub fn parse_custom<Tz: TimeZone>(text: &str, now: &DateTime<Tz>) -> Option<i64> {
    let text = text.trim();
    let (day, time) = text.split_once(' ').map(|(d, t)| (d, t.trim())).unwrap_or((text, "08:00"));
    let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    let time = NaiveTime::parse_from_str(time, "%H:%M").ok()?;
    let at = now.timezone().from_local_datetime(&date.and_time(time)).earliest()?.timestamp();
    (at > now.timestamp()).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::KdfParams;
    use chrono::{FixedOffset, Utc};
    use std::sync::atomic::{AtomicI64, Ordering};

    struct Fake(AtomicI64);

    impl Clock for Fake {
        fn now(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(&dir.path().join("s.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
        (dir, store)
    }

    fn snooze(account: &str, id: &str, wake_at: i64) -> Snooze {
        Snooze { account: account.into(), message_id: Some(id.into()), subject: "s".into(), date: 1, wake_at }
    }

    #[test]
    fn nothing_is_due_until_the_clock_reaches_the_wake_time() {
        let (_d, store) = store();
        let clock = Fake(AtomicI64::new(100));
        let table = SnoozeTable::new(&store, &clock);
        table.add(&snooze("a", "<x@y>", 200)).unwrap();
        table.add(&snooze("a", "<z@y>", 150)).unwrap();
        assert!(table.due("a").is_empty());
        assert_eq!(table.next_due("a"), Some(150));
        clock.0.store(150, Ordering::SeqCst);
        assert_eq!(table.due("a").len(), 1);
        clock.0.store(500, Ordering::SeqCst);
        let due = table.due("a");
        assert_eq!(due.iter().map(|s| s.wake_at).collect::<Vec<_>>(), vec![150, 200]);
        table.remove(&due[0]).unwrap();
        assert_eq!(table.due("a").len(), 1);
    }

    #[test]
    fn accounts_do_not_see_each_others_snoozes() {
        let (_d, store) = store();
        let clock = Fake(AtomicI64::new(1000));
        let table = SnoozeTable::new(&store, &clock);
        table.add(&snooze("a", "<x@y>", 5)).unwrap();
        table.add(&snooze("ab", "<x@y>", 5)).unwrap();
        assert_eq!(table.due("a").len(), 1);
        assert_eq!(table.due("b").len(), 0);
    }

    #[test]
    fn snoozing_twice_replaces_the_wake_time() {
        let (_d, store) = store();
        let clock = Fake(AtomicI64::new(0));
        let table = SnoozeTable::new(&store, &clock);
        table.add(&snooze("a", "<X@y>", 5)).unwrap();
        table.add(&snooze("a", "<x@Y>", 9)).unwrap();
        assert_eq!(table.all("a").len(), 1);
        assert_eq!(table.next_due("a"), Some(9));
    }

    #[test]
    fn snoozes_are_encrypted_at_rest() {
        let (dir, store) = store();
        let clock = Fake(AtomicI64::new(0));
        SnoozeTable::new(&store, &clock).add(&snooze("a", "<secret-id@y>", 5)).unwrap();
        drop(store);
        let bytes: Vec<u8> = std::fs::read_dir(dir.path())
            .unwrap()
            .flat_map(|f| std::fs::read(f.unwrap().path()).unwrap())
            .collect();
        assert!(bytes.len() > 1000);
        assert!(!bytes.windows(9).any(|w| w == b"secret-id"));
    }

    #[test]
    fn matches_by_message_id_or_by_date_and_subject() {
        let s = snooze("a", "<x@y>", 5);
        let m = MessageSummary { message_id: Some("<X@Y>".into()), ..Default::default() };
        assert!(s.matches(&m));
        let bare = Snooze { message_id: None, ..s.clone() };
        let m = MessageSummary { subject: Some("s".into()), date: Some(1), ..Default::default() };
        assert!(bare.matches(&m));
        assert!(!bare.matches(&MessageSummary { subject: Some("t".into()), date: Some(1), ..Default::default() }));
    }

    fn at(day: &str) -> DateTime<FixedOffset> {
        let tz = FixedOffset::east_opt(2 * 3600).unwrap();
        tz.from_local_datetime(&NaiveDate::parse_from_str(&day[..10], "%Y-%m-%d").unwrap().and_hms_opt(day[11..13].parse().unwrap(), day[14..16].parse().unwrap(), 0).unwrap())
            .unwrap()
    }

    fn local(ts: i64) -> String {
        Utc.timestamp_opt(ts, 0).unwrap().with_timezone(&FixedOffset::east_opt(2 * 3600).unwrap()).format("%Y-%m-%d %H:%M").to_string()
    }

    #[test]
    fn presets_follow_the_clock() {
        let wed = at("2025-07-09 10:25");
        let p = presets(&wed);
        assert_eq!(local(p[0].at), "2025-07-09 13:00");
        assert_eq!(local(p[1].at), "2025-07-10 08:00");
        assert_eq!(local(p[2].at), "2025-07-14 08:00");
        let mon = at("2025-07-14 09:00");
        assert_eq!(local(presets(&mon)[2].at), "2025-07-21 08:00");
    }

    #[test]
    fn custom_dates_parse_and_must_be_in_the_future() {
        let now = at("2025-07-09 10:25");
        assert_eq!(local(parse_custom("2025-08-01", &now).unwrap()), "2025-08-01 08:00");
        assert_eq!(local(parse_custom("2025-08-01 17:30", &now).unwrap()), "2025-08-01 17:30");
        assert_eq!(parse_custom("2025-07-01", &now), None);
        assert_eq!(parse_custom("soon", &now), None);
    }
}
