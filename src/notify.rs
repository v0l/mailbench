use serde::{Deserialize, Serialize};

const MINUTES_PER_DAY: u16 = 24 * 60;
const LISTED: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    pub start: u16,
    pub end: u16,
}

impl QuietHours {
    pub fn contains(&self, minute: u16) -> bool {
        let minute = minute % MINUTES_PER_DAY;
        if self.start == self.end {
            false
        } else if self.start < self.end {
            minute >= self.start && minute < self.end
        } else {
            minute >= self.start || minute < self.end
        }
    }
}

fn on() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyPrefs {
    #[serde(default = "on")]
    pub enabled: bool,
    #[serde(default)]
    pub quiet: Option<QuietHours>,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        NotifyPrefs { enabled: true, quiet: None }
    }
}

impl NotifyPrefs {
    pub fn allows(&self, minute: u16) -> bool {
        self.enabled && !self.quiet.is_some_and(|q| q.contains(minute))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incoming {
    pub id: u64,
    pub from: String,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub focus: u64,
    pub account: usize,
}

pub trait Notifier: Send + Sync {
    fn show(&self, notice: Notice, on_click: Box<dyn FnOnce() + Send>);
}

#[derive(Debug, Default)]
pub struct Batcher {
    armed: bool,
    pending: Vec<Incoming>,
}

impl Batcher {
    pub fn arm(&mut self) {
        self.armed = true;
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn note(&mut self, items: impl IntoIterator<Item = Incoming>) {
        if self.armed {
            self.pending.extend(items);
        }
    }

    pub fn discard(&mut self) {
        self.pending.clear();
    }

    pub fn take(&mut self, prefs: &NotifyPrefs, minute: u16) -> Option<Notice> {
        let batch = std::mem::take(&mut self.pending);
        if batch.is_empty() || !prefs.allows(minute) {
            return None;
        }
        Some(compose(&batch))
    }
}

pub fn compose(batch: &[Incoming]) -> Notice {
    let newest = batch.last().expect("a batch has a message");
    if batch.len() == 1 {
        return Notice { title: newest.from.clone(), body: newest.subject.clone(), focus: newest.id, account: 0 };
    }
    let mut lines: Vec<String> = batch.iter().rev().take(LISTED).map(|m| format!("{}: {}", m.from, m.subject)).collect();
    if batch.len() > LISTED {
        lines.push(format!("and {} more", batch.len() - LISTED));
    }
    Notice { title: format!("{} new messages", batch.len()), body: lines.join("\n"), focus: newest.id, account: 0 }
}

pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn show(&self, notice: Notice, on_click: Box<dyn FnOnce() + Send>) {
        #[cfg(target_os = "android")]
        {
            let _ = on_click;
            if let Err(e) = crate::android_bridge::post_notification(notice.account, &notice.title, &notice.body, notice.focus) {
                log::warn!("notification failed: {e}");
            }
        }
        #[cfg(not(target_os = "android"))]
        std::thread::spawn(move || {
            let mut n = notify_rust::Notification::new();
            n.appname("mailbench").summary(&notice.title).body(&notice.body);
            #[cfg(target_os = "macos")]
            {
                let _ = (n.show(), on_click);
            }
            #[cfg(not(target_os = "macos"))]
            {
                n.action("default", "Open");
                let Ok(handle) = n.show() else { return };
                handle.wait_for_action(|action| {
                    if action == "default" {
                        on_click();
                    }
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: u64, from: &str, subject: &str) -> Incoming {
        Incoming { id, from: from.into(), subject: subject.into() }
    }

    #[test]
    fn mail_from_the_first_sync_never_notifies() {
        let mut b = Batcher::default();
        b.note([mail(1, "a", "old")]);
        assert_eq!(b.take(&NotifyPrefs::default(), 600), None);
        b.arm();
        b.note([mail(2, "b", "new")]);
        assert_eq!(b.take(&NotifyPrefs::default(), 600).unwrap().body, "new");
    }

    #[test]
    fn a_single_message_shows_sender_and_subject() {
        let mut b = Batcher::default();
        b.arm();
        b.note([mail(7, "Ana", "Lunch?")]);
        let n = b.take(&NotifyPrefs::default(), 0).unwrap();
        assert_eq!((n.title.as_str(), n.body.as_str(), n.focus), ("Ana", "Lunch?", 7));
        assert_eq!(b.take(&NotifyPrefs::default(), 0), None);
    }

    #[test]
    fn a_batch_is_one_notification_listing_the_newest() {
        let mut b = Batcher::default();
        b.arm();
        b.note((1..=5).map(|i| mail(i, "p", &format!("s{i}"))));
        let n = b.take(&NotifyPrefs::default(), 0).unwrap();
        assert_eq!(n.title, "5 new messages");
        assert_eq!(n.body, "p: s5\np: s4\np: s3\nand 2 more");
        assert_eq!(n.focus, 5);
        assert_eq!(b.take(&NotifyPrefs::default(), 0), None);
    }

    #[test]
    fn disabled_accounts_and_quiet_hours_stay_silent_and_drop_the_batch() {
        let mut b = Batcher::default();
        b.arm();
        b.note([mail(1, "a", "x")]);
        assert_eq!(b.take(&NotifyPrefs { enabled: false, quiet: None }, 0), None);
        let quiet = NotifyPrefs { enabled: true, quiet: Some(QuietHours { start: 22 * 60, end: 7 * 60 }) };
        b.note([mail(2, "a", "y")]);
        assert_eq!(b.take(&quiet, 23 * 60), None);
        b.note([mail(3, "a", "z")]);
        assert!(b.take(&quiet, 8 * 60).is_some());
    }

    #[test]
    fn turning_an_account_off_then_on_only_notifies_for_mail_after_the_switch() {
        let mut b = Batcher::default();
        b.arm();
        let off = NotifyPrefs { enabled: false, quiet: None };
        b.note([mail(1, "a", "while off")]);
        assert_eq!(b.take(&off, 600), None);
        b.note([mail(2, "a", "after on")]);
        let n = b.take(&NotifyPrefs::default(), 600).unwrap();
        assert_eq!((n.body.as_str(), n.focus), ("after on", 2));
    }

    #[test]
    fn each_account_gets_its_own_notice_slot() {
        let notice = Notice { account: 2, ..compose(&[mail(9, "p", "s")]) };
        assert_eq!(notice.account, 2);
        assert_eq!(compose(&[mail(9, "p", "s")]).account, 0);
    }

    #[test]
    fn quiet_hours_wrap_past_midnight() {
        let q = QuietHours { start: 22 * 60, end: 7 * 60 };
        assert!(q.contains(23 * 60) && q.contains(0) && q.contains(6 * 60 + 59));
        assert!(!q.contains(7 * 60) && !q.contains(12 * 60) && !q.contains(21 * 60 + 59));
        let day = QuietHours { start: 9 * 60, end: 17 * 60 };
        assert!(day.contains(9 * 60) && !day.contains(17 * 60));
        assert!(!QuietHours { start: 5, end: 5 }.contains(5));
    }

    #[test]
    fn old_prefs_default_to_enabled() {
        let p: NotifyPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p, NotifyPrefs::default());
    }
}
