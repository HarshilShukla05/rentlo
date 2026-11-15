//! Identifiers. Each is its own type, so a profile id can never be passed
//! where a user id is expected.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(id: Uuid) -> Self {
                Self(id)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

uuid_id!(UserId);
uuid_id!(ProfileId);
uuid_id!(MatchId);

/// The short code people see and type for a profile, like `R7K2Q9`.
///
/// It starts with `R` and uses an alphabet without the look-alikes
/// `0 O 1 I L`, so it survives being read aloud or retyped.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PublicCode(String);

const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const CODE_BODY_LEN: usize = 5;

impl PublicCode {
    /// Builds a code from random bits. The caller supplies the randomness.
    pub fn from_entropy(mut bits: u64) -> Self {
        let base = CODE_ALPHABET.len() as u64;
        let mut code = String::with_capacity(1 + CODE_BODY_LEN);
        code.push('R');
        for _ in 0..CODE_BODY_LEN {
            code.push(CODE_ALPHABET[(bits % base) as usize] as char);
            bits /= base;
        }
        Self(code)
    }

    /// Accepts what a person typed: any case, surrounding spaces.
    pub fn parse(input: &str) -> Option<Self> {
        let code = input.trim().to_ascii_uppercase();
        let mut bytes = code.bytes();
        let valid =
            code.len() == 1 + CODE_BODY_LEN && bytes.next() == Some(b'R') && bytes.all(|b| CODE_ALPHABET.contains(&b));
        valid.then_some(Self(code))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PublicCode {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value).ok_or_else(|| format!("not a profile code: {value:?}"))
    }
}

impl From<PublicCode> for String {
    fn from(code: PublicCode) -> Self {
        code.0
    }
}

impl fmt::Display for PublicCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_code_parses_back() {
        for bits in [0, 1, 12345, u64::MAX, 0xDEAD_BEEF] {
            let code = PublicCode::from_entropy(bits);
            assert_eq!(PublicCode::parse(code.as_str()), Some(code));
        }
    }

    #[test]
    fn parsing_forgives_case_and_spaces_only() {
        assert_eq!(PublicCode::parse(" r7k2q9 ").map(|c| c.to_string()), Some("R7K2Q9".into()));
        for bad in ["", "R7K2Q", "R7K2Q99", "X7K2Q9", "R7K2O9", "R7K2I9", "R7K 2Q"] {
            assert_eq!(PublicCode::parse(bad), None, "{bad:?}");
        }
    }
}
