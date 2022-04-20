use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, string::FromUtf8Error};

#[derive(Debug, Serialize, Deserialize)]
struct PrimitiveApplication {
    prim: String,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Contract(Vec<PrimitiveApplication>);

impl Contract {
    fn sort_primitive_applications(&mut self) {
        self.0.sort_by(|app1, app2| app1.prim.cmp(&app2.prim))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CanonicalizeError {
    #[error(transparent)]
    Serde(#[from] serde_json::error::Error),
    #[error(transparent)]
    Utf8Error(#[from] FromUtf8Error),
}

pub fn canonicalize_json_micheline(json_text: &str) -> Result<String, CanonicalizeError> {
    let mut contract: Contract = serde_json::from_str(json_text)?;

    // We notice that an on chain contract sometimes has its top-level primitive applications in a
    // different order. To make them consistent, sort them for now.
    contract.sort_primitive_applications();

    let mut buf = Vec::new();
    let formatter = olpc_cjson::CanonicalFormatter::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    contract.serialize(&mut ser)?;
    Ok(String::from_utf8(buf)?)
}
