# TOML file for data, Keychain for passwords

Connections, Groups, Credential names and usernames, Trusted certificates and settings live in one human-readable TOML file in Bezel's application support directory; passwords live only in the system credential store (the macOS Keychain), keyed by Credential. The file can be backed up, hand-edited or kept in dotfiles without ever containing a secret. SQLite was rejected because it is not hand-editable and Bezel's data is small.
