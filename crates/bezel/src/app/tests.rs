use std::sync::Arc;
use std::time::Duration;

use bezel_core::keys::Keyboard;
use bezel_core::library::{Group, GroupId, Library as Store_};
use bezel_core::secrets::MemorySecrets;
use rune::{Env, Mods, Runtime};

use super::*;
use crate::manage::{Delete, Selected};

/// A Bezel whose Library is a file in a temporary folder, holding the Groups Infrastructure
/// and File servers, the Credential Domain admin (password "old") used by dc01 and dc02, and
/// the ungrouped lab, which asks each time. Hosts point at a local port that takes connections
/// and never answers, so a Session stays connecting until the test says otherwise.
struct Fixture {
    bezel: Bezel,
    secrets: MemorySecrets,
    _alive: (tempfile::TempDir, std::net::TcpListener),
}

fn fixture() -> Fixture {
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bezel.toml");
    let secrets = MemorySecrets::default();
    let mut library = Store_::open(&path, secrets.clone()).unwrap();
    let connection = |id: &str, name: &str, credential: Option<&str>, group: Option<&str>| Connection {
        id: id.into(),
        name: name.into(),
        host: "127.0.0.1".into(),
        port,
        credential: credential.map(Into::into),
        group: group.map(Into::into),
    };
    for change in [
        Change::AddGroup(Group {
            id: "g_infra".into(),
            name: "Infrastructure".into(),
        }),
        Change::AddGroup(Group {
            id: "g_files".into(),
            name: "File servers".into(),
        }),
        Change::AddCredential {
            credential: Credential {
                id: "c_admin".into(),
                name: "Domain admin".into(),
                username: r"CORP\admin.max".into(),
                password_updated: None,
            },
            password: Secret::from("old".to_owned()),
        },
        Change::AddConnection(connection("n_dc01", "dc01", Some("c_admin"), Some("g_infra"))),
        Change::AddConnection(connection("n_dc02", "dc02", Some("c_admin"), Some("g_infra"))),
        Change::AddConnection(connection("n_lab", "lab", None, None)),
    ] {
        library.apply(change).unwrap();
    }
    let mut bezel = Bezel::new();
    let outbox = bezel.outbox.clone();
    bezel.store = Some(Store::open(
        path,
        secrets.clone(),
        move |reply| drop(outbox.send(Message::Store(reply))),
        Arc::new(|| {}),
    ));
    wait_until(&mut bezel, |b| b.library.ready().is_some());
    Fixture {
        bezel,
        secrets,
        _alive: (dir, silent),
    }
}

fn wait_until(bezel: &mut Bezel, done: impl Fn(&Bezel) -> bool) {
    for _ in 0..500 {
        bezel.drain();
        if done(bezel) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out");
}

fn snapshot(bezel: &Bezel) -> &Snapshot {
    bezel.library.ready().unwrap()
}

fn rejected() -> Event {
    Event::SignInFailed {
        wrong_password: true,
        ended: Ended {
            message: "The username or password is incorrect".into(),
            details: String::new(),
            by_user: false,
        },
    }
}

fn connected() -> Event {
    Event::State(State::Connected {
        width: 2048,
        height: 1280,
    })
}

/// The Session's screen.
fn screen(bezel: &Bezel, id: SessionId) -> &Screen {
    &bezel.session(id).expect("the Session").screen
}

fn screen_mut(bezel: &mut Bezel, id: SessionId) -> &mut Screen {
    &mut bezel.session_mut(id).expect("the Session").screen
}

/// The engine behind a live Session.
fn engine(bezel: &Bezel, id: SessionId) -> u64 {
    match screen(bezel, id) {
        Screen::Live(live) => live.engine,
        _ => panic!("not live"),
    }
}

/// A Tab for `connection` signed in as if with `credential`, with engine `engine`.
fn live_tab(bezel: &mut Bezel, connection: &str, credential: Option<&str>, engine: u64) -> SessionId {
    let found = snapshot(bezel).connection(&connection.into()).unwrap().clone();
    let signing = credential.map(|id| Signing {
        credential: snapshot(bezel).credential(&id.into()).unwrap().clone(),
        adding: None,
        assign: false,
    });
    let id = bezel.next_id();
    let mut session = fixture_session(id, Destination::connection(&found), Phase::Connecting, None);
    if let Screen::Live(live) = &mut session.screen {
        live.engine = engine;
        live.signing = signing;
        live.keyboard = Keyboard::default();
    }
    bezel.sessions.push(session);
    bezel.activate(id);
    id
}

#[test]
fn a_rejected_password_is_saved_to_the_credential_and_the_connection_retries() {
    let Fixture {
        mut bezel,
        secrets,
        _alive,
    } = fixture();
    let id = live_tab(&mut bezel, "n_dc01", Some("c_admin"), 700);
    bezel.on_engine(700, rejected());
    let Screen::Retry(retry) = screen_mut(&mut bezel, id) else {
        panic!("expected the wrong-password prompt");
    };
    assert!(retry.rejected);
    assert_eq!(retry.credential.id, CredentialId::from("c_admin"));
    retry.password = TextEdit::new("new");
    bezel.retry(id);
    assert!(matches!(screen(&bezel, id), Screen::Retry(r) if r.saving()));
    wait_until(&mut bezel, |b| matches!(screen(b, id), Screen::Live(_)));
    assert_eq!(secrets.get(&"c_admin".into()), Some(Secret::from("new".to_owned())));
    assert_eq!(
        snapshot(&bezel).credential(&"c_admin".into()).unwrap().password_updated,
        Some(today())
    );
    let Screen::Live(live) = screen(&bezel, id) else {
        panic!()
    };
    assert_ne!(live.engine, 700, "the retry is a new engine");
    assert_eq!(
        live.signing.as_ref().map(|s| s.credential.id.clone()),
        Some("c_admin".into())
    );
}

#[test]
fn a_rejected_typed_password_brings_the_sign_in_sheet_back() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let id = live_tab(&mut bezel, "n_dc01", None, 700);
    bezel.on_engine(700, rejected());
    assert!(matches!(screen(&bezel, id), Screen::SignIn(s) if s.error.is_some()));
}

#[test]
fn a_connection_with_a_credential_reads_its_password_and_connects() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    bezel.open(&"n_dc01".into(), Opening::Tab, false);
    assert_eq!(bezel.dot(&"n_dc01".into()), Some(Dot::Connecting));
    let dc01 = bezel.active.unwrap();
    wait_until(&mut bezel, |b| matches!(screen(b, dc01), Screen::Live(_)));
    let lab = ConnectionId::from("n_lab");
    bezel.open(&lab, Opening::Tab, false);
    let tab = bezel.active.unwrap();
    assert_ne!(tab, dc01, "lab opens in a Tab of its own");
    assert!(matches!(screen(&bezel, tab), Screen::SignIn(_)));
    assert!(matches!(screen(&bezel, dc01), Screen::Live(_)), "dc01 keeps running");
}

#[test]
fn opening_an_open_connection_switches_to_it_and_option_opens_another() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let dc01 = ConnectionId::from("n_dc01");
    bezel.open(&dc01, Opening::Tab, false);
    let first = bezel.active.unwrap();
    bezel.open(&"n_lab".into(), Opening::Tab, false);
    bezel.open(&dc01, Opening::Tab, false);
    assert_eq!(bezel.active, Some(first));
    assert_eq!(bezel.sessions.len(), 2);
    bezel.open(&dc01, Opening::Tab, true);
    assert_eq!(bezel.sessions.len(), 3);
    assert_ne!(bezel.active, Some(first));
}

#[test]
fn a_missing_password_asks_for_one_and_stale_replies_are_ignored() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let to = Destination::connection(snapshot(&bezel).connection(&"n_dc01".into()).unwrap());
    let id = bezel.add(to, Opening::Tab);
    *screen_mut(&mut bezel, id) = Screen::Unlocking(Unlocking {
        credential: "c_admin".into(),
        request: 5,
    });
    bezel.on_password(4, Ok(Some(Secret::from("stale".to_owned()))));
    assert!(matches!(screen(&bezel, id), Screen::Unlocking(_)));
    bezel.on_password(5, Ok(None));
    assert!(matches!(screen(&bezel, id), Screen::Retry(r) if !r.rejected));
}

#[test]
fn each_sessions_events_and_replies_reach_that_session() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let dc01 = live_tab(&mut bezel, "n_dc01", Some("c_admin"), 701);
    let dc02 = live_tab(&mut bezel, "n_dc02", Some("c_admin"), 702);
    bezel.on_engine(701, connected());
    assert!(matches!(screen(&bezel, dc01), Screen::Live(l) if l.state != Phase::Connecting));
    assert!(matches!(screen(&bezel, dc02), Screen::Live(l) if l.state == Phase::Connecting));
    // A rejected password on dc01, while dc02 is the current Tab, prompts in dc01's Tab.
    bezel.on_engine(701, rejected());
    assert!(matches!(screen(&bezel, dc01), Screen::Retry(_)));
    assert!(matches!(screen(&bezel, dc02), Screen::Live(_)));
    assert_eq!(bezel.active, Some(dc02));
    // Its save, made from dc01's Tab, answers dc01 even after dc01 moved to a window.
    if let Screen::Retry(retry) = screen_mut(&mut bezel, dc01) {
        retry.password = TextEdit::new("new");
    }
    bezel.retry(dc01);
    bezel.move_to_window(dc01, None);
    wait_until(&mut bezel, |b| matches!(screen(b, dc01), Screen::Live(_)));
    assert!(!bezel.session(dc01).unwrap().place.is_tab());
    assert_eq!(engine(&bezel, dc02), 702, "dc02 never noticed");
    // Events from an engine that has been replaced go nowhere.
    bezel.on_engine(701, connected());
    assert!(matches!(screen(&bezel, dc01), Screen::Live(l) if l.state == Phase::Connecting));
}

#[test]
fn a_session_moves_to_a_window_and_back_without_reconnecting() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let dc01 = live_tab(&mut bezel, "n_dc01", Some("c_admin"), 701);
    let dc02 = live_tab(&mut bezel, "n_dc02", Some("c_admin"), 702);
    bezel.on_engine(702, connected());
    bezel.move_to_window(dc02, None);
    assert_eq!(bezel.active, Some(dc01), "the neighbour Tab becomes current");
    assert_eq!(bezel.tabs().count(), 1);
    assert!(matches!(screen(&bezel, dc02), Screen::Live(l) if l.engine == 702 && l.state != Phase::Connecting));
    // The Session window's turn: its Session's events still land.
    bezel.here.window = crate::windows::Window::Session(dc02);
    assert_eq!(bezel.current(), Some(dc02));
    bezel.on_engine(702, Event::State(State::Reconnecting));
    assert!(matches!(screen(&bezel, dc02), Screen::Live(l) if l.state == Phase::Reconnecting));
    bezel.move_to_main(dc02);
    bezel.here.window = crate::windows::Window::Main;
    assert_eq!(bezel.active, Some(dc02));
    assert_eq!(
        bezel.tabs().map(|s| s.id).collect::<Vec<_>>(),
        vec![dc01, dc02],
        "it comes back as the last Tab"
    );
    assert_eq!(engine(&bezel, dc02), 702, "the same connection");
}

#[test]
fn closing_a_tab_ends_its_session_and_picks_the_neighbour() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let a = live_tab(&mut bezel, "n_dc01", None, 701);
    let b = live_tab(&mut bezel, "n_dc02", None, 702);
    let c = live_tab(&mut bezel, "n_lab", None, 703);
    bezel.reveal(b);
    bezel.close(b);
    assert_eq!(bezel.active, Some(a));
    assert!(bezel.session(b).is_none());
    bezel.close(c);
    assert_eq!(bezel.active, Some(a), "closing another Tab keeps the current one");
    bezel.close(a);
    assert_eq!(bezel.active, None);
}

#[test]
fn tabs_reorder_and_step_with_the_keyboard() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let a = live_tab(&mut bezel, "n_dc01", None, 701);
    let b = live_tab(&mut bezel, "n_dc02", None, 702);
    let c = live_tab(&mut bezel, "n_lab", None, 703);
    bezel.reorder_tab(2, 0);
    assert_eq!(bezel.tabs().map(|s| s.id).collect::<Vec<_>>(), vec![c, a, b]);
    bezel.nth_tab(1);
    assert_eq!(bezel.active, Some(c));
    bezel.step_tab(-1);
    assert_eq!(bezel.active, Some(b), "stepping back from the first wraps to the last");
    bezel.nth_tab(9);
    assert_eq!(bezel.active, Some(b));
}

#[test]
fn quitting_with_sessions_open_asks_first() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let mut rt = Runtime::new(Bezel::new(), crate::view::view, (800.0, 600.0), Env::new(false, 1.0));
    let mut response = None;
    rt.update(|s, tx| response = Some(crate::windows::on_quit_request(s, tx)));
    assert_eq!(response, Some(rune::QuitResponse::Quit), "nothing open, nothing to ask");
    live_tab(&mut bezel, "n_dc01", None, 701);
    let mut rt = Runtime::new(bezel, crate::view::view, (800.0, 600.0), Env::new(false, 1.0));
    rt.update(|s, tx| response = Some(crate::windows::on_quit_request(s, tx)));
    assert_eq!(response, Some(rune::QuitResponse::Defer));
    assert!(rt.state.quitting);
    rt.frame(0.0);
    assert!(rt.find("quit-confirm").is_some());
}

/// Signs in to lab, which asks each time, with Save as credential ticked.
fn sign_in_saving(bezel: &mut Bezel, password: &str) -> (SessionId, Credential) {
    bezel.open(&"n_lab".into(), Opening::Tab, false);
    let id = bezel.active.unwrap();
    let Screen::SignIn(sign_in) = screen_mut(bezel, id) else {
        panic!()
    };
    sign_in.username = TextEdit::new("administrator");
    sign_in.password = TextEdit::new(password);
    bezel.toggle_save_credential(id);
    bezel.sign_in(id);
    assert!(
        matches!(screen(bezel, id), Screen::SignIn(s) if s.error.is_some()),
        "a name is needed"
    );
    let Screen::SignIn(sign_in) = screen_mut(bezel, id) else {
        panic!()
    };
    sign_in.password = TextEdit::new(password);
    sign_in.save_name = TextEdit::new("Local admin");
    bezel.sign_in(id);
    let Screen::Live(live) = screen(bezel, id) else {
        panic!()
    };
    (id, live.signing.as_ref().unwrap().credential.clone())
}

/// The Library file, for changes "another Bezel" makes.
fn library_file(alive: &(tempfile::TempDir, std::net::TcpListener)) -> std::path::PathBuf {
    alive.0.path().join("bezel.toml")
}

/// Makes every save fail by putting a folder where the Library writes its temporary file.
fn block_saves(alive: &(tempfile::TempDir, std::net::TcpListener)) {
    std::fs::create_dir(alive.0.path().join(format!("bezel.toml.{}.tmp", std::process::id()))).unwrap();
}

#[test]
fn a_credential_saved_at_sign_in_becomes_the_connections_once_signed_in() {
    let Fixture {
        mut bezel,
        secrets,
        _alive,
    } = fixture();
    let (id, saved) = sign_in_saving(&mut bezel, "typed");
    wait_until(&mut bezel, |b| snapshot(b).credentials.len() == 2);
    assert_eq!(
        (saved.name.as_str(), saved.username.as_str()),
        ("Local admin", "administrator")
    );
    assert_eq!(secrets.get(&saved.id), Some(Secret::from("typed".to_owned())));
    let lab = ConnectionId::from("n_lab");
    assert_eq!(snapshot(&bezel).connection(&lab).unwrap().credential, None);
    let engine = engine(&bezel, id);
    bezel.on_engine(engine, connected());
    wait_until(&mut bezel, |b| {
        snapshot(b).connection(&lab).unwrap().credential == Some(saved.id.clone())
    });
}

#[test]
fn a_rejected_password_saved_at_sign_in_is_corrected_in_that_credential() {
    let Fixture {
        mut bezel,
        secrets,
        _alive,
    } = fixture();
    let (id, saved) = sign_in_saving(&mut bezel, "typo");
    // The server answers before the Keychain has stored the new Credential's password.
    bezel.on_engine(engine(&bezel, id), rejected());
    let Screen::Retry(retry) = screen_mut(&mut bezel, id) else {
        panic!("expected the wrong-password prompt for the new Credential");
    };
    assert_eq!(retry.credential.id, saved.id);
    assert!(retry.saving(), "the prompt waits for the Credential to be saved");
    retry.password = TextEdit::new("right");
    bezel.retry(id);
    wait_until(&mut bezel, |b| matches!(screen(b, id), Screen::Retry(r) if !r.saving()));
    bezel.retry(id);
    wait_until(&mut bezel, |b| matches!(screen(b, id), Screen::Live(_)));
    assert_eq!(secrets.get(&saved.id), Some(Secret::from("right".to_owned())));
    bezel.on_engine(engine(&bezel, id), connected());
    wait_until(&mut bezel, |b| {
        snapshot(b).connection(&"n_lab".into()).unwrap().credential == Some(saved.id.clone())
    });
}

#[test]
fn a_credential_refused_after_its_sign_in_was_rejected_falls_back_to_typing() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    block_saves(&_alive);
    let (id, _) = sign_in_saving(&mut bezel, "typo");
    bezel.on_engine(engine(&bezel, id), rejected());
    assert!(matches!(screen(&bezel, id), Screen::Retry(_)));
    wait_until(&mut bezel, |b| matches!(screen(b, id), Screen::SignIn(_)));
    let Screen::SignIn(sign_in) = screen(&bezel, id) else {
        panic!()
    };
    assert_eq!(sign_in.username.text, "administrator");
    assert!(sign_in.error.as_deref().is_some_and(|e| e.contains("not saved")));
    assert_eq!(snapshot(&bezel).credentials.len(), 1);
}

#[test]
fn save_and_retry_for_a_credential_deleted_meanwhile_falls_back_to_the_sign_in_sheet() {
    let Fixture {
        mut bezel,
        secrets,
        _alive,
    } = fixture();
    let id = live_tab(&mut bezel, "n_dc01", Some("c_admin"), 700);
    bezel.on_engine(700, rejected());
    // Another Bezel deletes the Credential before this one has heard.
    let mut other = Store_::open(library_file(&_alive), secrets.clone()).unwrap();
    other.apply(Change::DeleteCredential("c_admin".into())).unwrap();
    if let Screen::Retry(retry) = screen_mut(&mut bezel, id) {
        retry.password = TextEdit::new("new");
    }
    bezel.retry(id);
    wait_until(&mut bezel, |b| matches!(screen(b, id), Screen::SignIn(_)));
    let Screen::SignIn(sign_in) = screen(&bezel, id) else {
        panic!()
    };
    assert_eq!(sign_in.error.as_deref(), Some(DELETED));
    assert_eq!(secrets.get(&"c_admin".into()), None);
}

#[test]
fn the_credentials_sheet_follows_the_library_and_keeps_passwords_to_their_account() {
    let Fixture {
        mut bezel,
        secrets,
        _alive,
    } = fixture();
    let mut other = Store_::open(library_file(&_alive), secrets.clone()).unwrap();
    let admin = snapshot(&bezel).credential(&"c_admin".into()).unwrap().clone();
    let rename_user = |from: &Credential, to: &str| Change::EditCredential {
        original: from.clone(),
        edited: Credential {
            username: to.into(),
            ..from.clone()
        },
        password: None,
        today: today(),
    };
    let sheet = |b: &Bezel| match &b.sheet {
        Some(Sheet::Credentials(sheet)) => (
            sheet.name.text.clone(),
            sheet.username.text.clone(),
            sheet.password.text.clone(),
            sheet.saving(),
            sheet.error.clone(),
        ),
        _ => panic!(),
    };
    bezel.open_credentials(None);
    other.apply(rename_user(&admin, r"CORP\bob")).unwrap();
    // Saving only a new name keeps Bob, and the form shows Bob from then on.
    if let Some(Sheet::Credentials(sheet)) = &mut bezel.sheet {
        sheet.name = TextEdit::new("Admin");
    }
    bezel.save_credential();
    wait_until(&mut bezel, |b| !sheet(b).3);
    assert_eq!(
        sheet(&bezel),
        ("Admin".into(), r"CORP\bob".into(), String::new(), false, None)
    );
    // A password typed for Bob is refused once the account became Carol, and cleared.
    let bob = snapshot(&bezel).credential(&"c_admin".into()).unwrap().clone();
    other.apply(rename_user(&bob, r"CORP\carol")).unwrap();
    if let Some(Sheet::Credentials(sheet)) = &mut bezel.sheet {
        sheet.password = TextEdit::new("bobs");
    }
    bezel.save_credential();
    wait_until(&mut bezel, |b| !sheet(b).3);
    let (_, username, password, _, error) = sheet(&bezel);
    assert_eq!((username.as_str(), password.as_str()), (r"CORP\carol", ""));
    assert!(error.is_some());
    assert_eq!(secrets.get(&"c_admin".into()), Some(Secret::from("old".to_owned())));
}

#[test]
fn the_connection_sheet_shows_a_host_changed_elsewhere_instead_of_overwriting_it() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let mut other = Store_::open(library_file(&_alive), MemorySecrets::default()).unwrap();
    let dc01 = snapshot(&bezel).connection(&"n_dc01".into()).unwrap().clone();
    bezel.edit_connection(&dc01.id);
    other
        .apply(Change::EditConnection {
            original: dc01.clone(),
            edited: Connection {
                host: "dc01-b.corp.lan".into(),
                ..dc01.clone()
            },
        })
        .unwrap();
    if let Some(Sheet::Connection(form)) = &mut bezel.sheet {
        form.host = TextEdit::new(format!("dc01-c.corp.lan:{}", dc01.port));
        form.name = TextEdit::new("primary");
    }
    bezel.save_connection();
    wait_until(
        &mut bezel,
        |b| matches!(&b.sheet, Some(Sheet::Connection(f)) if f.error.is_some()),
    );
    let Some(Sheet::Connection(form)) = &bezel.sheet else {
        panic!()
    };
    let shown = format!("dc01-b.corp.lan:{}", dc01.port);
    assert_eq!(
        (form.host.text.as_str(), form.name.text.as_str()),
        (shown.as_str(), "primary")
    );
    bezel.save_connection();
    wait_until(&mut bezel, |b| b.sheet.is_none());
    let saved = snapshot(&bezel).connection(&dc01.id).unwrap();
    assert_eq!(
        (saved.host.as_str(), saved.name.as_str()),
        ("dc01-b.corp.lan", "primary")
    );
}

#[test]
fn the_wrong_password_prompt_follows_its_credentials_account() {
    let Fixture {
        mut bezel,
        secrets,
        _alive,
    } = fixture();
    let mut other = Store_::open(library_file(&_alive), secrets.clone()).unwrap();
    let id = live_tab(&mut bezel, "n_dc01", Some("c_admin"), 700);
    bezel.on_engine(700, rejected());
    let Screen::Retry(retry) = screen_mut(&mut bezel, id) else {
        panic!()
    };
    retry.password = TextEdit::new("alices");
    let admin = retry.credential.clone();
    other
        .apply(Change::EditCredential {
            original: admin.clone(),
            edited: Credential {
                username: r"CORP\bob".into(),
                ..admin
            },
            password: None,
            today: today(),
        })
        .unwrap();
    bezel.retry(id);
    wait_until(&mut bezel, |b| matches!(screen(b, id), Screen::Retry(r) if !r.saving()));
    let Screen::Retry(retry) = screen(&bezel, id) else {
        panic!()
    };
    assert_eq!(retry.credential.username, r"CORP\bob");
    assert!(retry.password.text.is_empty() && retry.error.is_some());
    assert_eq!(secrets.get(&"c_admin".into()), Some(Secret::from("old".to_owned())));
}

#[test]
fn a_new_credential_refused_for_its_id_stays_new() {
    let Fixture {
        mut bezel,
        secrets,
        _alive,
    } = fixture();
    bezel.open_credentials(None);
    bezel.new_credential();
    if let Some(Sheet::Credentials(sheet)) = &mut bezel.sheet {
        sheet.selected = Selected::New("c_admin".into());
        sheet.name = TextEdit::new("Lab");
        sheet.username = TextEdit::new("labadmin");
        sheet.password = TextEdit::new("lab");
    }
    bezel.save_credential();
    wait_until(
        &mut bezel,
        |b| matches!(&b.sheet, Some(Sheet::Credentials(s)) if !s.saving()),
    );
    let Some(Sheet::Credentials(sheet)) = &bezel.sheet else {
        panic!()
    };
    assert!(matches!(&sheet.selected, Selected::New(id) if id.as_str() != "c_admin"));
    assert_eq!(sheet.name.text, "Lab");
    assert_eq!(secrets.get(&"c_admin".into()), Some(Secret::from("old".to_owned())));
}

#[test]
fn a_new_host_saved_as_a_connection_becomes_that_connections_session() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let address = Address::parse("fs09.corp.lan:3390").unwrap();
    let id = bezel.open_host(address.clone(), Opening::Tab);
    bezel.save_as_connection(id);
    let Some(Sheet::Connection(form)) = &bezel.sheet else {
        panic!("the New connection sheet")
    };
    assert_eq!(form.host.text, "fs09.corp.lan:3390");
    bezel.save_connection();
    wait_until(&mut bezel, |b| b.sheet.is_none());
    let session = bezel.session(id).unwrap();
    let saved = snapshot(&bezel).connections.iter().find(|c| c.name == "fs09").unwrap();
    assert_eq!(session.to.connection.as_ref(), Some(&saved.id));
    assert_eq!(session.to.name, "fs09");
}

/// Rune refuses to show a menu with an empty or repeated item ID.
fn assert_shows(menu: &rune::PopupMenu) {
    fn ids(menu: &rune::PopupMenu, out: &mut Vec<String>) {
        for entry in menu.entries() {
            match entry {
                rune::PopupEntry::Item(item) => out.push(item.id().to_owned()),
                rune::PopupEntry::Submenu { menu, .. } => ids(menu, out),
                _ => {}
            }
        }
    }
    let mut all = Vec::new();
    ids(menu, &mut all);
    let unique: std::collections::HashSet<&String> = all.iter().collect();
    assert!(
        all.iter().all(|id| !id.is_empty()) && unique.len() == all.len(),
        "{all:?}"
    );
}

fn press(rt: &mut Runtime<Bezel>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
}

/// Opens the picker and answers its menu with the item titled `title`.
fn pick(rt: &mut Runtime<Bezel>, key: &str, title: &str) {
    press(rt, &format!("{key}-trigger"));
    let menu = rt.popup_menu().expect("the picker asked for a menu");
    assert_shows(menu);
    let id = menu
        .entries()
        .iter()
        .find_map(|entry| match entry {
            rune::PopupEntry::Item(item) if item.title() == title => Some(item.id().to_owned()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {title}"));
    rt.resolve_popup(Some(&id)).unwrap();
    rt.frame(1.0 / 60.0);
}

#[test]
fn pickers_offer_menus_rune_shows_and_set_what_is_picked() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    bezel.edit_connection(&"n_dc01".into());
    let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
    rt.frame(0.0);
    pick(&mut rt, "connection-group", "File servers");
    pick(&mut rt, "connection-credential", "Ask each time");
    let Some(Sheet::Connection(form)) = &rt.state.sheet else {
        panic!()
    };
    assert_eq!((&form.group, &form.credential), (&Some("g_files".into()), &None));
    pick(&mut rt, "connection-group", "None");
    pick(&mut rt, "connection-credential", "Domain admin");
    let Some(Sheet::Connection(form)) = &rt.state.sheet else {
        panic!()
    };
    assert_eq!((&form.group, &form.credential), (&None, &Some("c_admin".into())));
}

#[test]
fn context_menus_offer_menus_rune_shows() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let dc01 = live_tab(&mut bezel, "n_dc01", None, 701);
    let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
    rt.frame(0.0);
    for key in ["n_dc01", "section-g_infra/section-header", &format!("t-{dc01}")] {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no {key}"));
        rt.secondary_down(x, y);
        rt.secondary_up(x, y);
        rt.frame(1.0 / 60.0);
        assert_shows(rt.popup_menu().expect("a context menu"));
        rt.resolve_popup(None).unwrap();
        rt.frame(1.0 / 60.0);
    }
    press(&mut rt, &format!("more-{dc01}"));
    assert_shows(rt.popup_menu().expect("the … menu"));
    rt.resolve_popup(None).unwrap();
    let (x, y) = rt.find("n_lab").unwrap();
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(1.0 / 60.0);
    let move_to_files = rt
        .popup_menu()
        .and_then(|menu| {
            menu.entries().iter().find_map(|entry| match entry {
                rune::PopupEntry::Submenu { menu, .. } => menu.entries().iter().find_map(|e| match e {
                    rune::PopupEntry::Item(item) if item.title() == "File servers" => Some(item.id().to_owned()),
                    _ => None,
                }),
                _ => None,
            })
        })
        .unwrap();
    rt.resolve_popup(Some(&move_to_files)).unwrap();
    rt.frame(1.0 / 60.0);
    settle(&mut rt, |b| {
        snapshot(b).connection(&"n_lab".into()).unwrap().group == Some("g_files".into())
    });
}

#[test]
fn the_connection_sheet_adds_and_keeps_its_input_when_a_save_is_refused() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    bezel.new_connection(Some("g_files".into()));
    if let Some(Sheet::Connection(form)) = &mut bezel.sheet {
        form.host = TextEdit::new("fs01.corp.lan:3390");
    }
    bezel.save_connection();
    wait_until(&mut bezel, |b| b.sheet.is_none());
    let fs01 = snapshot(&bezel).connections.iter().find(|c| c.name == "fs01").unwrap();
    assert_eq!((fs01.port, &fs01.group), (3390, &Some(GroupId::from("g_files"))));

    bezel.edit_connection(&"n_lab".into());
    bezel.apply(Change::DeleteConnection("n_lab".into()));
    bezel.save_connection();
    wait_until(
        &mut bezel,
        |b| matches!(&b.sheet, Some(Sheet::Connection(f)) if f.error.is_some()),
    );
}

#[test]
fn deleting_a_credential_makes_its_connections_ask() {
    let Fixture {
        mut bezel,
        secrets,
        _alive,
    } = fixture();
    bezel.open_credentials(None);
    bezel.ask_delete(Delete::Credential("c_admin".into()));
    bezel.delete();
    wait_until(&mut bezel, |b| snapshot(b).credentials.is_empty());
    assert_eq!(snapshot(&bezel).connection(&"n_dc01".into()).unwrap().credential, None);
    assert_eq!(secrets.get(&"c_admin".into()), None);
}

#[test]
fn the_credentials_sheet_changes_a_password_for_every_connection() {
    let Fixture {
        mut bezel,
        secrets,
        _alive,
    } = fixture();
    bezel.open_credentials(None);
    if let Some(Sheet::Credentials(sheet)) = &mut bezel.sheet {
        sheet.password = TextEdit::new("changed");
    }
    bezel.save_credential();
    wait_until(
        &mut bezel,
        |b| matches!(&b.sheet, Some(Sheet::Credentials(s)) if !s.saving()),
    );
    assert_eq!(secrets.get(&"c_admin".into()), Some(Secret::from("changed".to_owned())));
}

fn drag(rt: &mut Runtime<Bezel>, from: &str, to: &str) {
    let (x, y) = rt.find(from).unwrap();
    let (tx, ty) = rt.find(to).unwrap();
    rt.pointer_down(x, y);
    for step in 1..=10 {
        let t = step as f32 / 10.0;
        rt.pointer_move(x + (tx - x) * t, y + (ty - y) * t);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(tx, ty);
    rt.frame(1.0 / 60.0);
}

/// Waits on the Store as the window's poll would, then draws what it brought.
fn settle(rt: &mut Runtime<Bezel>, done: impl Fn(&Bezel) -> bool) {
    wait_until(&mut rt.state, done);
    rt.invalidate();
    rt.frame(1.0 / 60.0);
}

#[test]
fn dragging_moves_connections_between_groups_and_reorders_groups() {
    let Fixture { bezel, _alive, .. } = fixture();
    let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
    rt.frame(0.0);
    drag(&mut rt, "n_lab", "section-g_files");
    settle(&mut rt, |b| {
        snapshot(b).connection(&"n_lab".into()).unwrap().group == Some("g_files".into())
    });
    assert!(rt.state.sessions.is_empty(), "a drag is not a click");
    drag(
        &mut rt,
        "section-g_files/section-header",
        "section-g_infra/section-header",
    );
    settle(&mut rt, |b| snapshot(b).groups[0].id == GroupId::from("g_files"));
    let (x, y) = rt.find("n_dc01/actions/edit").unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
    assert!(
        matches!(&rt.state.sheet, Some(Sheet::Connection(f)) if f.editing.as_ref().is_some_and(|c| c.id.as_str() == "n_dc01"))
    );
    assert!(
        rt.state.sessions.is_empty(),
        "the hover action is not a click on the row"
    );
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Default::default(),
    });
    rt.frame(1.0 / 60.0);
    assert!(rt.state.sheet.is_none());
    // The sheet fades out before the sidebar takes clicks again.
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
    let (x, y) = rt.find("n_lab").unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
    let tab = rt.state.active.expect("lab opened in a Tab");
    assert!(matches!(screen(&rt.state, tab), Screen::SignIn(_)));
    assert_eq!(rt.state.session(tab).unwrap().to.name, "lab");
}

fn command(c: char) -> KeyEvent {
    KeyEvent {
        key: Key::Command(c),
        mods: Mods {
            cmd: true,
            logo: true,
            ..Mods::default()
        },
    }
}

/// A key no control took, as Rune hands it to `AppSpec::on_key` (a test `Runtime` has none).
fn shortcut(rt: &mut Runtime<Bezel>, event: KeyEvent) {
    let mut taken = false;
    rt.update(|s, tx| taken = s.on_key(&event, tx));
    assert!(taken, "{event:?} is a Bezel shortcut");
    rt.frame(1.0 / 60.0);
}

fn typed(rt: &mut Runtime<Bezel>, text: &str) {
    rt.key(KeyEvent {
        key: Key::Text(text.into()),
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
}

fn enter(rt: &mut Runtime<Bezel>, mods: Mods) {
    rt.key(KeyEvent { key: Key::Enter, mods });
    rt.frame(1.0 / 60.0);
}

#[test]
fn five_sessions_open_from_the_keyboard() {
    let Fixture { bezel, _alive, .. } = fixture();
    let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
    rt.frame(0.0);
    // dc01, dc02, another dc01 (⌥ on the open one), a New host, and lab in a Session window.
    let steps: [(&str, Mods); 5] = [
        ("dc01", Mods::default()),
        ("dc02", Mods::default()),
        (
            "dc01",
            Mods {
                alt: true,
                ..Mods::default()
            },
        ),
        ("127.0.0.1:1", Mods::default()),
        (
            "lab",
            Mods {
                cmd: true,
                logo: true,
                ..Mods::default()
            },
        ),
    ];
    for (query, mods) in steps {
        shortcut(&mut rt, command('k'));
        typed(&mut rt, query);
        assert_eq!(rt.state.here.palette.query.text, query, "⌘K focuses the palette");
        enter(&mut rt, mods);
        assert!(!rt.state.here.palette.open);
    }
    let names: Vec<&str> = rt.state.sessions.iter().map(|s| s.to.name.as_str()).collect();
    assert_eq!(names, vec!["dc01", "dc02", "dc01", "127.0.0.1", "lab"]);
    assert_eq!(rt.state.tabs().count(), 4);
    assert!(!rt.state.sessions[4].place.is_tab(), "⌘Return opens a Session window");
    // ⌘1 goes back to the first Tab.
    shortcut(&mut rt, command('1'));
    assert_eq!(rt.state.active, Some(rt.state.sessions[0].id));
}

#[test]
fn the_empty_panel_connects_to_a_new_host_from_its_field() {
    let Fixture { bezel, _alive, .. } = fixture();
    let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
    rt.frame(0.0);
    press(&mut rt, "palette-input");
    typed(&mut rt, "10.0.0.5");
    assert!(rt.find("palette-row-0").is_some(), "matches show under the field");
    enter(&mut rt, Mods::default());
    let tab = rt.state.active.expect("a Tab");
    assert_eq!(rt.state.session(tab).unwrap().to.name, "10.0.0.5");
    assert!(matches!(screen(&rt.state, tab), Screen::SignIn(_)));
}

#[test]
fn full_screen_hides_the_sidebar_and_tab_row_until_the_top_edge() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    live_tab(&mut bezel, "n_dc01", None, 701);
    let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
    rt.set_full_screen(true);
    rt.frame(0.0);
    assert!(rt.find("sidebar-column").is_none());
    assert!(rt.find("tabs").is_none());
    let (x, y) = rt.find("reveal-edge").expect("the top edge");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    assert!(rt.find("tabs").is_some(), "the tab row shows at the top edge");
    rt.pointer_move(x, 400.0);
    rt.frame(1.0 / 60.0);
    assert!(rt.find("tabs").is_none());
}

#[test]
fn a_session_window_draws_its_session_with_its_own_title_bar() {
    let Fixture { mut bezel, _alive, .. } = fixture();
    let dc01 = live_tab(&mut bezel, "n_dc01", None, 701);
    bezel.move_to_window(dc01, None);
    bezel.here.window = crate::windows::Window::Session(dc01);
    let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
    rt.frame(0.0);
    assert!(rt.find("session-titlebar").is_some());
    assert!(rt.find("sidebar-column").is_none() && rt.find("tabs").is_none());
    press(&mut rt, "move-to-main");
    assert!(rt.state.session(dc01).unwrap().place.is_tab());
    assert_eq!(rt.state.active, Some(dc01));
}
