use serde_json::Value;

use super::json::{count, strip_tags, text};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Search,
    Pornstar,
    Creator,
    Channel,
    Celebrity,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Search => "",
            Kind::Pornstar => "pornstar",
            Kind::Creator => "creator",
            Kind::Channel => "channel",
            Kind::Celebrity => "celebrity",
        }
    }

    pub fn is_person(self) -> bool {
        self != Kind::Search
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    pub query: String,
    pub url: String,
    pub kind: Kind,
    pub count: Option<u64>,
}

fn entry(raw: &Value) -> Option<Suggestion> {
    let url = text(raw.get("link"))?;
    let plain = text(raw.get("plainText"))
        .unwrap_or_else(|| strip_tags(&text(raw.get("text")).unwrap_or_default()));
    let shown = strip_tags(&text(raw.get("text")).unwrap_or_else(|| plain.clone()));
    let query = if plain.is_empty() {
        shown.clone()
    } else {
        plain
    };
    if query.is_empty() || shown.is_empty() {
        return None;
    }
    let declared = text(raw.get("type2")).unwrap_or_default().to_lowercase();
    let kind = match declared.as_str() {
        "search" => Kind::Search,
        "pornstar" => Kind::Pornstar,
        "creator" => Kind::Creator,
        "channel" => Kind::Channel,
        "celebrity" => Kind::Celebrity,
        "user" => return None,
        _ if url.contains("/search/") => Kind::Search,
        _ => Kind::Pornstar,
    };
    Some(Suggestion {
        text: shown,
        query,
        url,
        kind,
        count: count(&[raw.get("count")]).filter(|found| *found > 0),
    })
}

pub fn shape(data: &Value) -> Vec<Suggestion> {
    let Some(rows) = data.as_array() else {
        return Vec::new();
    };
    let mut out: Vec<Suggestion> = Vec::new();
    for row in rows {
        let Some(found) = entry(row) else {
            continue;
        };
        let duplicate = out.iter().any(|held| {
            held.kind == found.kind
                && held.query.eq_ignore_ascii_case(&found.query)
                && held.url == found.url
        });
        if !duplicate {
            out.push(found);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rows_become_searches_and_people_without_user_accounts() {
        let data = json!([
            {"link": "https://x.test/search/lofi-girl", "text": "<b>lofi</b> girl", "plainText": "lofi girl", "type2": "search", "count": "1,200"},
            {"link": "https://x.test/pornstars/jane", "text": "Jane Doe", "type2": "pornstar", "count": 12},
            {"link": "https://x.test/creators/amy", "text": "Amy", "type2": "creator"},
            {"link": "https://x.test/users/someone", "text": "Someone", "type2": "user"},
            {"link": "https://x.test/search/lofi-girl", "text": "Lofi Girl", "plainText": "LOFI GIRL", "type2": "search"},
            {"link": "https://x.test/other", "text": "Mystery"},
            {"text": "no link"},
            {"link": "https://x.test/search/x", "text": ""}
        ]);
        let rows = shape(&data);
        let described: Vec<(&str, Kind)> = rows
            .iter()
            .map(|row| (row.query.as_str(), row.kind))
            .collect();
        assert_eq!(
            described,
            vec![
                ("lofi girl", Kind::Search),
                ("Jane Doe", Kind::Pornstar),
                ("Amy", Kind::Creator),
                ("Mystery", Kind::Pornstar),
            ]
        );
        assert_eq!(rows[0].text, "lofi girl");
        assert_eq!(rows[0].count, Some(1_200));
        assert!(rows[1].kind.is_person() && !rows[0].kind.is_person());
        assert!(shape(&json!({"not": "a list"})).is_empty());
    }
}
