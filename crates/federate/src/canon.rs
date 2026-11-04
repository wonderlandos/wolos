// SPDX-License-Identifier: BSD-2-Clause
//! The bytes a Federate signature covers (signatures.md, "Canonical signing format"): the record
//! as JSON with its `signature` set to `null`, compact (no whitespace), object keys sorted at every
//! level, arrays in order, strings and numbers as `serde_json` writes them (the reference
//! implementation's encoder).
use serde_json::Value;

/// Appends the canonical form of `v`.
pub fn write(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend(Value::String((*k).clone()).to_string().as_bytes());
                out.push(b':');
                write(&map[k.as_str()], out);
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write(item, out);
            }
            out.push(b']');
        }
        other => out.extend(other.to_string().as_bytes()),
    }
}

/// The signed bytes of a record: `None` when it is not a JSON object.
pub fn signable(record: &Value) -> Option<Vec<u8>> {
    let mut unsigned = record.as_object()?.clone();
    unsigned.insert("signature".into(), Value::Null);
    let mut out = Vec::new();
    write(&Value::Object(unsigned), &mut out);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_sorted_at_every_level_and_nothing_is_spaced() {
        let v: Value = serde_json::from_str(
            r#"{ "b": 1, "a": { "z": [3, {"y": 1, "x": "é\""}], "c": null } }"#,
        )
        .unwrap();
        let mut out = Vec::new();
        write(&v, &mut out);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            r#"{"a":{"c":null,"z":[3,{"x":"é\"","y":1}]},"b":1}"#
        );
    }

    #[test]
    fn the_signature_is_null_in_what_is_signed_present_or_not() {
        let with: Value = serde_json::from_str(r#"{"k":"v","signature":"ab"}"#).unwrap();
        let without: Value = serde_json::from_str(r#"{"k":"v"}"#).unwrap();
        let want = br#"{"k":"v","signature":null}"#.to_vec();
        assert_eq!(signable(&with).unwrap(), want);
        assert_eq!(signable(&without).unwrap(), want);
        assert_eq!(signable(&Value::Null), None);
    }
}
