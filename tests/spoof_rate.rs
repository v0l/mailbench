use mailbench::spoof::{SpoofWarning, assess};

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
    for line in inbox.lines() {
        let Some((name, address)) = line.split_once('\t') else { continue };
        let w = assess(name, address, &known);
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
