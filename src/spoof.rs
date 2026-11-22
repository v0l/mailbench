#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpoofWarning {
    NameHasOtherAddress(String),
    Lookalike { domain: String, resembles: String },
    FirstTimeSender,
}

impl SpoofWarning {
    pub fn text(&self) -> String {
        match self {
            SpoofWarning::NameHasOtherAddress(a) => format!("name shows {a}, sender is different"),
            SpoofWarning::Lookalike { domain, resembles } => format!("{domain} looks like {resembles}"),
            SpoofWarning::FirstTimeSender => "first time sender".to_string(),
        }
    }

    pub fn is_alarm(&self) -> bool {
        !matches!(self, SpoofWarning::FirstTimeSender)
    }
}

fn domain_of(address: &str) -> Option<String> {
    address.trim().rsplit_once('@').map(|(_, d)| d.trim().trim_end_matches('.').to_lowercase()).filter(|d| !d.is_empty())
}

fn embedded_address(name: &str) -> Option<String> {
    name.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '(' | ')' | '"' | '\'' | ',' | ';' | '[' | ']'))
        .find_map(|t| {
            let (local, domain) = t.split_once('@')?;
            let domain = domain.trim_end_matches('.');
            let ok = !local.is_empty() && domain.contains('.') && !domain.starts_with('.');
            ok.then(|| t.trim_end_matches('.').to_lowercase())
        })
}

fn decode_punycode(input: &str) -> Option<String> {
    const BASE: u32 = 36;
    const TMIN: u32 = 1;
    const TMAX: u32 = 26;
    let (basic, rest) = match input.rfind('-') {
        Some(i) => (&input[..i], &input[i + 1..]),
        None => ("", input),
    };
    let mut out: Vec<char> = basic.chars().collect();
    let (mut n, mut i, mut bias) = (128u32, 0u32, 72u32);
    let mut digits = rest.chars().peekable();
    while digits.peek().is_some() {
        let old = i;
        let mut w = 1u32;
        let mut k = BASE;
        loop {
            let c = digits.next()?;
            let d = match c {
                'a'..='z' => c as u32 - 'a' as u32,
                'A'..='Z' => c as u32 - 'A' as u32,
                '0'..='9' => c as u32 - '0' as u32 + 26,
                _ => return None,
            };
            i = i.checked_add(d.checked_mul(w)?)?;
            let t = if k <= bias { TMIN } else if k >= bias + TMAX { TMAX } else { k - bias };
            if d < t {
                break;
            }
            w = w.checked_mul(BASE - t)?;
            k += BASE;
        }
        let len = out.len() as u32 + 1;
        let mut delta = if old == 0 { (i - old) / 700 } else { (i - old) / 2 };
        delta += delta / len;
        let mut k2 = 0;
        while delta > ((BASE - TMIN) * TMAX) / 2 {
            delta /= BASE - TMIN;
            k2 += BASE;
        }
        bias = k2 + (BASE - TMIN + 1) * delta / (delta + 38);
        n = n.checked_add(i / len)?;
        i %= len;
        out.insert(i as usize, char::from_u32(n)?);
        i += 1;
    }
    Some(out.into_iter().collect())
}

pub fn unicode_domain(domain: &str) -> String {
    domain
        .split('.')
        .map(|l| match l.strip_prefix("xn--") {
            Some(p) => decode_punycode(p).unwrap_or_else(|| l.to_string()),
            None => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn confusable(c: char) -> char {
    match c {
        'а' | 'α' | 'ɑ' => 'a',
        'в' => 'b',
        'с' | 'ϲ' => 'c',
        'ԁ' => 'd',
        'е' | 'ε' | 'ҽ' => 'e',
        'ɡ' => 'g',
        'һ' => 'h',
        'і' | 'ι' | 'ı' | 'ӏ' | 'l' | '1' | '|' => 'l',
        'ј' => 'j',
        'к' | 'κ' => 'k',
        'м' => 'm',
        'η' | 'п' => 'n',
        'о' | 'ο' | 'σ' | '0' => 'o',
        'р' | 'ρ' => 'p',
        'ԛ' => 'q',
        'ѕ' => 's',
        'т' | 'τ' => 't',
        'υ' | 'ս' => 'u',
        'ν' | 'ѵ' => 'v',
        'ԝ' => 'w',
        'х' | 'χ' => 'x',
        'у' | 'γ' => 'y',
        other => other,
    }
}

fn skeleton(domain: &str) -> String {
    let lowered: String = unicode_domain(domain).chars().flat_map(|c| c.to_lowercase()).collect();
    let mapped: String = lowered.chars().map(confusable).collect();
    mapped.replace("rn", "m").replace("vv", "w")
}

fn edit_distance_one(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a == b || a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    let prefix = short.iter().zip(long.iter()).take_while(|(x, y)| x == y).count();
    if short.len() == long.len() {
        short[prefix + 1..] == long[prefix + 1..]
    } else {
        short[prefix..] == long[prefix + 1..]
    }
}

fn labels(domain: &str) -> (String, String) {
    let parts: Vec<&str> = domain.split('.').collect();
    let n = parts.len();
    if n < 2 {
        return (domain.to_string(), String::new());
    }
    (parts[n - 2].to_string(), parts[n - 1].to_string())
}

fn resembles(domain: &str, known: &str) -> bool {
    if domain == known {
        return false;
    }
    if skeleton(domain) == skeleton(known) {
        return true;
    }
    let (name, tld) = labels(&unicode_domain(domain));
    let (kname, ktld) = labels(&unicode_domain(known));
    name.chars().count() >= 6 && tld == ktld && edit_distance_one(&name, &kname)
}

pub fn assess(from_name: &str, from_address: &str, known: &[String]) -> Vec<SpoofWarning> {
    let from = from_address.trim().to_lowercase();
    let mut out = Vec::new();
    if let Some(shown) = embedded_address(from_name).filter(|a| *a != from) {
        out.push(SpoofWarning::NameHasOtherAddress(shown));
    }
    let Some(domain) = domain_of(&from) else {
        return out;
    };
    let known_lower: Vec<String> = known.iter().map(|k| k.trim().to_lowercase()).collect();
    let mut seen = Vec::new();
    for k in &known_lower {
        let Some(kd) = domain_of(k) else { continue };
        if seen.contains(&kd) {
            continue;
        }
        seen.push(kd.clone());
        if known_lower.iter().any(|x| domain_of(x).as_deref() == Some(domain.as_str())) {
            break;
        }
        if resembles(&domain, &kd) {
            out.push(SpoofWarning::Lookalike { domain: domain.clone(), resembles: kd });
            break;
        }
    }
    if !known_lower.contains(&from) {
        out.push(SpoofWarning::FirstTimeSender);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        vec!["alice@paypal.com".into(), "bob@example.org".into()]
    }

    #[test]
    fn name_with_a_different_address_is_flagged() {
        let w = assess("support@paypal.com", "x@evil.test", &[]);
        assert!(w.contains(&SpoofWarning::NameHasOtherAddress("support@paypal.com".into())));
    }

    #[test]
    fn name_with_the_same_address_is_fine() {
        let w = assess("Alice <alice@paypal.com>", "Alice@PayPal.com", &known());
        assert!(w.is_empty());
    }

    #[test]
    fn plain_name_is_not_flagged() {
        let w = assess("Alice Smith", "alice@paypal.com", &known());
        assert!(w.is_empty());
    }

    #[test]
    fn unknown_sender_gets_first_time_badge() {
        let w = assess("Carol", "carol@fresh.test", &known());
        assert_eq!(w, vec![SpoofWarning::FirstTimeSender]);
        assert!(!w[0].is_alarm());
    }

    #[test]
    fn known_sender_gets_no_badge() {
        assert!(assess("Bob", "bob@example.org", &known()).is_empty());
    }

    #[test]
    fn another_person_at_a_known_domain_is_not_a_lookalike() {
        let w = assess("Dan", "dan@paypal.com", &known());
        assert_eq!(w, vec![SpoofWarning::FirstTimeSender]);
    }

    #[test]
    fn cyrillic_lookalike_domain_is_flagged() {
        let w = assess("Alice", "alice@раypal.com", &known());
        assert!(w.iter().any(|x| matches!(x, SpoofWarning::Lookalike { resembles, .. } if resembles == "paypal.com")));
    }

    #[test]
    fn punycode_lookalike_domain_is_flagged() {
        assert_eq!(unicode_domain("xn--ypal-43d9g.com"), "раypal.com");
        let w = assess("Alice", "alice@xn--ypal-43d9g.com", &known());
        assert!(w.iter().any(|x| matches!(x, SpoofWarning::Lookalike { .. })));
    }

    #[test]
    fn rn_for_m_and_digit_swaps_are_flagged() {
        let known = vec!["me@mircosoft.com".to_string()];
        assert!(assess("x", "a@rnircosoft.com", &known).iter().any(|w| matches!(w, SpoofWarning::Lookalike { .. })));
        let known = vec!["me@google.com".to_string()];
        assert!(assess("x", "a@g00gle.com", &known).iter().any(|w| matches!(w, SpoofWarning::Lookalike { .. })));
    }

    #[test]
    fn one_letter_typo_of_a_long_domain_is_flagged() {
        let w = assess("x", "a@exampel.org", &known());
        assert!(!w.iter().any(|x| matches!(x, SpoofWarning::Lookalike { .. })));
        let w = assess("x", "a@exmple.org", &known());
        assert!(w.iter().any(|x| matches!(x, SpoofWarning::Lookalike { .. })));
    }

    #[test]
    fn unrelated_domains_are_not_lookalikes() {
        let w = assess("x", "a@totally-different.net", &known());
        assert_eq!(w, vec![SpoofWarning::FirstTimeSender]);
    }

    #[test]
    fn punycode_decodes() {
        assert_eq!(unicode_domain("xn--bcher-kva.example"), "bücher.example");
    }
}
