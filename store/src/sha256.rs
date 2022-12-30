use std::{
    borrow::{Borrow, Cow},
    fmt::{self, Debug, LowerHex, UpperHex},
    fs::File,
    num::ParseIntError,
    ops::Deref,
    path::Path,
    str::FromStr,
};

use serde::{
    de::{Deserialize, Deserializer, Error},
    ser::Serializer,
    Serialize,
};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Hash {
    bytes: [u8; 32],
}

impl Deref for Sha256Hash {
    type Target = [u8; 32];
    fn deref(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl Borrow<[u8; 32]> for Sha256Hash {
    fn borrow(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl Borrow<[u8]> for Sha256Hash {
    fn borrow(&self) -> &[u8] {
        &self.bytes
    }
}

impl Debug for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        LowerHex::fmt(self, f)
    }
}

impl LowerHex for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            f.write_str("0x")?;
        }

        let v = u128::from_be_bytes(self.bytes[0..16].try_into().unwrap());
        write!(f, "{:032x}", v)?;
        let v = u128::from_be_bytes(self.bytes[16..32].try_into().unwrap());
        write!(f, "{:032x}", v)?;

        Ok(())
    }
}

impl UpperHex for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            f.write_str("0X")?;
        }

        let v = u128::from_be_bytes(self.bytes[0..16].try_into().unwrap());
        write!(f, "{:032X}", v)?;
        let v = u128::from_be_bytes(self.bytes[16..32].try_into().unwrap());
        write!(f, "{:032X}", v)?;

        Ok(())
    }
}

impl FromStr for Sha256Hash {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, ParseIntError> {
        let mut bytes = [0; 32];

        let l = s.len();
        if l > 32 {
            let upper = u128::from_str_radix(&s[..l - 32], 16)?;
            let lower = u128::from_str_radix(&s[l - 32..], 16)?;

            bytes[0..16].copy_from_slice(&upper.to_be_bytes());
            bytes[16..32].copy_from_slice(&lower.to_be_bytes());
        } else {
            let lower = u128::from_str_radix(s, 16)?;
            bytes[16..32].copy_from_slice(&lower.to_be_bytes());
        }

        Ok(Sha256Hash { bytes })
    }
}

impl Sha256Hash {
    pub fn new(data: impl AsRef<[u8]>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = hasher.finalize();
        let mut bytes = [0; 32];
        bytes.copy_from_slice(&hash);
        Sha256Hash { bytes }
    }

    pub fn file_hash(path: &Path) -> std::io::Result<Sha256Hash> {
        // Check for a duplicate.
        let mut hasher = Sha256::new();

        let mut file = File::open(&path)?;
        std::io::copy(&mut file, &mut hasher)?;

        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&hasher.finalize());
        Ok(Sha256Hash { bytes })
    }
}

impl Serialize for Sha256Hash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use std::io::Write;

        if serializer.is_human_readable() {
            let mut hex = [0u8; 64];
            write!(std::io::Cursor::new(&mut hex[..]), "{:x}", self).expect("Must fit");
            let hex = std::str::from_utf8(&hex).expect("Must be UTF-8");
            serializer.serialize_str(hex)
        } else {
            serializer.serialize_bytes(&self.bytes)
        }
    }
}

impl<'de> Deserialize<'de> for Sha256Hash {
    fn deserialize<D>(deserializer: D) -> Result<Sha256Hash, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let hex = Cow::<str>::deserialize(deserializer)?;
            hex.parse().map_err(Error::custom)
        } else {
            let bytes = serde_bytes::deserialize::<Cow<[u8]>, _>(deserializer)?;
            let bytes = TryFrom::try_from(bytes.as_ref()).map_err(Error::custom)?;
            Ok(Sha256Hash { bytes })
        }
    }
}
