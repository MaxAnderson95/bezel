# Bezel v1 spec

Bezel is a small, fast RDP client for connecting from a Mac to Windows machines. It is personal software, MIT licensed, built from source, and macOS only in v1. Vocabulary follows [CONTEXT.md](../CONTEXT.md); recorded decisions live in [docs/adr](adr). Background research is in [research/landscape.md](research/landscape.md).

## Goals

- Connect by typing a host name and pressing Return, as many times as needed, each in its own Tab.
- Saved Connections in a compact sidebar, not tiles.
- Credentials as named, shared items whose password can be changed in the app.
- A server certificate can be remembered once and never asked about again unless it changes.
- The remote desktop always matches the window: sharp on Retina, resized live, no reconnect.
- The look of Anomaly's apps (OpenCode, Hex), in light and dark.

## Non-goals for v1

File or image clipboard, microphone, RD Gateway, `.rdp` import, multiple monitors per Session, shared Mac folders, printers, smart cards, Azure Virtual Desktop or Entra ID sign-in, Kerberos single sign-on from Mac tickets, KDC proxy, Windows or Linux releases, signed or notarized builds, auto-update, restoring Sessions at launch, per-Connection display or device settings.

## Experience

### Main window

One Main window, laid out like OpenCode Desktop: a deep-background sidebar on the left and a raised, rounded content panel on the right with a row of Tabs above it. The remote desktop sits inside the panel with a thin inset, which is the "bezel".

- **Sidebar.** A search field at the top ("Connect or search", shows ⌘K) opens the command palette. Below it, Groups in the order Max set, each listing its Connections alphabetically; Connections without a Group appear last under "Ungrouped". A Connection with an open Session shows a status dot (green connected, amber connecting or reconnecting). Hovering a row shows two actions: open in a Session window, and edit. The foot has Credentials, Certificates, Settings and a + button (New connection, New group). A toggle next to the traffic lights hides the sidebar (⌃⌘S).
- **Tabs.** One per Session: status dot, Connection name (or host for a New host), close button. The row ends with +, which opens the palette. Actions for the current Session sit at the right of the row: Send Ctrl+Alt+Del, Move to new window, Full screen, and a … menu (Mute, Disconnect, Sign out of Windows, Save as Connection for a New host, Copy details).
- **Status line.** A slim line at the top of the panel: host (mono), username, the live remote size with "Retina", and "Trusted" with a shield when the certificate chain or a Trusted certificate was accepted.
- **Empty panel.** With no Tab open, including first run, the panel shows the palette's search field inline, with "Type a host name to connect". On first run it also shows an "Add connection" button, and the sidebar reads "No connections yet".
- Closing the Main window hides it; its Sessions keep running. Clicking the Dock icon or pressing ⌘K shows it again. Launching Bezel opens the Main window with no Tabs.

### Command palette (⌘K)

A floating box over the current window with a scrim. Typing matches, in this order:

1. **Open**: Sessions already open, by name or host. Return switches to that Tab or Session window.
2. **Saved**: Connections, by name or host, showing their Credential name.
3. **New host**: "Connect to "<text>"", which connects to exactly what was typed (`host`, `host:port` or an IP).
4. **Manage**: Edit <Connection>…, Credentials…, Certificates…, Settings….

Return opens the selection in a new Tab (or switches to it if already open). ⌘Return opens it in a Session window. ⌥Return edits it. ⌥ with a Connection that is already open opens a second Session. Esc closes.

### Session windows

A Tab becomes a Session window by dragging it out of the tab row or choosing "Move to new window" from its context menu. A Connection can also open directly in a Session window (⌘Return, or the hover action). A Session window has no sidebar and no tab row: a thin title bar with the Connection name and, on the right, Send Ctrl+Alt+Del, Move to main window, and the … menu. The status line appears on hover. Dragging the Session window by its title bar onto the Main window's tab row, or choosing "Move to main window", turns it back into a Tab.

### Opening a Connection

Clicking a Connection, or choosing it in the palette, switches to its existing Tab or Session window if one exists; otherwise it opens a new Tab. ⌥-click opens another Session regardless.

### Closing and quitting

⌘W on a Tab or Session window disconnects without asking; the Windows session stays signed in on the server. Sign out of Windows is a separate … menu item. ⌘Q asks once, "Disconnect 3 Sessions and quit?", when Sessions are open.

### Full screen

⌃⌘F uses native macOS full screen. In the Main window the sidebar and tab row hide, and moving the pointer to the top edge reveals the tab row. ⌘K and tab switching still work.

### Sign-in

- A Connection with a Credential signs in with it silently.
- A Connection set to "Ask each time", or a New host, shows a sign-in sheet: pick an existing Credential or type a username and password, with "Save as Credential" (off by default; asks for a name when ticked). A Credential saved there for a Connection becomes that Connection's Credential once the sign-in succeeds.
- **Wrong password.** When the server rejects the password, the Tab shows "Sign-in failed for Domain admin" with a password field. Saving updates that Credential in the Keychain for every Connection that uses it, then retries. For a one-off typed password, the sheet simply reappears.
- Methods, tried automatically with nothing to configure: NLA (CredSSP) with Kerberos, finding the domain controller through DNS SRV records and `/etc/krb5.conf`, then NTLM. If the server has NLA turned off, Bezel connects over TLS and shows the Windows login screen, filling in the Credential when there is one. Servers offering only legacy RDP security without TLS are refused with a clear error.
- Usernames are entered as one field: `DOMAIN\user`, `user@domain` or `user` (a local account). Bezel splits them for the protocol.

### Certificates

When the server's certificate chains to an authority the Mac trusts and matches the host, Bezel connects silently. Otherwise:

- **No Trusted certificate for this host.** A dialog shows Issued to, Issued by (with "self-signed" when it is), Expires and the SHA-256 fingerprint, with a "Remember this certificate" checkbox (default off; Settings can flip the default) and buttons Cancel and Trust and connect (default). Trusting without Remember lasts for that Session, including its automatic reconnects. Trusting with Remember stores a Trusted certificate.
- **A Trusted certificate exists and matches.** Connect silently.
- **A Trusted certificate exists and does not match.** A danger-toned dialog: "<host> has a different certificate", the date it was trusted, the old fingerprint struck through, the new one, issuer and issue date. Don't connect is the default; Trust new certificate… is styled as dangerous and asks for confirmation, then replaces the Trusted certificate.

The Certificates window lists Trusted certificates (host, fingerprint, issuer, expiry, date remembered) and removes them.

### Credentials window

A sheet with a searchable list on the left (name, username in mono, count of Connections using it) and the selected Credential on the right: Name, Username and Password fields, "Password updated <n> days ago", and a "Used by" list of Connections. Changing the password and pressing Save password writes it to the Keychain. Delete… names how many Connections use the Credential; after confirming, those Connections switch to "Ask each time" and the Keychain item is removed.

### Connections and Groups

- New connection and Edit open a sheet with Name (defaults to the first label of the host, for example `dc01`), Host (host name or IP, optional `:port`, default 3389), Credential (a saved Credential or "Ask each time") and Group (optional).
- Right-clicking a Connection: Open, Open in new window, Edit…, Duplicate, Move to group, Delete… (confirms). Right-clicking a Group: Rename, Delete… (its Connections move to Ungrouped).
- Connections drag between Groups; Groups drag to reorder. Groups do not nest.

### Display

- The remote desktop always fits the content area. While a window edge is dragged, the last frame stretches to fit; about 250 ms after resizing pauses, Bezel asks Windows for the new size using the Display Control channel, falling back to a reconnect when the server does not support it.
- Retina is always on. Bezel requests the desktop in device pixels (points × backing scale, width rounded to even, clamped to 200 to 8192), sends a desktop scale factor of `round(scale × 100)` and the nearest device scale factor of 100, 140 or 180, and draws frames 1:1. Moving a window to a display with a different scale triggers the same resize.

### Input

- Keys are sent as physical scancodes; Windows applies its own keyboard layout.
- ⌘ key, default "Mac shortcuts": ⌘ plus a key that Bezel does not reserve is sent as Ctrl plus that key, so ⌘C and ⌘V copy and paste. ⌘ pressed and released alone is sent as the Windows key. The alternative setting, "⌘ is the Windows key", forwards ⌘ as the Windows key in all combinations.
- Bezel reserves: ⌘K, ⌘T (palette), ⌘W, ⌘Q, ⌘, (Settings), ⇧⌘[ and ⇧⌘], ⌘1 to ⌘9, ⌃⌘F, ⌃⌘S. Everything else reaches the Session. Menu key equivalents must not claim keys outside this list.
- Mouse buttons, wheel and trackpad scrolling (with correct wheel units) are forwarded; the remote pointer shape is drawn locally.

### Clipboard and audio

Text clipboard is shared both ways for every Session. Remote audio plays on the Mac; the … menu has Mute for the current Session.

### Reconnecting

When the link drops, the last frame dims with "Reconnecting…", the status dot turns amber, and Bezel retries for about 30 seconds using RDP's auto-reconnect cookie. After that the Tab shows the reason and a Reconnect button.

### Settings

Appearance (System, Light, Dark), ⌘ key behaviour, "Remember certificates by default", and Open data folder.

### Diagnostics

Bezel writes a rotating log to `~/Library/Logs/Bezel`. Passwords and other secrets never reach the log. Connection errors appear in plain language ("dc01.corp.lan refused the connection on port 3389") with Copy details, which copies the underlying error chain and the Bezel and IronRDP versions.

### Copy and look

`rune-theme-opencode` and `rune-kit` provide every colour, type style, radius, elevation and motion token; views never contain colour literals (the rule Hex follows). Sentence case everywhere except native macOS menus. One primary button per screen. Hover fills only on single-target rows, and hover changes are instant. The ember brand colour appears only in the app icon and mark.

## Data

One TOML file, `~/Library/Application Support/Bezel/bezel.toml`, written atomically (write to a temporary file, then rename). Passwords live only in the Keychain, one item per Credential with service `io.github.maxanderson95.bezel.<credential id>` and account `<credential id>` ([ADR 0002](adr/0002-toml-file-and-keychain-storage.md)); `rune::Credentials` keeps one item per service. IDs are random and stable, so renaming never orphans a Keychain item. Array order is display order for Groups.

```toml
version = 1

[settings]
appearance = "system"          # system | light | dark
command_key = "mac-shortcuts"  # mac-shortcuts | windows-key
remember_certificates = false

[[groups]]
id = "g_7k2m"
name = "Infrastructure"

[[credentials]]
id = "c_9x4p"
name = "Domain admin"
username = 'CORP\admin.max'
password_updated = 2026-08-23

[[connections]]
id = "n_3j8q"
name = "dc01"
host = "dc01.corp.lan"
port = 3389          # optional
credential = "c_9x4p" # optional; absent means ask each time
group = "g_7k2m"      # optional

[[trusted_certificates]]
host = "fs01.corp.lan"
port = 3389
sha256 = "7F3A91C20D5E88B41A6CE34792FD057BC8219E44A06DF1385B72CE0913B68AE5"
subject = "fs01.corp.lan"
issuer = "fs01.corp.lan"
not_after = 2027-03-14
remembered = 2026-10-03
```

A file that fails to parse is never overwritten: Bezel shows the parse error with the line, and offers Open data folder. Each save applies its change to the file as it is on disk at that moment and keeps any table that version of Bezel doesn't know, so hand edits and newer data survive.

## Architecture

A Cargo workspace with three crates. Rune and IronRDP are git dependencies pinned to one revision each ([ADR 0001](adr/0001-ironrdp-as-the-rdp-engine.md)); Rune's `[patch.crates-io]` block sits in the workspace root at the same revision, as Rune's README requires.

```
bezel/
├── Cargo.toml              workspace, pins, Rune patch block
├── crates/
│   ├── bezel-core/         no UI, no network: library, trust, keys, usernames
│   ├── bezel-rdp/          the Session engine on IronRDP
│   └── bezel/              the Rune app (binary)
└── docs/
```

### bezel-core

Pure Rust with no Rune or IronRDP dependency, so it builds and tests fast on any OS.

- **Library**: the persistent model and its invariants behind a small interface: `Library::open(path, secrets)`, `snapshot()` returning the current Connections, Groups, Credentials, Trusted certificates and Settings, and `apply(Change)` for every mutation (add or edit a Connection, reorder Groups, delete a Credential and reassign its Connections, remember or forget a certificate, change a setting). `apply` validates, updates, and saves atomically. Passwords are read with `password(credential)` and written by the Credential changes (adding one, a new password, deleting one), which keep the Keychain and the file together. The `Secrets` seam has two adapters: the system credential store through `rune::Credentials` in the app, and an in-memory store in tests.
- **Trust**: one function deciding what to do with a server certificate: given host, port, leaf certificate, whether the platform chain validated, the Trusted certificate for that host if any, and the Session's trusted-once fingerprints, it returns `Accept`, `AskFirstUse(details)` or `AskChanged { trusted, presented }`. Certificate parsing for the dialog's details lives here.
- **Keys**: maps a physical key event with modifiers and the ⌘ key setting to either a Bezel shortcut or the scancode sequence to send, including the "⌘ alone is the Windows key" rule.
- **Usernames**: splits `DOMAIN\user`, `user@domain` and `user` into the form the sign-in protocol needs.

### bezel-rdp

The deep module that owns everything IronRDP: threads, the tokio runtime, configuration, the Retina recipe, resize coalescing, reconnects, clipboard and audio backends. Its interface:

- `Session::connect(target, auth, events) -> SessionHandle`, where `target` is host, port and the content size in points with its backing scale, and `auth` is a username and password or "login screen".
- `SessionHandle` methods: `input(event)`, `resize(points, scale)` (callable on every layout change; the engine debounces and applies the Retina recipe), `set_muted(bool)`, `send_ctrl_alt_del()`, `sign_out()`, `disconnect()`.
- Events out, on one channel: `Frame` (the shared framebuffer changed, with dirty regions), `Pointer` (shape, position, hidden), `State` (connecting, signing in, connected with remote size, reconnecting, disconnected with reason), `Certificate(request)` (the engine pauses the handshake until the app replies through the request with Trust or Cancel), `SignInFailed { wrong_password }`.

The app never sees an IronRDP type. Frames land in a framebuffer the engine owns and the app reads; the first version uploads it with `El::pixels` and `Pixels::replace` buffer reuse, and a later version can move to a compositor slot that applies dirty rectangles, without changing this interface.

### bezel (the app)

Rune views and window management: Main window, Session windows, sidebar, tabs, palette, sheets, the Credentials and Certificates windows, Settings, menus, and the glue that turns engine events into view state. Keychain and file I/O never run on the UI thread. macOS specifics (data and log directories, the bundle) sit in one `platform` module; everything else is platform-neutral so the Linux CI build keeps it honest.

## Build, CI and testing

- `cargo build` builds everything; a `scripts/bundle.sh` wraps the binary into `Bezel.app` with an ad-hoc signature for local use. Installing is building from source.
- CI on GitHub Actions: macOS builds and tests the workspace; Linux builds and tests the workspace too. Actions are pinned by SHA.
- **Unit tests** in `bezel-core`: Library invariants and round-trips (including refusing to overwrite a bad file), every Trust outcome, key mapping, username splitting.
- **Snapshot tests**: named fixture scenes of each screen, rendered headless in light and dark and compared with goldens, as Hex does.
- **Manual checklist** in `docs/testing.md`, run before calling a milestone done: Windows Server 2019, 2022 and 2025 and Windows 11; domain account through Kerberos and through NTLM fallback; local account; NLA off; self-signed certificate, first use and changed; resize in place and the reconnect fallback; Retina and non-Retina displays; dropped network; clipboard; audio.

## Milestones

Each milestone is usable on its own and ends with the manual checklist items it touches.

### M1: one Session in a window

Workspace skeleton with pinned Rune and IronRDP, `LICENSE` (MIT), CI on macOS and Linux, and the `design/mockups` folder removed. A single window with a host field: connect, sign in through the sign-in sheet (NLA with Kerberos then NTLM, login-screen fallback, legacy security refused), sharp Retina, live resize with the 250 ms debounce, keyboard as physical scancodes with the default ⌘ mapping, mouse and scrolling, remote pointer, the certificate dialogs with Remember (the Library holds Trusted certificates only so far), the log file and Copy details. `bezel-rdp`'s interface is the one above from the start.

**Done when**: Max connects to a self-signed Windows Server from the window, sees crisp text on a Retina display, resizes the window and the desktop follows without reconnecting, and is asked about the certificate once after ticking Remember.

### M2: data and sidebar

The full Library (Connections, Groups, Credentials, Settings), Keychain storage, the Main window sidebar with Groups, adding and editing Connections and Groups, the Credentials window, the wrong-password prompt, "Ask each time" with Save as Credential, and the Certificates window.

**Done when**: Max can save his servers in Groups, change a Credential's password in one place and have every Connection use it, and recover from a changed AD password from the failed sign-in itself.

### M3: tabs, Session windows and the palette

Tabs in the Main window, opening a Connection switches to its existing Session, Session windows by drag or menu and back again, the ⌘K palette with all four sections, the empty-panel connect box, full screen with the top-edge reveal, hiding the Main window on close, and quitting with open Sessions.

**Done when**: Max opens five Sessions from the keyboard without touching the mouse, tears one out to its own window, and puts it back.

### M4: clipboard, audio, reconnect and settings

Text clipboard both ways, audio with Mute, auto-reconnect for about 30 seconds then the Reconnect button, the full Bezel shortcut list, the ⌘ key setting, and the Settings window.

**Done when**: copy and paste work both ways with ⌘C and ⌘V, a Wi-Fi blip recovers on its own, and Settings changes persist.

### M5: snapshot tests and polish

Fixture scenes and goldens for every screen in light and dark, `docs/testing.md` with the full manual checklist, an app icon, and visual polish against the Anomaly look.

**Done when**: the snapshot suite runs in CI and the manual checklist passes on all listed hosts.

## Risks

- Rune is pre-release and changes daily; IronRDP's `master` is far ahead of its crates.io release. Both are pinned and bumped deliberately, and Bezel keeps its use of each behind `bezel` views and `bezel-rdp` respectively.
- Full-screen 5K Retina frames are about 59 MB each; whole-frame uploads may be too slow there. Measure in M1 before moving to dirty-rectangle compositing.
- Without an H.264 decoder, sessions use RemoteFX and bitmap codecs. A VideoToolbox decoder is future work if bandwidth or CPU demands it.
- Dragging a Tab out of a window and a Session window back onto the tab row needs cross-window drag tracking; Rune support for it is unverified. The menu items work regardless, so drag can slip without blocking M3.
- Interop with real Windows hosts (resize in place, Kerberos, NLA off) is the least proven part of the spike; the manual checklist starts in M1.
