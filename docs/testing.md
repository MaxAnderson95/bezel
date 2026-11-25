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
