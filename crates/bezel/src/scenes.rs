//! Named states for headless renders (`bezel --snap out.png --scene sign-in`).

use bezel_core::address::Address;
use bezel_core::trust::{Certificate, Fingerprint, Question, TrustedCertificate};
use bezel_rdp::{CertificateRequest, Ended};
use rune::TextEdit;
use time::macros::date;

use crate::app::{Bezel, Phase, Screen, SignIn, fixture_session};

type Scene = (&'static str, fn(&mut Bezel));

pub const SCENES: &[Scene] = &[
    ("error", |s| {
        s.host = TextEdit::new("dc01.corp.lan");
        s.ended = Some(Ended {
            message: "dc01.corp.lan refused the connection on port 3389".into(),
            details: "connection refused".into(),
            by_user: false,
        });
    }),
    ("sign-in", |s| {
        s.screen = Screen::SignIn(SignIn {
            address: fs01(),
            username: TextEdit::new(r"CORP\admin.max"),
            password: TextEdit::new("hunter2").secure(),
            error: Some("The username or password is incorrect".into()),
        });
    }),
    ("connecting", |s| {
        s.screen = Screen::Session(fixture_session(fs01(), Phase::Connecting, None))
    }),
    ("certificate", |s| {
        let request = CertificateRequest::detached("fs01.corp.lan", 3389, Question::FirstUse(certificate()));
        s.screen = Screen::Session(fixture_session(fs01(), Phase::Connecting, Some(request)));
    }),
    ("certificate-changed", |s| {
        let trusted = TrustedCertificate {
            sha256: "7F3A91C20D5E88B41A6CE34792FD057BC8219E44A06DF1385B72CE0913B68AE5"
                .parse()
                .expect("a fingerprint"),
            ..TrustedCertificate::new("fs01.corp.lan", 3389, &certificate(), date!(2026 - 10 - 03))
        };
        let question = Question::Changed {
            trusted,
            presented: certificate(),
        };
        let request = CertificateRequest::detached("fs01.corp.lan", 3389, question);
        s.screen = Screen::Session(fixture_session(fs01(), Phase::Connecting, Some(request)));
    }),
];

fn fs01() -> Address {
    Address {
        host: "fs01.corp.lan".into(),
        port: 3389,
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
