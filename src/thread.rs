use std::collections::HashSet;

const SNIPPET_CHARS: usize = 140;

fn is_quote_line(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn is_attribution(line: &str) -> bool {
    let t = line.trim();
    (t.starts_with("On ") && t.ends_with("wrote:")) || (t.starts_with("-----Original Message") || t.starts_with("From:") && t.len() < 4)
}

fn is_signature_marker(line: &str) -> bool {
    matches!(line.trim_end(), "-- " | "--" | "__")
}

pub fn new_text(paragraphs: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    'outer: for p in paragraphs {
        let mut kept = Vec::new();
        for line in p.lines() {
            if is_signature_marker(line) {
                if !kept.is_empty() {
                    out.push(kept.join("\n"));
                }
                break 'outer;
            }
            if is_quote_line(line) || is_attribution(line) {
                continue;
            }
            kept.push(line);
        }
        if !kept.is_empty() {
            out.push(kept.join("\n"));
        }
    }
    out
}

pub fn snippet(paragraphs: &[String]) -> String {
    let text = new_text(paragraphs).into_iter().next().unwrap_or_default();
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= SNIPPET_CHARS {
        return flat;
    }
    let cut: String = flat.chars().take(SNIPPET_CHARS).collect();
    format!("{}...", cut.trim_end())
}

fn normal(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn without_parent_text(paragraphs: &[String], parent: &[String]) -> Vec<String> {
    let parent_paras: HashSet<String> = new_text(parent).iter().map(|p| normal(p)).filter(|p| !p.is_empty()).collect();
    let parent_lines: HashSet<String> = parent.iter().flat_map(|p| p.lines()).map(normal).filter(|l| !l.is_empty()).collect();
    paragraphs
        .iter()
        .filter(|p| {
            let n = normal(p);
            if n.is_empty() {
                return false;
            }
            if parent_paras.contains(&n) {
                return false;
            }
            let lines: Vec<String> = p.lines().map(|l| normal(l.trim_start_matches(|c: char| c == '>' || c.is_whitespace()))).filter(|l| !l.is_empty()).collect();
            let all_parent = !lines.is_empty() && lines.iter().all(|l| parent_lines.contains(l));
            !(all_parent && p.lines().all(|l| is_quote_line(l) || l.trim().is_empty() || is_attribution(l)))
        })
        .cloned()
        .collect()
}

pub fn without_parent_quote(quoted: &[String], parent: &[String]) -> Vec<String> {
    let parent_lines: HashSet<String> = parent.iter().flat_map(|p| p.lines()).map(normal).filter(|l| !l.is_empty()).collect();
    if quoted.is_empty() {
        return Vec::new();
    }
    let stripped: Vec<String> = quoted.iter().map(|l| normal(l.trim_start_matches(|c: char| c == '>' || c.is_whitespace()))).collect();
    let covered = stripped.iter().filter(|l| l.is_empty() || parent_lines.contains(*l)).count();
    if covered == stripped.len() {
        Vec::new()
    } else {
        quoted.to_vec()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenCards {
    pub head: Option<u64>,
    pub open: Vec<u64>,
    pub quotes: Vec<u64>,
    pub touched: bool,
}

impl OpenCards {
    pub fn reset(&mut self, head: Option<u64>, ids: &[u64], default_open: Option<u64>) {
        if self.head == head {
            self.open.retain(|i| ids.contains(i));
            return;
        }
        *self = OpenCards { head, open: default_open.into_iter().collect(), quotes: Vec::new(), touched: false };
    }

    pub fn is_open(&self, id: u64) -> bool {
        self.open.contains(&id)
    }

    pub fn toggle(&mut self, id: u64) {
        self.touched = true;
        if let Some(i) = self.open.iter().position(|o| *o == id) {
            self.open.remove(i);
        } else {
            self.open.push(id);
        }
    }

    pub fn open_one(&mut self, id: u64) {
        self.touched = true;
        if !self.open.contains(&id) {
            self.open.push(id);
        }
    }

    pub fn expand_all(&mut self, ids: &[u64]) {
        self.touched = true;
        self.open = ids.to_vec();
    }

    pub fn collapse_all(&mut self) {
        self.touched = true;
        self.open.clear();
    }

    pub fn quotes_shown(&self, id: u64) -> bool {
        self.quotes.contains(&id)
    }

    pub fn toggle_quotes(&mut self, id: u64) {
        if let Some(i) = self.quotes.iter().position(|o| *o == id) {
            self.quotes.remove(i);
        } else {
            self.quotes.push(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn snippet_takes_the_first_new_line_and_skips_quoted_history() {
        let body = p(&["On Mon, Ann wrote:\n> old words here", "Sounds good, see you then.", "More detail."]);
        assert_eq!(snippet(&body), "Sounds good, see you then.");
    }

    #[test]
    fn snippet_never_comes_from_quoted_lines() {
        let body = p(&["> only old text\n> and more"]);
        assert_eq!(snippet(&body), "");
    }

    #[test]
    fn signature_is_not_snippet_text() {
        let body = p(&["-- \nKieran\nCEO"]);
        assert_eq!(snippet(&body), "");
        let body = p(&["Thanks\n-- \nKieran"]);
        assert_eq!(snippet(&body), "Thanks");
    }

    #[test]
    fn long_snippets_are_cut_with_dots() {
        let body = p(&[&"word ".repeat(80)]);
        let s = snippet(&body);
        assert!(s.ends_with("...") && s.chars().count() <= SNIPPET_CHARS + 3);
    }

    #[test]
    fn paragraph_equal_to_the_parent_body_is_dropped() {
        let parent = p(&["Hi,\nCan we meet on Friday?"]);
        let child = p(&["Hi,  Can we meet\non Friday?", "Yes, Friday works."]);
        assert_eq!(without_parent_text(&child, &parent), p(&["Yes, Friday works."]));
    }

    #[test]
    fn quoted_block_that_repeats_the_parent_is_dropped_but_a_foreign_one_stays() {
        let parent = p(&["Can we meet on Friday?", "Bring the board."]);
        let same = vec!["> Can we meet on Friday?".to_string(), ">".to_string(), "> Bring the board.".to_string()];
        assert!(without_parent_quote(&same, &parent).is_empty());
        let other = vec!["> Something else entirely".to_string()];
        assert_eq!(without_parent_quote(&other, &parent), other);
    }

    #[test]
    fn open_cards_remember_choices_until_the_conversation_changes() {
        let mut c = OpenCards::default();
        c.reset(Some(1), &[1, 2, 3], Some(3));
        assert!(c.is_open(3) && !c.is_open(1));
        c.toggle(1);
        c.reset(Some(1), &[1, 2, 3], Some(3));
        assert!(c.is_open(1) && c.is_open(3));
        c.collapse_all();
        assert!(c.open.is_empty());
        c.expand_all(&[1, 2, 3]);
        assert_eq!(c.open, vec![1, 2, 3]);
        c.reset(Some(9), &[9], Some(9));
        assert_eq!(c.open, vec![9]);
    }
}
