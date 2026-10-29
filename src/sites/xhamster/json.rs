use serde_json::Value;

pub fn dig<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut node = root;
    for key in path.split('.') {
        node = node.as_object()?.get(key)?;
    }
    Some(node)
}

pub fn text(value: Option<&Value>) -> Option<String> {
    let found = value?.as_str()?;
    (!found.trim().is_empty()).then(|| found.to_string())
}

pub fn first_text(values: &[Option<&Value>]) -> Option<String> {
    values.iter().find_map(|value| text(*value))
}

pub fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

pub fn count(values: &[Option<&Value>]) -> Option<u64> {
    values.iter().find_map(|value| count_of(*value))
}

fn count_of(value: Option<&Value>) -> Option<u64> {
    let raw = match value? {
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        _ => return None,
    };
    let cleaned = raw.trim().replace(',', "").to_ascii_lowercase();
    let split = cleaned
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(cleaned.len());
    let (digits, suffix) = cleaned.split_at(split);
    let base: f64 = digits.parse().ok()?;
    let factor = match suffix.trim() {
        "" => 1.0,
        "k" | "thousand" => 1_000.0,
        "m" | "million" => 1_000_000.0,
        "b" | "billion" => 1_000_000_000.0,
        _ => return None,
    };
    let total = base * factor;
    (total.is_finite() && total >= 0.0).then(|| total.round() as u64)
}

pub fn flag(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

pub fn array<'a>(root: &'a Value, path: &str) -> &'a [Value] {
    dig(root, path)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn tidy(input: &str) -> String {
    decode_entities(input)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn strip_tags(input: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for character in input.chars() {
        match character {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(character),
            _ => {}
        }
    }
    tidy(&out)
}

pub fn find_array<'a>(root: &'a Value, key: &str, skip: &[&str]) -> Option<&'a Vec<Value>> {
    let mut queue = std::collections::VecDeque::from([root]);
    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if visited > 20_000 {
            return None;
        }
        match node {
            Value::Array(items) => queue.extend(items.iter()),
            Value::Object(map) => {
                for (name, value) in map {
                    if name == key {
                        if let Some(items) = value.as_array().filter(|items| !items.is_empty()) {
                            return Some(items);
                        }
                    }
                    if (value.is_object() || value.is_array()) && !skip.contains(&name.as_str()) {
                        queue.push_back(value);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn digs_and_tolerates_gaps() {
        let value = json!({"a": {"b": {"c": 3}}, "list": [1]});
        assert_eq!(dig(&value, "a.b.c").and_then(Value::as_i64), Some(3));
        assert!(dig(&value, "a.x.c").is_none());
        assert!(dig(&value, "list.0").is_none());
        assert!(array(&value, "nothing").is_empty());
    }

    #[test]
    fn counts_understand_suffixes_and_commas() {
        assert_eq!(count(&[Some(&json!("1.5K"))]), Some(1_500));
        assert_eq!(count(&[Some(&json!("2,400"))]), Some(2_400));
        assert_eq!(count(&[Some(&json!("3 million"))]), Some(3_000_000));
        assert_eq!(count(&[Some(&json!(12))]), Some(12));
        assert_eq!(count(&[Some(&json!("n/a")), Some(&json!(7))]), Some(7));
        assert_eq!(count(&[None]), None);
    }

    #[test]
    fn entities_and_tags_are_cleaned() {
        assert_eq!(
            decode_entities("a &amp; b &#39;c&#x21; &bogus; &"),
            "a & b 'c! &bogus; &"
        );
        assert_eq!(strip_tags("<b>Hello</b>  &quot;you&quot;"), "Hello \"you\"");
        assert_eq!(tidy("  a \n b "), "a b");
    }

    #[test]
    fn deep_search_skips_named_subtrees() {
        let value = json!({"skip": {"items": [1]}, "deep": {"x": [{"items": [2, 3]}]}});
        assert_eq!(find_array(&value, "items", &[]).map(Vec::len), Some(1));
        assert_eq!(
            find_array(&value, "items", &["skip"]).map(Vec::len),
            Some(2)
        );
    }
}
