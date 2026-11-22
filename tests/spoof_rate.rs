use mailbench::spoof::{Familiarity, HistoryItem, SpoofWarning, assess, assess_with};

fn senders() -> Vec<(String, String)> {
    (1..=70_000u32).map(|uid| (format!("Sender {}", uid % 300), format!("s{}@example{}.test", uid % 300, uid % 17))).collect()
}

#[test]
#[ignore]
fn rate_on_extracted_pairs() {
    let dir = std::env::var("SPOOF_DATA").expect("SPOOF_DATA");
    let inbox = std::fs::read_to_string(format!("{dir}/inbox.tsv")).unwrap();
    let known: Vec<String> = std::fs::read_to_string(format!("{dir}/known.txt")).unwrap().lines().map(String::from).collect();
    let (mut name_alarm, mut lookalike, mut first_time, mut clean) = (0, 0, 0, 0);
    let rows: Vec<(&str, &str, i64, bool, bool)> = inbox
        .lines()
        .filter_map(|line| {
            let mut p = line.split('\t');
            let (name, address) = (p.next()?, p.next()?);
            let date = p.next().and_then(|d| d.parse().ok()).unwrap_or(0);
            let opened = p.next() == Some("1");
            let flagged = p.next() == Some("1");
            Some((name, address, date, opened, flagged))
        })
        .collect();
    let sent_to: Vec<String> = std::fs::read_to_string(format!("{dir}/sent_to.txt")).map(|s| s.lines().map(String::from).collect()).unwrap_or_default();
    let mut history: Vec<HistoryItem> = rows.iter().map(|(_, a, d, o, f)| HistoryItem { from: a, to: Vec::new(), written_by_user: false, opened: *o, flagged: *f, date: *d }).collect();
    history.push(HistoryItem { from: "me@invalid.test", to: sent_to.iter().map(String::as_str).collect(), written_by_user: true, opened: true, flagged: false, date: 0 });
    let familiar = Familiarity::build(&history, &known);
    for (name, address, date, _, _) in &rows {
        let w = assess_with(name, address, &known, Some(&familiar), *date);
        let address = *address;
        let domain = |a: &str| a.rsplit_once('@').map(|(_, d)| d.to_lowercase()).unwrap_or_default();
        for x in &w {
            match x {
                SpoofWarning::NameHasOtherAddress(shown) => {
                    name_alarm += 1;
                    eprintln!("NAME_ADDR sender_domain={} shown_domain={}", domain(address), domain(shown));
                }
                SpoofWarning::Lookalike { domain, resembles } => {
                    lookalike += 1;
                    eprintln!("LOOKALIKE {domain} vs {resembles}");
                }
                SpoofWarning::FirstTimeSender => first_time += 1,
            }
        }
        if w.is_empty() {
            clean += 1;
        }
    }
    eprintln!("name_alarm {name_alarm} lookalike {lookalike} first_time {first_time} clean {clean} known {}", known.len());
}

#[test]
fn ordinary_mail_never_raises_an_alarm() {
    let all = senders();
    let mut distinct: Vec<String> = all.iter().map(|(_, a)| a.clone()).collect();
    distinct.sort();
    distinct.dedup();
    let known: Vec<String> = distinct.iter().step_by(2).cloned().collect();
    let (mut alarms, mut first_time, mut clean) = (0, 0, 0);
    for (name, address) in &all {
        let w = assess(name, address, &known);
        if w.iter().any(SpoofWarning::is_alarm) {
            alarms += 1;
        } else if w.is_empty() {
            clean += 1;
        } else {
            first_time += 1;
        }
    }
    eprintln!("messages {} distinct senders {} alarms {alarms} first_time {first_time} clean {clean}", all.len(), distinct.len());
    assert_eq!(alarms, 0);
}
