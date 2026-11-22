use mailbench::spoof::{SpoofWarning, assess};

fn senders() -> Vec<(String, String)> {
    (1..=70_000u32).map(|uid| (format!("Sender {}", uid % 300), format!("s{}@example{}.test", uid % 300, uid % 17))).collect()
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
