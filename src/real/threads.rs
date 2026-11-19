use std::collections::HashMap;

pub const WINDOW: i64 = 30 * 24 * 3600;

const PREFIXES: [&str; 9] = ["re:", "fwd:", "fw:", "aw:", "sv:", "wg:", "antw:", "vs:", "tr:"];

pub struct Input<'a> {
    pub message_id: Option<&'a str>,
    pub refs: &'a [String],
    pub subject: &'a str,
    pub date: i64,
}

fn clean(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').trim().to_lowercase()
}

pub fn normalise(subject: &str) -> (String, bool) {
    let mut rest = subject.trim();
    let mut prefixed = false;
    'strip: loop {
        for p in PREFIXES {
            if rest.len() >= p.len() && rest.is_char_boundary(p.len()) && rest[..p.len()].eq_ignore_ascii_case(p) {
                rest = rest[p.len()..].trim_start();
                prefixed = true;
                continue 'strip;
            }
        }
        break;
    }
    (rest.to_lowercase(), prefixed)
}

struct Sets {
    parent: Vec<usize>,
}

impl Sets {
    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a.max(b)] = a.min(b);
        }
    }
}

pub fn assign(inputs: &[Input<'_>], ids: &[u64]) -> Vec<u64> {
    let mut sets = Sets { parent: (0..inputs.len()).collect() };
    let mut nodes: HashMap<String, usize> = HashMap::new();
    for (i, input) in inputs.iter().enumerate() {
        let own = input.message_id.iter().map(|id| clean(id));
        let linked = input.refs.iter().map(|r| clean(r));
        for key in own.chain(linked).filter(|k| !k.is_empty()) {
            match nodes.get(&key) {
                Some(j) => sets.union(i, *j),
                None => {
                    nodes.insert(key, i);
                }
            }
        }
    }
    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for i in 0..inputs.len() {
        *sizes.entry(sets.find(i)).or_default() += 1;
    }
    let mut order: Vec<usize> = (0..inputs.len()).collect();
    order.sort_by_key(|i| inputs[*i].date);
    let mut latest: HashMap<String, (i64, usize)> = HashMap::new();
    for i in order {
        let (norm, prefixed) = normalise(inputs[i].subject);
        if norm.is_empty() {
            continue;
        }
        let alone = sizes.get(&sets.find(i)).copied().unwrap_or(1) == 1;
        if prefixed && alone {
            if let Some((date, j)) = latest.get(&norm) {
                if inputs[i].date - date <= WINDOW {
                    sets.union(i, *j);
                }
            }
        }
        latest.insert(norm, (inputs[i].date, i));
    }
    let mut lowest: HashMap<usize, u64> = HashMap::new();
    for i in 0..inputs.len() {
        let root = sets.find(i);
        let slot = lowest.entry(root).or_insert(ids[i]);
        *slot = (*slot).min(ids[i]);
    }
    (0..inputs.len()).map(|i| lowest[&sets.find(i)]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 3600;

    struct Owned {
        id: String,
        refs: Vec<String>,
        subject: String,
        date: i64,
    }

    fn msg(id: &str, refs: &[&str], subject: &str, day: i64) -> Owned {
        Owned {
            id: id.into(),
            refs: refs.iter().map(|r| r.to_string()).collect(),
            subject: subject.into(),
            date: day * DAY,
        }
    }

    fn threads(owned: &[Owned]) -> Vec<u64> {
        let inputs: Vec<Input> = owned
            .iter()
            .map(|o| Input {
                message_id: if o.id.is_empty() { None } else { Some(o.id.as_str()) },
                refs: &o.refs,
                subject: &o.subject,
                date: o.date,
            })
            .collect();
        let ids: Vec<u64> = (1..=inputs.len() as u64).collect();
        assign(&inputs, &ids)
    }

    #[test]
    fn a_reply_chain_is_one_thread() {
        let t = threads(&[
            msg("a@x", &[], "Plan", 1),
            msg("b@x", &["a@x"], "Re: Plan", 2),
            msg("c@x", &["a@x", "b@x"], "Re: Re: Plan", 3),
            msg("z@x", &[], "Other", 3),
        ]);
        assert_eq!(t[0], t[1]);
        assert_eq!(t[1], t[2]);
        assert_ne!(t[0], t[3]);
    }

    #[test]
    fn broken_references_still_join_through_a_missing_parent() {
        let t = threads(&[
            msg("b@x", &["gone@x"], "Re: Plan", 2),
            msg("c@x", &["gone@x", "b@x"], "Re: Plan", 3),
            msg("d@x", &["<GONE@x>"], "Re: Plan", 4),
        ]);
        assert_eq!(t[0], t[1]);
        assert_eq!(t[0], t[2]);
    }

    #[test]
    fn a_reply_without_headers_falls_back_to_the_subject() {
        let t = threads(&[
            msg("", &[], "Project plan", 1),
            msg("", &[], "AW: Project plan", 5),
            msg("", &[], "Sv: Fwd: PROJECT plan", 6),
            msg("", &[], "Re: Something else", 6),
            msg("", &[], "Re: Project plan", 60),
        ]);
        assert_eq!(t[0], t[1]);
        assert_eq!(t[0], t[2]);
        assert_ne!(t[0], t[3]);
        assert_ne!(t[0], t[4]);
    }

    #[test]
    fn unprefixed_repeats_are_not_merged_by_subject() {
        let t = threads(&[msg("", &[], "Newsletter", 1), msg("", &[], "Newsletter", 2)]);
        assert_ne!(t[0], t[1]);
    }

    #[test]
    fn the_same_message_in_two_folders_shares_a_thread() {
        let t = threads(&[
            msg("a@x", &[], "Plan", 1),
            msg("b@x", &["a@x"], "Re: Plan", 2),
            msg("b@x", &["a@x"], "Re: Plan", 2),
        ]);
        assert_eq!(t[0], t[1]);
        assert_eq!(t[1], t[2]);
    }

    #[test]
    fn a_reply_in_sent_joins_the_inbox_thread() {
        let t = threads(&[
            msg("a@x", &[], "Plan", 1),
            msg("mine@x", &["a@x"], "Re: Plan", 2),
            msg("c@x", &["a@x", "mine@x"], "Re: Plan", 3),
        ]);
        assert_eq!(t[0], t[1]);
        assert_eq!(t[1], t[2]);
    }

    #[test]
    fn normalise_strips_reply_prefixes() {
        assert_eq!(normalise("Re: RE: Fwd: Hello"), ("hello".to_string(), true));
        assert_eq!(normalise("Hello"), ("hello".to_string(), false));
    }
}
