use crate::attach::safe_name;
use percent_encoding::percent_decode_str;

pub fn parse_picked(joined: &str) -> Vec<String> {
    joined.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

pub fn display_name(uri: &str, reported: Option<&str>) -> String {
    if let Some(name) = reported.map(str::trim).filter(|n| !n.is_empty()) {
        return safe_name(name, "attachment");
    }
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let last = path.rsplit('/').next().unwrap_or(path);
    let decoded = percent_decode_str(last).decode_utf8_lossy().to_string();
    let tail = decoded.rsplit(['/', ':']).next().unwrap_or(&decoded);
    safe_name(tail, "attachment")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picked_list_skips_blank_lines() {
        assert_eq!(parse_picked("content://a/1\n\n content://a/2 \n"), vec!["content://a/1", "content://a/2"]);
        assert!(parse_picked("").is_empty());
    }

    #[test]
    fn the_reported_display_name_wins_and_is_made_safe() {
        assert_eq!(display_name("content://x/document/5", Some("report.pdf")), "report.pdf");
        assert_eq!(display_name("content://x/document/5", Some("../a:b.txt")), "_a_b.txt");
    }

    #[test]
    fn a_missing_name_comes_from_the_uri() {
        assert_eq!(display_name("content://x/tree/primary%3ADownload/document/primary%3ADownload%2Fnotes%20v2.txt", None), "notes v2.txt");
        assert_eq!(display_name("content://x/document/msf%3A12", Some("  ")), "12");
        assert_eq!(display_name("content://x/", None), "attachment");
    }
}
