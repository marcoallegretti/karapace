//! Newtype wrappers for string identifiers, providing compile-time type safety.
//!
//! All newtypes serialize/deserialize as plain strings for backward compatibility.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;

macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Create a new instance from a string.
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            /// Return the inner string as a slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume self and return the inner `String`.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<$name> for String {
            fn eq(&self, other: &$name) -> bool {
                *self == other.0
            }
        }

        impl AsRef<std::path::Path> for $name {
            fn as_ref(&self) -> &std::path::Path {
                std::path::Path::new(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    };
}

string_newtype!(
    /// Full 64-character hex environment identifier, derived from locked manifest content.
    EnvId
);

string_newtype!(
    /// Truncated 12-character prefix of an [`EnvId`], used for display.
    ShortId
);

string_newtype!(
    /// Blake3 hash of a content-addressable object in the store.
    ObjectHash
);

string_newtype!(
    /// Blake3 hash identifying a layer manifest.
    LayerHash
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_id_display_and_as_ref() {
        let id = EnvId::new("abc123");
        assert_eq!(id.to_string(), "abc123");
        assert_eq!(id.as_str(), "abc123");
        assert_eq!(AsRef::<str>::as_ref(&id), "abc123");
    }

    #[test]
    fn env_id_serde_roundtrip() {
        let id = EnvId::new("deadbeef");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"deadbeef\"");
        let back: EnvId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn short_id_from_str() {
        let sid = ShortId::from("abc123def456");
        assert_eq!(sid.as_str(), "abc123def456");
    }

    #[test]
    fn object_hash_into_inner() {
        let h = ObjectHash::new("hash_value".to_owned());
        assert_eq!(h.into_inner(), "hash_value");
    }

    #[test]
    fn layer_hash_equality() {
        let a = LayerHash::new("same");
        let b = LayerHash::new("same");
        let c = LayerHash::new("diff");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn env_id_from_string() {
        let s = String::from("test_id");
        let id: EnvId = s.into();
        assert_eq!(id.as_str(), "test_id");
    }
}
