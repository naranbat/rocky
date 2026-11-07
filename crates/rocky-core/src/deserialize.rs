use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};

/// Deserializes a present optional field as `Some`, so an explicit `null` is passed to `T`
/// instead of being treated like a missing field.
pub(crate) fn some<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Deserializes a present optional field while rejecting YAML `null`. Inspecting the YAML
/// option before its value prevents `serde_yaml` from coercing `null` to the string `"null"`.
pub(crate) fn some_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    non_null(deserializer).map(Some)
}

pub(crate) fn non_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct NonNull<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for NonNull<T> {
        type Value = T;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a non-null value")
        }

        fn visit_none<E: de::Error>(self) -> Result<T, E> {
            Err(E::custom("`null` is not allowed; omit the field instead"))
        }

        fn visit_unit<E: de::Error>(self) -> Result<T, E> {
            self.visit_none()
        }

        fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
            T::deserialize(deserializer)
        }
    }

    deserializer.deserialize_option(NonNull(PhantomData))
}
