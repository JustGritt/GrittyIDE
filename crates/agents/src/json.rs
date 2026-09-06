//! A JSON document that remembers the order its keys were written in (T221).
//!
//! `serde_json::Value` stores objects in a `BTreeMap`, so a file round-tripped
//! through it comes back **alphabetised**. ADR-018 §4 says the opposite: Gritty
//! is invited to append one entry to a file it does not own, and reordering
//! everything else in it is not a change the user asked for. The one this was
//! written against opens `attribution, permissions, model, hooks, …`, and an
//! installer that returned it as `askUserQuestionTimeout, attribution, …` would
//! have rewritten the whole file to add a line.
//!
//! `serde_json`'s `preserve_order` feature would fix that by swapping the map
//! for an `IndexMap`, but it is a *workspace-wide* switch: it makes every
//! `Value` in the tree larger, and `CoreError` carries one, which pushes the
//! core's whole `Result<_, CoreError>` surface past clippy's `result_large_err`
//! threshold. A repository-wide representation change to append one line to one
//! file is the wrong trade, so the ordering lives here instead — in the one
//! module that needs it, over a type nothing else sees.
//!
//! Order survives because serde's `MapAccess` yields entries in document order;
//! all this does is keep them in a `Vec` rather than sorting them into a map.

use std::fmt;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One JSON value, with object keys in the order they were read.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Json>),
    /// Entries in document order. A `Vec` rather than a map: a configuration
    /// file is a handful of keys, and lookup cost is irrelevant beside keeping
    /// the user's own ordering.
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Vec<(String, Json)>> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Json>> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// The value of one key, or `None` for a non-object or a missing key.
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.as_object()?
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }
}

/// The entry for `key`, appended in place if it is not there yet — which is
/// what keeps a newly created key at the end rather than at the front.
pub fn entry<'a>(object: &'a mut Vec<(String, Json)>, key: &str, default: Json) -> &'a mut Json {
    let index = match object.iter().position(|(name, _)| name == key) {
        Some(index) => index,
        None => {
            object.push((key.to_owned(), default));
            object.len() - 1
        }
    };
    &mut object[index].1
}

/// Remove one key, keeping the order of everything else.
pub fn remove(object: &mut Vec<(String, Json)>, key: &str) {
    object.retain(|(name, _)| name != key);
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(value) => value.serialize(serializer),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Self::Object(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_none<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Json, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Json, E> {
        Ok(Json::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Json, E> {
        Ok(Json::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Json, E> {
        Ok(Json::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Json, E> {
        // A non-finite float cannot appear in JSON input, so the fallback is
        // unreachable in practice — and zero is still a number rather than a
        // panic if a future deserializer offers one.
        Ok(Json::Number(
            serde_json::Number::from_f64(value).unwrap_or_else(|| 0.into()),
        ))
    }

    fn visit_str<E>(self, value: &str) -> Result<Json, E> {
        Ok(Json::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Json, E> {
        Ok(Json::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        // `MapAccess` yields entries in document order, which is the whole
        // reason this type exists.
        let mut entries = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, Json>()? {
            entries.push((key, value));
        }
        Ok(Json::Object(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_order_survives_a_round_trip() {
        // Alphabetically these would come back the other way round, which is
        // exactly what `serde_json::Value` would have done.
        let raw = r#"{"zebra":1,"apple":{"nested":true,"also":[1,2]},"model":"opus"}"#;
        let parsed: Json = serde_json::from_str(raw).expect("parse");
        assert_eq!(serde_json::to_string(&parsed).expect("emit"), raw);
    }

    #[test]
    fn every_json_type_round_trips() {
        let raw = r#"{"n":null,"t":true,"i":-3,"u":7,"f":1.5,"s":"x","a":[],"o":{}}"#;
        let parsed: Json = serde_json::from_str(raw).expect("parse");
        assert_eq!(serde_json::to_string(&parsed).expect("emit"), raw);
    }

    #[test]
    fn pretty_printing_uses_two_spaces() {
        let parsed: Json = serde_json::from_str(r#"{"a":{"b":1}}"#).expect("parse");
        assert_eq!(
            serde_json::to_string_pretty(&parsed).expect("emit"),
            "{\n  \"a\": {\n    \"b\": 1\n  }\n}"
        );
    }

    #[test]
    fn a_new_key_lands_at_the_end_rather_than_disturbing_the_order() {
        let mut parsed: Json = serde_json::from_str(r#"{"b":1,"a":2}"#).expect("parse");
        let object = parsed.as_object_mut().expect("object");
        *entry(object, "c", Json::Null) = Json::Bool(true);
        assert_eq!(
            serde_json::to_string(&parsed).expect("emit"),
            r#"{"b":1,"a":2,"c":true}"#
        );

        // And an existing key is edited where it stands.
        let object = parsed.as_object_mut().expect("object");
        *entry(object, "b", Json::Null) = Json::Bool(false);
        assert_eq!(
            serde_json::to_string(&parsed).expect("emit"),
            r#"{"b":false,"a":2,"c":true}"#
        );

        remove(parsed.as_object_mut().expect("object"), "a");
        assert_eq!(
            serde_json::to_string(&parsed).expect("emit"),
            r#"{"b":false,"c":true}"#
        );
    }
}
