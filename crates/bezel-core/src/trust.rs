//! What to do with a server's certificate, and the details its dialogs show.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::Date;
use x509_cert::der::Decode;

/// A certificate's SHA-256 over its DER encoding.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub fn of(der: &[u8]) -> Self {
        Self(Sha256::digest(der).into())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02X}"))
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("a SHA-256 fingerprint is 64 hexadecimal digits")]
pub struct FingerprintError;

impl FromStr for Fingerprint {
    type Err = FingerprintError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let digits = s.as_bytes();
        if digits.len() != 64 {
            return Err(FingerprintError);
        }
        let mut bytes = [0; 32];
        for (byte, pair) in bytes.iter_mut().zip(digits.as_chunks::<2>().0) {
            let pair = std::str::from_utf8(pair).map_err(|_| FingerprintError)?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| FingerprintError)?;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// A server certificate as its dialogs describe it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    pub sha256: Fingerprint,
    /// The subject's common name, or the whole subject when it has none.
    pub subject: String,
    pub issuer: String,
    pub self_signed: bool,
    pub not_before: Date,
    pub not_after: Date,
}

#[derive(Debug, thiserror::Error)]
#[error("the server's certificate could not be read: {0}")]
pub struct CertificateError(String);

impl Certificate {
    pub fn parse(der: &[u8]) -> Result<Self, CertificateError> {
        let error = |e: x509_cert::der::Error| CertificateError(e.to_string());
        let cert = x509_cert::Certificate::from_der(der).map_err(error)?;
        let tbs = cert.tbs_certificate();
        let name = |name: &x509_cert::name::Name| -> Result<String, CertificateError> {
            Ok(match name.common_name().map_err(error)? {
                Some(cn) => cn.value().into_owned(),
                None => name.to_string(),
            })
        };
        let date = |time: x509_cert::time::Time| {
            let time = time.to_date_time();
            time::Month::try_from(time.month())
                .and_then(|month| Date::from_calendar_date(time.year().into(), month, time.day()))
                .map_err(|e| CertificateError(e.to_string()))
        };
        Ok(Self {
            sha256: Fingerprint::of(der),
            subject: name(tbs.subject())?,
            issuer: name(tbs.issuer())?,
            self_signed: tbs.subject() == tbs.issuer(),
            not_before: date(tbs.validity().not_before)?,
            not_after: date(tbs.validity().not_after)?,
        })
    }
}

/// A certificate Bezel was told to remember for a host and port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedCertificate {
    pub host: String,
    pub port: u16,
    pub sha256: Fingerprint,
    pub subject: String,
    pub issuer: String,
    #[serde(with = "crate::library::toml_date")]
    pub not_after: Date,
    #[serde(with = "crate::library::toml_date")]
    pub remembered: Date,
}

impl TrustedCertificate {
    pub fn new(host: &str, port: u16, certificate: &Certificate, remembered: Date) -> Self {
        Self {
            host: host.to_ascii_lowercase(),
            port,
            sha256: certificate.sha256,
            subject: certificate.subject.clone(),
            issuer: certificate.issuer.clone(),
            not_after: certificate.not_after,
            remembered,
        }
    }

    pub fn is_for(&self, host: &str, port: u16) -> bool {
        self.port == port && self.host.eq_ignore_ascii_case(host)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Ask(Question),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    /// No Trusted certificate exists for the host.
    FirstUse(Certificate),
    /// The host presented a different certificate from the one trusted for it.
    Changed {
        trusted: TrustedCertificate,
        presented: Certificate,
    },
}

/// `trusted` is the Trusted certificate for the host the Session connects to, and
/// `trusted_once` the fingerprints accepted without Remember during this Session.
pub fn decide(
    presented: &Certificate,
    chain_valid: bool,
    trusted: Option<&TrustedCertificate>,
    trusted_once: &HashSet<Fingerprint>,
) -> Decision {
    if chain_valid || trusted_once.contains(&presented.sha256) {
        return Decision::Accept;
    }
    match trusted {
        Some(trusted) if trusted.sha256 == presented.sha256 => Decision::Accept,
        Some(trusted) => Decision::Ask(Question::Changed {
            trusted: trusted.clone(),
            presented: presented.clone(),
        }),
        None => Decision::Ask(Question::FirstUse(presented.clone())),
    }
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::*;

    // A self-signed certificate for fs01.corp.lan, valid from 2026-11-25 until 2027-03-14.
    const SELF_SIGNED: &[u8] = include_bytes!("../testdata/fs01.der");

    fn presented() -> Certificate {
        Certificate::parse(SELF_SIGNED).unwrap()
    }

    fn trusted(sha256: Fingerprint) -> TrustedCertificate {
        TrustedCertificate {
            sha256,
            ..TrustedCertificate::new("fs01.corp.lan", 3389, &presented(), date!(2026 - 10 - 03))
        }
    }

    #[test]
    fn parses_the_dialog_details() {
        let cert = presented();
        assert_eq!(cert.subject, "fs01.corp.lan");
        assert_eq!(cert.issuer, "fs01.corp.lan");
        assert!(cert.self_signed);
        assert_eq!(cert.not_before, date!(2026 - 11 - 25));
        assert_eq!(cert.not_after, date!(2027 - 03 - 14));
        assert_eq!(cert.sha256, Fingerprint::of(SELF_SIGNED));
    }

    #[test]
    fn refuses_bytes_that_are_not_a_certificate() {
        assert!(Certificate::parse(b"not a certificate").is_err());
    }

    #[test]
    fn a_chain_the_mac_trusts_is_accepted_silently() {
        let other = trusted(Fingerprint([7; 32]));
        assert_eq!(decide(&presented(), true, None, &HashSet::new()), Decision::Accept);
        assert_eq!(
            decide(&presented(), true, Some(&other), &HashSet::new()),
            Decision::Accept
        );
    }

    #[test]
    fn an_unknown_host_asks_on_first_use() {
        assert_eq!(
            decide(&presented(), false, None, &HashSet::new()),
            Decision::Ask(Question::FirstUse(presented()))
        );
    }

    #[test]
    fn a_matching_trusted_certificate_is_accepted() {
        let trusted = trusted(presented().sha256);
        assert_eq!(
            decide(&presented(), false, Some(&trusted), &HashSet::new()),
            Decision::Accept
        );
    }

    #[test]
    fn a_different_certificate_asks_about_the_change() {
        let old = trusted(Fingerprint([7; 32]));
        assert_eq!(
            decide(&presented(), false, Some(&old), &HashSet::new()),
            Decision::Ask(Question::Changed {
                trusted: old.clone(),
                presented: presented()
            })
        );
    }

    #[test]
    fn trust_without_remember_lasts_for_the_session() {
        let once = HashSet::from([presented().sha256]);
        assert_eq!(decide(&presented(), false, None, &once), Decision::Accept);
        let old = trusted(Fingerprint([7; 32]));
        assert_eq!(decide(&presented(), false, Some(&old), &once), Decision::Accept);
    }

    #[test]
    fn hosts_match_without_case() {
        let trusted = trusted(presented().sha256);
        assert!(trusted.is_for("FS01.corp.lan", 3389));
        assert!(!trusted.is_for("fs01.corp.lan", 3390));
    }

    #[test]
    fn fingerprints_read_back_what_they_write() {
        let fp = presented().sha256;
        assert_eq!(fp.to_string().len(), 64);
        assert_eq!(fp.to_string().parse::<Fingerprint>().unwrap(), fp);
        assert_eq!(fp.to_string().to_lowercase().parse::<Fingerprint>().unwrap(), fp);
        assert!("7F3A".parse::<Fingerprint>().is_err());
        assert!("Z".repeat(64).parse::<Fingerprint>().is_err());
    }
}
