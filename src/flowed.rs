const WRAP: usize = 72;
const HARD_LIMIT: usize = 990;

pub fn flow(text: &str) -> String {
    let normal = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out: Vec<String> = Vec::new();
    for line in normal.split('\n') {
        flow_line(line, &mut out);
    }
    out.join("\r\n")
}

fn split_quote(line: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = line;
    while let Some(r) = rest.strip_prefix('>') {
        depth += 1;
        rest = r.strip_prefix(' ').unwrap_or(r);
    }
    (depth, rest)
}

fn flow_line(line: &str, out: &mut Vec<String>) {
    if line == "-- " || line == "--" {
        out.push("-- ".to_string());
        return;
    }
    let (depth, content) = split_quote(line);
    let content = content.trim_end_matches(' ');
    let quote = ">".repeat(depth);
    if content.is_empty() {
        out.push(quote);
        return;
    }
    let mut rest = content;
    loop {
        let (head, tail) = take_chunk(rest, WRAP.saturating_sub(depth + 1).max(20));
        let soft = !tail.is_empty();
        out.push(compose_line(&quote, depth, head, soft));
        if !soft {
            break;
        }
        rest = tail;
    }
}

fn compose_line(quote: &str, depth: usize, content: &str, soft: bool) -> String {
    let stuff = depth > 0 || content.starts_with(' ') || content.starts_with('>') || content.starts_with("From ");
    let mut line = String::with_capacity(quote.len() + content.len() + 2);
    line.push_str(quote);
    if stuff {
        line.push(' ');
    }
    line.push_str(content);
    if soft && !line.ends_with(' ') {
        line.push(' ');
    }
    line
}

fn take_chunk(text: &str, width: usize) -> (&str, &str) {
    if text.chars().count() <= width && text.len() <= HARD_LIMIT {
        return (text, "");
    }
    let mut last_space = None;
    let mut count = 0;
    for (i, c) in text.char_indices() {
        if count >= width || i >= HARD_LIMIT {
            break;
        }
        if c == ' ' && i > 0 {
            last_space = Some(i);
        }
        count += 1;
    }
    if let Some(i) = last_space {
        return (&text[..i + 1], &text[i + 1..]);
    }
    let word_end = text.find(' ').map(|i| i + 1);
    match word_end {
        Some(i) if i <= HARD_LIMIT => (&text[..i], &text[i..]),
        _ => {
            let mut cut = HARD_LIMIT.min(text.len());
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            if cut == text.len() {
                (text, "")
            } else {
                (&text[..cut], &text[cut..])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lines_are_untouched() {
        assert_eq!(flow("a\nb\n\nc"), "a\r\nb\r\n\r\nc");
    }

    #[test]
    fn long_lines_break_after_a_space() {
        let words = vec!["word"; 40].join(" ");
        let out = flow(&words);
        for l in out.split("\r\n") {
            assert!(l.chars().count() <= 78, "{l}");
        }
        assert!(out.split("\r\n").next().unwrap().ends_with(' '));
        let joined: String = out.split("\r\n").collect();
        assert_eq!(joined, words);
    }

    #[test]
    fn signature_separator_keeps_its_space() {
        assert_eq!(flow("hi\n--\nname"), "hi\r\n-- \r\nname");
        assert_eq!(flow("hi\n-- \nname"), "hi\r\n-- \r\nname");
    }

    #[test]
    fn quoted_lines_keep_their_prefix_when_wrapped() {
        let q = format!("> {}", vec!["quoted"; 30].join(" "));
        let out = flow(&q);
        let lines: Vec<&str> = out.split("\r\n").collect();
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| l.starts_with("> ")));
    }

    #[test]
    fn nested_and_empty_quote_lines() {
        assert_eq!(flow(">> deep\n>\n> back"), ">> deep\r\n>\r\n> back");
    }

    #[test]
    fn leading_space_and_from_are_stuffed() {
        assert_eq!(flow(" indented\nFrom here\nx"), "  indented\r\n From here\r\nx");
    }

    #[test]
    fn trailing_spaces_on_hard_lines_are_removed() {
        assert_eq!(flow("end  \nnext"), "end\r\nnext");
    }

    #[test]
    fn a_huge_word_never_exceeds_the_line_limit() {
        let out = flow(&"é".repeat(3000));
        for l in out.split("\r\n") {
            assert!(l.len() <= 998, "{}", l.len());
        }
    }
}
