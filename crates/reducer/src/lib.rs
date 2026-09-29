use serde_json::Value;

/// Greedily remove JSON object fields and array items while a caller confirms
/// that the original failure contract still matches.
pub fn minimize(mut value: Value, mut still_fails: impl FnMut(&Value) -> bool) -> Value {
    loop {
        let mut changed = false;
        match &value {
            Value::Object(object) => {
                for key in object.keys().cloned().collect::<Vec<_>>() {
                    let mut candidate = value.clone();
                    candidate.as_object_mut().expect("object").remove(&key);
                    if still_fails(&candidate) {
                        value = candidate;
                        changed = true;
                        break;
                    }
                }
            }
            Value::Array(items) => {
                for index in 0..items.len() {
                    let mut candidate = value.clone();
                    candidate.as_array_mut().expect("array").remove(index);
                    if still_fails(&candidate) {
                        value = candidate;
                        changed = true;
                        break;
                    }
                }
            }
            _ => {}
        }
        if !changed {
            return value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn removes_irrelevant_fields() {
        let reduced = minimize(json!({"needed": true, "debug": "remove me"}), |candidate| {
            candidate.get("needed") == Some(&json!(true))
        });
        assert_eq!(reduced, json!({"needed": true}));
    }
}
