# Bezel

Bezel is a small, fast RDP client for connecting from a Mac to Windows machines, with saved connections, reusable credentials and remembered server certificates.

## Language

### Connecting

**Connection**:
A saved remote machine: its name, host and the settings used to reach it.
_Avoid_: Bookmark, favorite, profile, server, device

**New host**:
A machine reached by typing its name or address into the search box, without saving it as a Connection.
_Avoid_: Quick connect, ad-hoc connection

**Session**:
One live RDP link to a remote machine, from connecting until it disconnects.
_Avoid_: Connection (when meaning the live link)

**Main window**:
The one Bezel window with the sidebar of Connections and a row of Tabs.
_Avoid_: Library, launcher, home window

**Tab**:
The place a Session lives inside the Main window; the Main window holds any number of Tabs.
_Avoid_: Pane

**Session window**:
A window, with no sidebar, that holds exactly one Session after its Tab is torn out of the Main window or it is opened there directly.
_Avoid_: Detached tab, standalone window

**Group**:
A named set of Connections shown together in the sidebar.
_Avoid_: Folder, collection

### Identity and trust

**Credential**:
A named username and password that any number of Connections can use.
_Avoid_: Login, account, identity

**Trusted certificate**:
A server certificate Bezel has been told to remember for a host, so connecting to that host no longer asks about it.
_Avoid_: Pinned certificate, exception
