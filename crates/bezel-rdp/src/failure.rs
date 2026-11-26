//! Why a Session ended, in plain language, with the error chain kept for Copy details.

use std::error::Error;
use std::io;

use ironrdp_connector::sspi::credssp::NStatusCode;
use ironrdp_connector::{ConnectorError, ConnectorErrorKind};
use ironrdp_pdu::nego::FailureCode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    /// One sentence for the window.
    pub message: String,
    /// The underlying error chain and versions, for Copy details. Empty when there is none.
    pub details: String,
    /// The person ended it (disconnected or cancelled), so there is nothing to report.
    pub by_user: bool,
}

impl Ended {
    pub(crate) fn by_user() -> Self {
        Self {
            message: "Disconnected".into(),
            details: String::new(),
            by_user: true,
        }
    }

    pub(crate) fn new(message: impl Into<String>, error: &dyn Error) -> Self {
        let mut chain = error.to_string();
        let mut source = error.source();
        while let Some(e) = source {
            chain.push_str("\ncaused by: ");
            chain.push_str(&e.to_string());
            source = e.source();
        }
        Self {
            message: message.into(),
            details: details(&chain),
            by_user: false,
        }
    }

    pub(crate) fn plain(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            details: details(&message),
            message,
            by_user: false,
        }
    }
}

fn details(chain: &str) -> String {
    format!(
        "{chain}\n\nBezel {} on {}, IronRDP {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        crate::IRONRDP_REVISION
    )
}

pub(crate) enum Failure {
    SignIn { wrong_password: bool, ended: Ended },
    Ended(Ended),
}

/// Turns a connection failure into what the person needs to hear.
pub(crate) fn classify(error: &ConnectorError, host: &str, port: u16) -> Failure {
    let ended = |message: String| Failure::Ended(Ended::new(message, error));
    match error.kind() {
        ConnectorErrorKind::Negotiation(failure) => {
            return ended(match failure.code() {
                FailureCode::SSL_NOT_ALLOWED_BY_SERVER => {
                    format!("{host} offers only legacy RDP security without TLS, which Bezel refuses")
                }
                FailureCode::SSL_CERT_NOT_ON_SERVER => {
                    format!("{host} has no certificate to secure the connection with")
                }
                _ => format!("{host} and Bezel could not agree on how to secure the connection"),
            });
        }
        ConnectorErrorKind::Credssp(sspi) => {
            let wrong_password = matches!(
                sspi.nstatus,
                Some(NStatusCode::LOGON_FAILURE | NStatusCode::WRONG_PASSWORD)
            ) || sspi.error_type == ironrdp_connector::sspi::ErrorKind::LogonDenied;
            let message = match sspi.nstatus {
                _ if wrong_password => "The username or password is incorrect".to_owned(),
                Some(NStatusCode::ACCOUNT_LOCKED_OUT) => "The account is locked out".to_owned(),
                Some(NStatusCode::ACCOUNT_DISABLED) => "The account is disabled".to_owned(),
                Some(NStatusCode::PASSWORD_EXPIRED | NStatusCode::PASSWORD_MUST_CHANGE) => {
                    "The password has expired and must be changed in Windows".to_owned()
                }
                Some(
                    NStatusCode::ACCOUNT_RESTRICTION
                    | NStatusCode::INVALID_LOGON_HOURS
                    | NStatusCode::LOGON_TYPE_NOT_GRANTED,
                ) => {
                    format!("The account may not sign in to {host} now")
                }
                _ => format!("{host} did not accept the sign-in"),
            };
            return Failure::SignIn {
                wrong_password,
                ended: Ended::new(message, error),
            };
        }
        ConnectorErrorKind::AccessDenied => return ended(format!("{host} denied access")),
        _ => {}
    }
    match io_kind(error) {
        Some(io::ErrorKind::ConnectionRefused) => ended(format!("{host} refused the connection on port {port}")),
        Some(io::ErrorKind::TimedOut) => ended(format!("{host} did not answer on port {port}")),
        Some(io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable) => {
            ended(format!("{host} could not be reached"))
        }
        Some(io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted | io::ErrorKind::UnexpectedEof) => {
            ended(format!("{host} closed the connection"))
        }
        _ => ended(format!("Could not connect to {host}")),
    }
}

/// An ended Session after it was connected.
pub(crate) fn lost(error: &dyn Error, host: &str) -> Ended {
    Ended::new(format!("The connection to {host} was lost"), error)
}

/// An ended Session whose link dropped and did not come back while Bezel reconnected.
pub(crate) fn not_back(error: &dyn Error, host: &str) -> Ended {
    Ended::new(
        format!("The connection to {host} was lost and did not come back"),
        error,
    )
}

fn io_kind(error: &(dyn Error + 'static)) -> Option<io::ErrorKind> {
    let mut current = Some(error);
    while let Some(e) = current {
        if let Some(io) = e.downcast_ref::<io::Error>() {
            return Some(io.kind());
        }
        current = e.source();
    }
    None
}
