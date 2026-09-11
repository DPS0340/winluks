use crate::{Error, Result};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

struct Node(usize);
impl<'de> DeserializeSeed<'de> for Node {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> std::result::Result<Value, D::Error> {
        if self.0 > 32 {
            return Err(de::Error::custom("depth"));
        }
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Node {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_unit<E: de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Value, E> {
        Ok(Value::Number(Number::from(v)))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Value, E> {
        Ok(Value::Number(Number::from(v)))
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<Value, E> {
        Err(E::custom("non-integer"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Value, E> {
        Ok(Value::String(v.into()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<Value, A::Error> {
        let mut v = Vec::new();
        while let Some(x) = a.next_element_seed(Node(self.0 + 1))? {
            v.push(x);
        }
        Ok(Value::Array(v))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<Value, A::Error> {
        let mut m = Map::new();
        while let Some(k) = a.next_key::<String>()? {
            if m.contains_key(&k) {
                return Err(de::Error::custom("duplicate key"));
            }
            m.insert(k, a.next_value_seed(Node(self.0 + 1))?);
        }
        Ok(Value::Object(m))
    }
}
pub(crate) fn parse(bytes: &[u8]) -> Result<Value> {
    let mut d = serde_json::Deserializer::from_slice(bytes);
    let v = Node(0)
        .deserialize(&mut d)
        .map_err(|_| Error::MetadataInvalid)?;
    d.end().map_err(|_| Error::MetadataInvalid)?;
    Ok(v)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_and_overflow_rejected() {
        for s in [
            r#"{"x":1,"x":2}"#,
            r#"{"n":18446744073709551616}"#,
            r#"{"n":1.5}"#,
        ] {
            assert!(parse(s.as_bytes()).is_err());
        }
    }
    #[test]
    fn depth_limited() {
        assert!(parse(format!("{}0{}", "[".repeat(34), "]".repeat(34)).as_bytes()).is_err());
    }
}
