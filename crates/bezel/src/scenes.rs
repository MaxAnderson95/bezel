//! Named states for headless renders (`bezel --snap out.png --scene sign-in`). Each brings its
//! own Library, so a render never shows the person's data.

use bezel_core::address::Address;
use bezel_core::library::{Connection, Credential, Group, Snapshot};
use bezel_core::trust::{Certificate, Fingerprint, Question, TrustedCertificate};
use bezel_rdp::{CertificateRequest, Ended};
use rune::TextEdit;
use time::macros::date;

use crate::app::{
    Bezel, Destination, Library, Opening, Phase, Screen, Session, fixture_retry, fixture_session, fixture_sign_in,
};
use crate::manage::Delete;
use crate::windows::Window;

type Scene = (&'static str, fn(&mut Bezel));

pub const SCENES: &[Scene] = &[
    ("error", |s| {
        s.library = Library::Ready(library());
        let ended = Ended {
            message: "dc01.corp.lan refused the connection on port 3389".into(),
            details: "connection refused".into(),
            by_user: false,
        };
        tab(
            s,
            session(1, Destination::new_host(address("dc01.corp.lan")), Screen::Ended(ended)),
        );
    }),
    ("first-run", |s| s.library = Library::Ready(Snapshot::default())),
    ("library-error", |s| {
        s.library = Library::Failed(
            "~/Library/Application Support/Bezel/bezel.toml has an error on line 12: invalid type: string \"rdp\", expected u16".into(),
        )
    }),
    ("connect-box", |s| {
        s.library = Library::Ready(library());
        s.here.palette.query = TextEdit::new("dc");
    }),
    ("sidebar", |s| {
        s.library = Library::Ready(library());
        tab(s, connected(1, &dc01()));
    }),
    ("sidebar-hidden", |s| {
        s.library = Library::Ready(library());
        s.sidebar = false;
    }),
    ("tabs", |s| {
        s.library = Library::Ready(library());
        tabs(s);
    }),
    ("palette", |s| {
        s.library = Library::Ready(library());
        tabs(s);
        s.here.palette.open = true;
        s.here.palette.query = TextEdit::new("dc");
        s.here.palette.selected = 1;
    }),
    ("palette-new-host", |s| {
        s.library = Library::Ready(library());
        tabs(s);
        s.here.palette.open = true;
        s.here.palette.query = TextEdit::new("10.0.0.5:3390");
    }),
    ("session-window", |s| {
        s.library = Library::Ready(library());
        tabs(s);
        s.open_window(2, None);
        s.here.window = Window::Session(2);
    }),
    ("session-window-sign-in", |s| {
        s.library = Library::Ready(library());
        let id = s.open_host(address("10.0.0.5"), Opening::Window);
        s.here.window = Window::Session(id);
    }),
    ("quit", |s| {
        s.library = Library::Ready(library());
        tabs(s);
        s.quitting = true;
    }),
    ("sign-in", |s| {
        s.library = Library::Ready(library());
        s.open(&"n_lab".into(), Opening::Tab, false);
        if let Some(Screen::SignIn(sign_in)) = s.sessions.last_mut().map(|x| &mut x.screen) {
            sign_in.username = TextEdit::new(r"CORP\admin.max");
            sign_in.password = TextEdit::new("hunter2").secure();
            sign_in.error = Some("The username or password is incorrect".into());
        }
    }),
    ("sign-in-save", |s| {
        s.library = Library::Ready(library());
        let id = s.open_host(address("10.0.0.5"), Opening::Tab);
        if let Some(Screen::SignIn(sign_in)) = s.sessions.last_mut().map(|x| &mut x.screen) {
            sign_in.username = TextEdit::new("administrator");
            sign_in.password = TextEdit::new("hunter2").secure();
        }
        s.toggle_save_credential(id);
    }),
    ("sign-in-saved", |s| {
        s.library = Library::Ready(library());
        s.open(&"n_lab".into(), Opening::Tab, false);
        if let Some(id) = s.active {
            s.sign_in_with(id, Some("c_admin".into()));
        }
    }),
    ("retry", |s| {
        s.library = Library::Ready(library());
        let admin = library().credentials[0].clone();
        tab(s, session(1, Destination::connection(&dc01()), fixture_retry(admin)));
    }),
    ("connecting", |s| {
        s.library = Library::Ready(library());
        tab(
            s,
            fixture_session(1, Destination::new_host(fs01()), Phase::Connecting, None),
        );
    }),
    ("certificate", |s| {
        s.library = Library::Ready(library());
        let request = CertificateRequest::detached("fs01.corp.lan", 3389, Question::FirstUse(certificate()));
        tab(
            s,
            fixture_session(1, Destination::new_host(fs01()), Phase::Connecting, Some(request)),
        );
    }),
    ("certificate-changed", |s| {
        s.library = Library::Ready(library());
        let question = Question::Changed {
            trusted: trusted(),
            presented: certificate(),
        };
        let request = CertificateRequest::detached("fs01.corp.lan", 3389, question);
        tab(
            s,
            fixture_session(1, Destination::new_host(fs01()), Phase::Connecting, Some(request)),
        );
    }),
    ("new-connection", |s| {
        s.library = Library::Ready(library());
        s.new_connection(Some("g_infra".into()));
        if let Some(crate::manage::Sheet::Connection(form)) = &mut s.sheet {
            form.host = TextEdit::new("dc03.corp.lan");
            form.credential = Some("c_admin".into());
        }
    }),
    ("edit-connection", |s| {
        s.library = Library::Ready(library());
        s.edit_connection(&"n_dc01".into());
    }),
    ("new-group", |s| {
        s.library = Library::Ready(library());
        s.new_group();
    }),
    ("credentials", |s| {
        s.library = Library::Ready(library());
        s.open_credentials(None);
    }),
    ("new-credential", |s| {
        s.library = Library::Ready(library());
        s.open_credentials(None);
        s.new_credential();
    }),
    ("delete-credential", |s| {
        s.library = Library::Ready(library());
        s.open_credentials(None);
        s.ask_delete(Delete::Credential("c_admin".into()));
    }),
    ("delete-group", |s| {
        s.library = Library::Ready(library());
        s.ask_delete(Delete::Group("g_infra".into()));
    }),
    ("certificates", |s| {
        s.library = Library::Ready(library());
        s.open_certificates();
    }),
];

fn session(id: u64, to: Destination, screen: Screen) -> Session {
    let mut session = fixture_session(id, to, Phase::Connecting, None);
    session.screen = screen;
    session
}

fn connected(id: u64, connection: &Connection) -> Session {
    fixture_session(
        id,
        Destination::connection(connection),
        Phase::Connected {
            width: 2048,
            height: 1280,
        },
        None,
    )
}

/// Adds the Session as a Tab and makes it the current one.
fn tab(s: &mut Bezel, session: Session) {
    s.active = Some(session.id);
    s.sessions.push(session);
}

/// dc01 connected and current, dc02 connecting, fs01 asking for a password, a New host that
/// ended.
fn tabs(s: &mut Bezel) {
    let library = library();
    let connection = |id: &str| library.connection(&id.into()).cloned().expect("a fixture connection");
    let ended = Ended {
        message: "10.0.0.9 refused the connection on port 3389".into(),
        details: "connection refused".into(),
        by_user: false,
    };
    s.sessions = vec![
        connected(1, &dc01()),
        fixture_session(
            2,
            Destination::connection(&connection("n_dc02")),
            Phase::Connecting,
            None,
        ),
        session(3, Destination::connection(&connection("n_fs01")), fixture_sign_in()),
        session(4, Destination::new_host(address("10.0.0.9")), Screen::Ended(ended)),
    ];
    s.active = Some(1);
}

fn address(host: &str) -> Address {
    Address::parse(host).expect("a fixture address")
}

fn fs01() -> Address {
    address("fs01.corp.lan")
}

fn dc01() -> Connection {
    Connection {
        id: "n_dc01".into(),
        name: "dc01".into(),
        host: "dc01.corp.lan".into(),
        port: 3389,
        credential: Some("c_admin".into()),
        group: Some("g_infra".into()),
    }
}

fn library() -> Snapshot {
    let connection = |id: &str, name: &str, credential: Option<&str>, group: Option<&str>| Connection {
        id: id.into(),
        name: name.into(),
        host: format!("{name}.corp.lan"),
        port: 3389,
        credential: credential.map(Into::into),
        group: group.map(Into::into),
    };
    Snapshot {
        groups: vec![
            Group {
                id: "g_infra".into(),
                name: "Infrastructure".into(),
            },
            Group {
                id: "g_files".into(),
                name: "File servers".into(),
            },
        ],
        credentials: vec![
            Credential {
                id: "c_admin".into(),
                name: "Domain admin".into(),
                username: r"CORP\admin.max".into(),
                password_updated: Some(date!(2026 - 08 - 23)),
            },
            Credential {
                id: "c_local".into(),
                name: "Local administrator".into(),
                username: "administrator".into(),
                password_updated: None,
            },
        ],
        connections: vec![
            dc01(),
            connection("n_dc02", "dc02", Some("c_admin"), Some("g_infra")),
            connection("n_ca01", "ca01", Some("c_admin"), Some("g_infra")),
            connection("n_fs01", "fs01", Some("c_admin"), Some("g_files")),
            connection("n_fs02", "fs02", Some("c_local"), Some("g_files")),
            connection("n_lab", "lab-win11", None, None),
        ],
        trusted_certificates: vec![trusted()],
        ..Snapshot::default()
    }
}

fn trusted() -> TrustedCertificate {
    TrustedCertificate {
        sha256: "7F3A91C20D5E88B41A6CE34792FD057BC8219E44A06DF1385B72CE0913B68AE5"
            .parse()
            .expect("a fingerprint"),
        ..TrustedCertificate::new("fs01.corp.lan", 3389, &certificate(), date!(2026 - 10 - 03))
    }
}

fn certificate() -> Certificate {
    Certificate {
        sha256: Fingerprint::of(b"fs01"),
        subject: "fs01.corp.lan".into(),
        issuer: "fs01.corp.lan".into(),
        self_signed: true,
        not_before: date!(2026 - 03 - 14),
        not_after: date!(2027 - 03 - 14),
    }
}
