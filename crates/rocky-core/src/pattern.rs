use std::ops::Deref;

use regex::Regex;
use serde::de::{self, Deserialize, Deserializer};

/// A regex compiled while the project is parsed, so an invalid one fails at load.
#[derive(Clone, Debug)]
pub struct Pattern(Regex);

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pattern = String::deserialize(deserializer)?;
        Regex::new(&pattern)
            .map(Pattern)
            .map_err(|err| de::Error::custom(format!("invalid regex `{pattern}`: {err}")))
    }
}

impl Deref for Pattern {
    type Target = Regex;

    fn deref(&self) -> &Regex {
        &self.0
    }
}
