# Manual checklist

Run before calling a milestone done. The automated checks (`cargo test --workspace`, with the golden renders of every fixture scene) prove the views and the logic; this list proves Bezel against real Windows hosts, which only a person at a Mac can do.

## Setup

- Build with `scripts/bundle.sh` and open `target/release/Bezel.app`; the bundle declares Retina support and carries the icon. Check the Dock and Finder show the ember icon.
- Hosts: Windows Server 2019, Windows Server 2022, Windows Server 2025 and Windows 11, each joined to a test domain, each with a local `administrator`, and each with its self-signed RDP certificate. The Sign-in section turns NLA off on one of them.
- A Mac with a Retina display and a non-Retina external display.
- Start from an empty data folder: quit Bezel, move `~/Library/Application Support/Bezel` aside, and delete any `io.github.maxanderson95.bezel.*` items in Keychain Access.

## Every host

Run this section once per host and tick the host's column.

| Check | 2019 | 2022 | 2025 | 11 |
| --- | --- | --- | --- | --- |
| Domain account signs in through Kerberos (a domain controller found through DNS SRV or `/etc/krb5.conf`); the log shows no NTLM fallback. | [ ] | [ ] | [ ] | [ ] |
| Domain account signs in through NTLM when no domain controller can be found (for example `DOMAIN\user` from off the network's DNS). | [ ] | [ ] | [ ] | [ ] |
| Local account (`administrator`) signs in. | [ ] | [ ] | [ ] | [ ] |
| Self-signed certificate, first use: the dialog shows Issued to, Issued by "(self-signed)", Expires and the SHA-256 fingerprint; Trust and connect with Remember ticked connects, and the next Session connects without asking. | [ ] | [ ] | [ ] | [ ] |
| Text is crisp on the Retina display; the status line shows the remote size as twice the window's points with "Retina". | [ ] | [ ] | [ ] | [ ] |
| Dragging a window edge stretches the last frame, and about a quarter second after the drag stops the desktop matches the window without reconnecting. | [ ] | [ ] | [ ] | [ ] |
| Copy text on the Mac, ⌘V in Notepad: it pastes. Copy in Notepad with ⌘C, ⌘V in a Mac app: it pastes. | [ ] | [ ] | [ ] | [ ] |
| A sound in Windows (Settings, System, Sound, test) plays on the Mac. | [ ] | [ ] | [ ] | [ ] |
| Wi-Fi off for about 10 seconds: the desktop dims with "Reconnecting…" and an amber dot, then comes back on its own in the same Windows session. | [ ] | [ ] | [ ] | [ ] |

## Sign-in

- [ ] A wrong password typed in the sign-in sheet brings it back with "The username or password is incorrect".
- [ ] NLA turned off on the server: the Windows login screen shows, with the username filled in.
- [ ] A server allowing only legacy RDP security is refused with a plain message.
- [ ] Unreachable host, refused port and unknown host each show one plain sentence; Copy details copies the error chain and versions.

## Certificates

- [ ] First use: Cancel ends the Session; Trust and connect without Remember connects and asks again on the next Session.
- [ ] Trust with Remember ticked: `bezel.toml` holds the certificate, and Certificates in the sidebar lists it.
- [ ] Changed certificate (edit the fingerprint in `bezel.toml`): the danger dialog shows the old fingerprint struck through; Don't connect is the default; Trust new certificate… asks to confirm and then replaces the entry.
- [ ] Remove in Certificates forgets one: the next Session asks again.

## Display

- [ ] Non-Retina display: the remote size equals the window's points.
- [ ] Moving the window between the Retina and the non-Retina display resizes the desktop to the new scale.
- [ ] A server that refuses to resize in place (no Display Control channel) resizes through a reconnect ("Reconnecting…"). If none of the hosts refuses, write that down beside the item instead of ticking it.
- [ ] ⌃⌘F: the sidebar and tab row hide; the pointer at the top edge shows the tab row; ⌘K and tab switching still work.

## Input

- [ ] ⌘C and ⌘V copy and paste inside Windows; ⌘ alone opens the Start menu; typing on a non-US layout gives that layout's characters.
- [ ] Left, right and middle click, right-drag, mouse wheel (one notch scrolls three lines) and trackpad scrolling, vertical and horizontal.
- [ ] The remote pointer shape (text I-beam, resize arrows) shows over the desktop at the right size.
- [ ] Every shortcut in the spec's list works over the desktop and away from it, and none of them reaches Windows; the Session menu and Settings… in the app menu show them. ⌘H, ⌘M, ⌘Z and ⌥⌘H reach Windows over the desktop. ⌘Q with ⌘ released after Cancel opens no Start menu.
- [ ] Releasing the left Shift while the right Shift, held from before a Tab switch, is still down: a click is still a Shift-click.

## Connections, Groups and Credentials

- [ ] First run: the sidebar reads "No connections yet" and the panel shows Add connection.
- [ ] New connection with only a Host (`dc01.corp.lan:3390`) is named `dc01`, keeps the port, and appears under its Group, alphabetically; Connections without a Group sit under Ungrouped, last.
- [ ] Groups keep the order set by dragging a Group's title and by Move up and Move down; Connections drag between Groups and move with Move to group; Delete on a Group moves its Connections to Ungrouped.
- [ ] Edit, Duplicate and Delete (with its confirmation) from a Connection's context menu and its hover pencil.
- [ ] A Connection with a Credential signs in without asking; its row shows an amber dot while connecting and a green one once connected.
- [ ] "Ask each time" shows the sign-in sheet; picking a saved Credential there connects with it; typing one with Save as credential ticked asks for a name, connects, lists it under Credentials with the password in Keychain Access under `io.github.maxanderson95.bezel.<credential id>`, and the Connection signs in with it from then on; a wrong password typed that way asks for the Credential's password, not the sheet.
- [ ] Change a Credential's password in Credentials: every Connection using it signs in with the new one, and "Password updated today" shows.
- [ ] Change the account's password in Active Directory, then open a Connection using the Credential: "Sign-in failed for <Credential>" appears in the panel; Save and retry signs in and the other Connections using it sign in with the new password too.
- [ ] Delete a Credential: the confirmation names how many Connections use it, those Connections ask each time afterwards, and its Keychain item is gone.
- [ ] ⌃⌘S hides and shows the sidebar, with the desktop focused and without.
- [ ] Hand-edit `bezel.toml` (add a key Bezel doesn't know to a Connection, then break the file): the unknown key survives an edit in Bezel; the broken file shows its line and Open data folder and is not overwritten.

## Tabs, Session windows and the palette

- [ ] From launch, without the mouse: ⌘K, type a saved Connection, Return opens it in a Tab; repeat until five Sessions are open, using ⌥Return on an open one for a second Session, a typed host for a New host, and ⌘Return for one in a Session window. ⌘1 to ⌘9 and ⇧⌘[ / ⇧⌘] switch Tabs; ⌘W closes the current one.
- [ ] Choosing an open Connection in the sidebar or the palette switches to its Tab or Session window; ⌥-click in the sidebar opens another Session.
- [ ] Drag a Tab down off the tab row: it becomes a Session window under the pointer, still connected (no "Connecting…", the remote desktop keeps its state). Move to new window in the Tab's context menu and the tab row's actions does the same.
- [ ] In a Session window: the title bar shows the name, the status line appears on hover, Send Ctrl+Alt+Del reaches Windows, and Move to main window makes it the last Tab, still connected.
- [ ] Drag a Session window by its title bar onto the Main window's tab row: it becomes a Tab again, still connected.
- [ ] The sidebar's hover action and Open in new window open a Connection straight into a Session window.
- [ ] The empty panel's field connects to a typed host.
- [ ] The Main window's close button hides it and its Sessions keep running; the Dock icon brings it back. (A utility that quits apps when their last window closes, such as Vorssaint, will send Bezel a quit instead; with Sessions open Bezel then asks.)
- [ ] ⌘Q with Sessions open asks "Disconnect 3 Sessions and quit?" once; with none it quits at once.

## Clipboard and audio

- [ ] With two Sessions open, text copied in one pastes in the other.
- [ ] Copy text on the Mac and switch straight to a Session: ⌘V pastes the new text, not the one before.
- [ ] Copy text in Windows and paste it straight back in the same Session: it pastes what was just copied.
- [ ] Copy an image in Windows: the Mac's clipboard keeps what it had. Copy an image on the Mac: ⌘V in Windows pastes nothing.
- [ ] Mute in the … menu silences that Session only and shows as ticked; Unmute brings it back, and a muted Session stays muted after a reconnect.

## Dropped network

- [ ] After a 10 second Wi-Fi drop the clipboard and the mute are as they were.
- [ ] Wi-Fi off for a minute: about 30 seconds after the drop the Tab says the connection was lost, and Reconnect signs in again (asking for the password of a one-off sign-in).

## Settings and diagnostics

- [ ] Settings (⌘, the sidebar's gear, or Settings… in the palette): Appearance switches light and dark at once; "⌘ is the Windows key" makes ⌘C reach Windows as Windows+C in the open Sessions; Remember certificates by default ticks Remember this certificate; Open data folder shows `bezel.toml` in Finder. Quit and relaunch: every setting is as it was left.
- [ ] `~/Library/Logs/Bezel` has today's log with no password in it.
