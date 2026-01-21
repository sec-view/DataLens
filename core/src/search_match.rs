use crate::models::SearchQuery;

#[derive(Debug, Clone)]
pub(crate) struct PreparedKv {
  pub(crate) key: String,
  pub(crate) key_quoted: String,
  pub(crate) value: String,
  pub(crate) value_quoted: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedSearch {
  pub(crate) q: String,
  pub(crate) q_quoted: String,
  pub(crate) kv: Option<PreparedKv>,
}

fn strip_quotes(s: &str) -> String {
  let t = s.trim();
  if (t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')) {
    return t[1..t.len().saturating_sub(1)].to_string();
  }
  t.to_string()
}

fn parse_key_value_query(q: &str) -> Option<(String, String)> {
  let t = q.trim();
  let idx = t.find(':')?;
  if idx == 0 {
    return None;
  }
  let k = t[..idx].trim();
  let v = t[idx + 1..].trim();
  if k.is_empty() || v.is_empty() {
    return None;
  }
  Some((strip_quotes(k), strip_quotes(v)))
}

fn json_quote(s: &str) -> String {
  serde_json::to_string(s).unwrap_or_else(|_| format!("\"{}\"", s.replace('"', "\\\"")))
}

impl PreparedSearch {
  pub(crate) fn new(query: &SearchQuery) -> Option<Self> {
    let t = query.text.trim();
    if t.is_empty() {
      return None;
    }

    let case_sensitive = query.case_sensitive;
    let norm = |s: String| if case_sensitive { s } else { s.to_lowercase() };

    let q = norm(t.to_string());
    let q_quoted = norm(json_quote(t));

    let kv = parse_key_value_query(t).map(|(k, v)| PreparedKv {
      key: norm(k.clone()),
      key_quoted: norm(json_quote(&k)),
      value: norm(v.clone()),
      value_quoted: norm(json_quote(&v)),
    });

    Some(Self {
      q,
      q_quoted,
      kv,
    })
  }

  /// `hay` must already be normalized according to `case_sensitive`:
  /// - case_sensitive=true  => original text
  /// - case_sensitive=false => lowercased text
  pub(crate) fn matches_in_hay(&self, hay: &str) -> bool {
    if let Some(kv) = &self.kv {
      let key_ok = hay.contains(&kv.key) || hay.contains(&kv.key_quoted);
      let val_ok = hay.contains(&kv.value) || hay.contains(&kv.value_quoted);
      return key_ok && val_ok;
    }
    hay.contains(&self.q) || hay.contains(&self.q_quoted)
  }
}

