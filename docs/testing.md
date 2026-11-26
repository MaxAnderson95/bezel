# Manual checklist

Run before calling a milestone done. Each item names the milestone that added it; M5 completes the list across Windows Server 2019, 2022 and 2025 and Windows 11.

## M1: one Session in a window

Use `scripts/bundle.sh` and open `Bezel.app` (the bundle declares Retina support).

- [ ] Domain account signs in through Kerberos (a domain controller found through DNS SRV or `/etc/krb5.conf`); the log shows no NTLM fallback.
- [ ] Domain account signs in through NTLM when no domain controller can be found (for example `DOMAIN\user` from off the network's DNS).
- [ ] Local account (`administrator`) signs in.
- [ ] A wrong password brings the sign-in sheet back with "The username or password is incorrect".
- [ ] NLA turned off on the server: the Windows login screen shows, with the username filled in.
- [ ] A server allowing only legacy RDP security is refused with a plain message.
- [ ] Self-signed certificate, first use: the dialog shows Issued to, Issued by "(self-signed)", Expires and the SHA-256 fingerprint; Cancel ends the Session; Trust and connect without Remember connects and asks again on the next Session.
- [ ] Trust with Remember ticked: the next Session connects without asking, and `bezel.toml` holds the certificate.
- [ ] Changed certificate (edit the fingerprint in `bezel.toml`): the danger dialog shows the old fingerprint struck through; Don't connect is the default; Trust new certificate… asks to confirm and then replaces the entry.
- [ ] Text is crisp on a Retina display; the status line shows the remote size as twice the window's points with "Retina".
- [ ] Non-Retina display: the remote size equals the window's points.
- [ ] Dragging a window edge stretches the last frame, and about a quarter second after the drag stops the desktop matches the window without reconnecting.
- [ ] A server without Display Control resizes through a reconnect ("Reconnecting…").
- [ ] Moving the window between a Retina and a non-Retina display resizes the desktop to the new scale.
- [ ] ⌘C and ⌘V copy and paste inside Windows; ⌘ alone opens the Start menu; ⌘W disconnects; typing on a non-US layout gives that layout's characters.
- [ ] Left, right and middle click, right-drag, mouse wheel (one notch scrolls three lines) and trackpad scrolling, vertical and horizontal.
- [ ] The remote pointer shape (text I-beam, resize arrows) shows over the desktop at the right size.
- [ ] Unreachable host, refused port and unknown host each show one plain sentence; Copy details copies the error chain and versions.
- [ ] `~/Library/Logs/Bezel` has today's log with no password in it.

## M2: data and sidebar

- [ ] First run: the sidebar reads "No connections yet" and the panel shows Add connection.
- [ ] New connection with only a Host (`dc01.corp.lan:3390`) is named `dc01`, keeps the port, and appears under its Group, alphabetically; Connections without a Group sit under Ungrouped, last.
- [ ] Groups keep the order set by dragging a Group's title and by Move up and Move down; Connections drag between Groups and move with Move to group; Delete on a Group moves its Connections to Ungrouped.
- [ ] Edit, Duplicate and Delete (with its confirmation) from a Connection's context menu and its hover pencil.
- [ ] A Connection with a Credential signs in without asking; its row shows an amber dot while connecting and a green one once connected.
- [ ] "Ask each time" shows the sign-in sheet; picking a saved Credential there connects with it; typing one with Save as credential ticked asks for a name, connects, lists it under Credentials with the password in Keychain Access under `io.github.maxanderson95.bezel.<credential id>`, and the Connection signs in with it from then on; a wrong password typed that way asks for the Credential's password, not the sheet.
- [ ] Change a Credential's password in Credentials: every Connection using it signs in with the new one, and "Password updated today" shows.
- [ ] Change the account's password in Active Directory, then open a Connection using the Credential: "Sign-in failed for <Credential>" appears in the panel; Save and retry signs in and the other Connections using it sign in with the new password too.
- [ ] Delete a Credential: the confirmation names how many Connections use it, those Connections ask each time afterwards, and its Keychain item is gone.
- [ ] Certificates lists the remembered certificates and Remove forgets one (the next Session asks again).
- [ ] ⌃⌘S hides and shows the sidebar, with the desktop focused and without.
- [ ] Hand-edit `bezel.toml` (add a key Bezel doesn't know to a Connection, then break the file): the unknown key survives an edit in Bezel; the broken file shows its line and Open data folder and is not overwritten.
