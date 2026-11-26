//! Usernames are typed as one field and split here into what the sign-in protocol needs.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Username {
    /// `DOMAIN\user`
    DownLevel { domain: String, user: String },
    /// `user@domain`
    Principal { user: String, suffix: String },
    /// `user`, an account local to the remote machine.
    Local { user: String },
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UsernameError {
    #[error("Enter a username")]
    Empty,
    #[error("Enter a name on both sides of the \"{0}\"")]
    MissingPart(char),
}

impl Username {
    /// A `\` wins over `@`, as Windows reads `AzureAD\user@example.com`.
    pub fn parse(input: &str) -> Result<Self, UsernameError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(UsernameError::Empty);
        }
        if let Some((domain, user)) = input.split_once('\\') {
            return match (domain, user) {
                ("", _) | (_, "") => Err(UsernameError::MissingPart('\\')),
                _ => Ok(Self::DownLevel {
                    domain: domain.into(),
                    user: user.into(),
                }),
            };
        }
        if let Some((user, suffix)) = input.rsplit_once('@') {
            return match (user, suffix) {
                ("", _) | (_, "") => Err(UsernameError::MissingPart('@')),
                _ => Ok(Self::Principal {
                    user: user.into(),
                    suffix: suffix.into(),
                }),
            };
        }
        Ok(Self::Local { user: input.into() })
    }

    /// The protocol's username and domain fields. A principal name travels whole in the
    /// username with no domain, which CredSSP reads as a UPN; splitting it would send the UPN
    /// suffix as a NetBIOS domain, which fails wherever the two differ.
    pub fn protocol_parts(&self) -> (String, Option<String>) {
        match self {
            Self::DownLevel { domain, user } => (user.clone(), Some(domain.clone())),
            Self::Principal { .. } => (self.to_string(), None),
            Self::Local { user } => (user.clone(), None),
        }
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DownLevel { domain, user } => write!(f, "{domain}\\{user}"),
            Self::Principal { user, suffix } => write!(f, "{user}@{suffix}"),
            Self::Local { user } => f.write_str(user),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_the_three_forms() {
        assert_eq!(
            Username::parse(r"CORP\admin.max").unwrap().protocol_parts(),
            ("admin.max".into(), Some("CORP".into()))
        );
        assert_eq!(
            Username::parse("max@corp.lan").unwrap().protocol_parts(),
            ("max@corp.lan".into(), None)
        );
        assert_eq!(
            Username::parse("administrator").unwrap().protocol_parts(),
            ("administrator".into(), None)
        );
    }

    #[test]
    fn a_backslash_wins_over_an_at_sign() {
        assert_eq!(
            Username::parse(r"AzureAD\max@example.com").unwrap(),
            Username::DownLevel {
                domain: "AzureAD".into(),
                user: "max@example.com".into()
            }
        );
    }

    #[test]
    fn the_last_at_sign_separates_the_suffix() {
        assert_eq!(
            Username::parse("first@last@corp.lan").unwrap(),
            Username::Principal {
                user: "first@last".into(),
                suffix: "corp.lan".into()
            }
        );
    }

    #[test]
    fn surrounding_space_is_ignored() {
        assert_eq!(
            Username::parse("  max  ").unwrap(),
            Username::Local { user: "max".into() }
        );
    }

    #[test]
    fn rejects_empty_and_half_names() {
        assert_eq!(Username::parse("   "), Err(UsernameError::Empty));
        assert_eq!(Username::parse(r"CORP\"), Err(UsernameError::MissingPart('\\')));
        assert_eq!(Username::parse(r"\max"), Err(UsernameError::MissingPart('\\')));
        assert_eq!(Username::parse("max@"), Err(UsernameError::MissingPart('@')));
        assert_eq!(Username::parse("@corp.lan"), Err(UsernameError::MissingPart('@')));
    }

    #[test]
    fn displays_as_typed() {
        for typed in [r"CORP\max", "max@corp.lan", "max"] {
            assert_eq!(Username::parse(typed).unwrap().to_string(), typed);
        }
    }
}
