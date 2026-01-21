use base64::Engine as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct Cursor {
  pub offset: u64,
  pub line: u64,
}

pub(crate) fn encode_cursor(c: Cursor) -> String {
  let json = serde_json::to_vec(&c).expect("cursor serialize");
  base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
}

pub(crate) fn decode_cursor(token: Option<&str>) -> Result<Cursor, crate::engine::CoreError> {
  match token {
    None => Ok(Cursor { offset: 0, line: 0 }),
    Some(t) if t.is_empty() => Ok(Cursor { offset: 0, line: 0 }),
    Some(t) => {
      let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(t)
        .map_err(|e| crate::engine::CoreError::BadCursor(e.to_string()))?;
      let c: Cursor = serde_json::from_slice(&bytes)
        .map_err(|e| crate::engine::CoreError::BadCursor(e.to_string()))?;
      Ok(c)
    }
  }
}

