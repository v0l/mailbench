use mailbench::backend::*;

fn imap(host: &str) -> ImapBackend {
    ImapBackend::new(ImapConfig::new(host, 993, Security::Tls, Auth::Password { user: "u".into(), password: "p".into() }))
}

#[test]
fn gmail_hosts_file_the_sent_copy_themselves_and_others_do_not() {
    assert!(imap("imap.gmail.com").sent_copy_by_server());
    assert!(imap("imap.googlemail.com").sent_copy_by_server());
    assert!(!imap("imap.example.com").sent_copy_by_server());
    assert!(!imap("127.0.0.1").sent_copy_by_server());
    let account = Account { mail: Box::new(imap("imap.gmail.com")), outbound: Box::new(imap("smtp.gmail.com")) };
    assert!(account.sent_copy_by_server());
}
