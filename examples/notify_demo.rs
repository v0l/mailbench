use mailbench::notify::{DesktopNotifier, Notice, Notifier};

fn main() {
    let (tx, rx) = std::sync::mpsc::channel();
    DesktopNotifier.show(
        Notice { title: "Ana".into(), body: "Lunch tomorrow?".into(), focus: 1, account: 0 },
        Box::new(move || {
            let _ = tx.send(());
        }),
    );
    let _ = rx.recv_timeout(std::time::Duration::from_secs(8));
}
