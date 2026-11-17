pub struct Account {
    pub name: &'static str,
    pub address: &'static str,
    pub unread: u32,
    pub state: AccountState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AccountState {
    Idle,
    Syncing,
    Failed,
}

pub const ACCOUNTS: [Account; 3] = [
    Account { name: "kestrel", address: "mara.ilves@kestrel-instruments.example", unread: 9, state: AccountState::Idle },
    Account { name: "home", address: "mara@ilves.example", unread: 4, state: AccountState::Syncing },
    Account { name: "lists", address: "mara@lists.ilves.example", unread: 1, state: AccountState::Idle },
];

pub const FOLDERS: [(&str, u32); 6] =
    [("inbox", 14), ("drafts", 2), ("sent", 0), ("archive", 0), ("junk", 0), ("trash", 0)];

pub const LABELS: [(&str, u32); 3] = [("fab", 5), ("bench night", 1), ("invoices", 0)];

#[derive(Clone, Copy)]
pub struct Msg {
    pub from: &'static str,
    pub subject: &'static str,
    pub snippet: &'static str,
    pub when: &'static str,
    pub unread: bool,
    pub flagged: bool,
    pub new: bool,
    pub attach: bool,
    pub account: usize,
    pub count: u8,
}

fn m(from: &'static str, subject: &'static str, when: &'static str, flags: &str, account: usize, count: u8) -> Msg {
    Msg {
        from,
        subject,
        snippet: "",
        when,
        unread: flags.contains('u'),
        flagged: flags.contains('f'),
        new: flags.contains('n'),
        attach: flags.contains('a'),
        account,
        count,
    }
}

impl Msg {
    fn s(mut self, snippet: &'static str) -> Msg {
        self.snippet = snippet;
        self
    }
}

pub fn inbox() -> Vec<Msg> {
    vec![
        m("Tomás Rivera", "Rev C stencil quote and lead time", "09:41", "unaf", 0, 3)
            .s("Quote attached. 0.12 mm stainless, framed, five working days from paid order."),
        m("Wren Okafor", "Re: Reflow profile for the SAC305 run", "09:12", "unf", 0, 6)
            .s("Peak at 245 was too hot for the tantalums. Try 238 with a longer soak."),
        m("GitHub", "[kestrel/firmware] PR #212: recover the I2C bus after a stuck SDA", "08:55", "un", 0, 1)
            .s("ines-d requested your review on this pull request."),
        m("Partsbin", "Your order PB-48213 has shipped", "08:30", "ua", 1, 1)
            .s("Tracking 7731 0042 9918. Expected Monday 20 Nov."),
        m("Ines Duarte", "Bench night: Thursday, bring a scope", "08:02", "u", 2, 4)
            .s("Room is booked from 18:30. I have the logic analyser and a lot of pizza."),
        m("Halden Fabrication", "Invoice 2023-117, due 29 Nov", "Thu", "a", 0, 1)
            .s("Net 14. Bank details are on the invoice."),
        m("Bench Weekly", "Issue 96: eight scope probes, measured", "Thu", "u", 1, 1)
            .s("We put a 1x, 10x and 100x probe on the same 50 MHz square wave."),
        m("Jonas Weber", "Firmware release notes for 2.4", "Thu", "", 0, 2)
            .s("Draft below. Please check the bootloader paragraph before I send it."),
        m("Priya Natarajan", "Re: Calibration certificate for the DMM", "Wed", "f", 0, 5)
            .s("They need the serial number and the date of the last cert."),
        m("Linux Kernel Mailing List digest", "linux-embedded: 31 messages", "Wed", "", 2, 1),
        m("Aisha Karimi", "Datasheet for the new level shifter", "Wed", "a", 0, 1),
        m("Harbour Credit Union", "Your statement for October is ready", "Tue", "", 1, 1),
        m("GitHub", "[kestrel/firmware] Run failed: build-arm (main)", "Tue", "", 0, 1),
        m("Oskar Lindqvist", "Re: Shared enclosure drawings", "Tue", "", 0, 7),
        m("Cinder Café", "Your loyalty card has 9 of 10 stamps", "Mon", "", 1, 1),
        m("Ines Duarte", "Re: Panel layout for the 4 channel board", "Mon", "a", 0, 9),
        m("Partsbin", "Back in stock: ADS1115 breakout", "Mon", "", 1, 1),
        m("Marisol Quinn", "Notes from Friday's design review", "Sun", "", 0, 2),
        m("Tomás Rivera", "Re: Gerber check for Rev B", "19 Nov", "", 0, 4),
        m("Hackspace committee", "Minutes: 12 November", "17 Nov", "a", 2, 1),
        m("Wren Okafor", "Re: Which clamp meter did you buy?", "16 Nov", "", 0, 3),
        m("GitHub", "[kestrel/firmware] PR #209 merged", "15 Nov", "", 0, 1),
        m("Priya Natarajan", "Travel dates for the Nuremberg trip", "14 Nov", "", 0, 2),
        m("Halden Fabrication", "Your Rev B order is complete", "13 Nov", "a", 0, 1),
        m("Jonas Weber", "Standup moves to 10:00", "13 Nov", "", 0, 1),
        m("Bench Weekly", "Issue 95: the mess on your desk", "10 Nov", "", 1, 1),
        m("Aisha Karimi", "Re: Reference clock options", "9 Nov", "", 0, 6),
        m("Ines Duarte", "Photos from the soldering workshop", "8 Nov", "a", 2, 1),
        m("Oskar Lindqvist", "Re: Cable gland sizes", "7 Nov", "", 0, 2),
        m("Partsbin", "Your order PB-47990 was delivered", "6 Nov", "", 1, 1),
    ]
}

pub struct Turn {
    pub from: &'static str,
    pub to: &'static str,
    pub when: &'static str,
    pub mine: bool,
    pub new: bool,
    pub paragraphs: &'static [&'static str],
    pub quoted_lines: u32,
}

pub const THREAD_SUBJECT: &str = "Rev C stencil quote and lead time";

pub const THREAD: [Turn; 3] = [
    Turn {
        from: "Tomás Rivera",
        to: "me, Wren Okafor",
        when: "Wed 15 Nov 16:20",
        mine: false,
        new: false,
        paragraphs: &[
            "Hi Mara, the Rev C Gerbers passed our check. Two small things: the fiducials on the bottom layer are 0.9 mm and we want 1.0 mm, and the paste layer has no aperture for U7, the exposed pad.",
            "I can fix both on our side if you say so. I will send the stencil quote once you confirm.",
        ],
        quoted_lines: 0,
    },
    Turn {
        from: "me",
        to: "Tomás Rivera, Wren Okafor",
        when: "Thu 16 Nov 09:52",
        mine: true,
        new: false,
        paragraphs: &[
            "Yes please, fix both. For U7 use a 3 by 3 window pattern with 60 percent coverage. Can you quote stainless, framed, with a 0.12 mm foil? Wren will do the reflow on Monday.",
        ],
        quoted_lines: 6,
    },
    Turn {
        from: "Tomás Rivera",
        to: "me, Wren Okafor",
        when: "Fri 17 Nov 09:41",
        mine: false,
        new: true,
        paragraphs: &[
            "Quote attached. The framed 0.12 mm stainless stencil is 86.00 EUR plus shipping, and it is five working days from a paid order. If you pay today it ships next Friday.",
            "I used the 3 by 3 window pattern on U7 as you described and moved the bottom fiducials to 1.0 mm. The updated paste layer is in the zip so you can check it against your own copy.",
            "One question: do you want the edge of the board panel left unpopulated? Wren mentioned a test coupon, and we would need to know before we cut the foil.",
        ],
        quoted_lines: 14,
    },
];

pub const ATTACHMENTS: [(&str, &str); 2] =
    [("stencil-quote-revc.pdf", "84 KB"), ("revc-paste-layer.zip", "212 KB")];

pub const BLOCKED_HOST: &str = "cdn.halden-fab.example";
