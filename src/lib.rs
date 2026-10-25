//! Extract first, decide second, in two small examples. Clairic extracts typed facts, Clef
//! decides from those facts, and plain code makes the final call.
//!
//! - `dispute`: flag emails to a debt collector that dispute the claim.
//! - `invoice`: read a photo of an invoice and decide whether it is ready to pay.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub mod dispute;
mod ffi;
pub mod invoice;

/// One example's fields, question and bar, from its file in `config/`.
#[derive(Deserialize)]
struct Config {
    fields: Value,
    question: Question,
    bar: f64,
}

#[derive(Deserialize)]
struct Question {
    id: String,
    instructions: String,
}

impl Config {
    fn parse(toml: &str) -> Self {
        toml::from_str(toml).expect("config file is not valid")
    }

    /// The question in the shape Clef expects.
    fn questions(&self) -> Value {
        json!({ &self.question.id: { "type": "noul", "instructions": self.question.instructions } })
    }

    /// Clef's answer to the question, the probability of yes.
    fn answer(&self, decided: &Value) -> Result<f64> {
        decided["answers"][&self.question.id]["noul"].as_f64().context("Clef returned no answer")
    }
}

pub struct Client {
    account_id: String,
    token: String,
    http: reqwest::blocking::Client,
}

#[derive(Debug, Serialize)]
pub struct Fact {
    pub value: Value,
    pub confidence: Option<f64>,
    /// Character offsets into the text Clairic read (`raw.clairic.state_text`). Empty for
    /// values read from an image.
    pub spans: Vec<[usize; 2]>,
}

impl Client {
    pub fn new(account_id: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            account_id: account_id.into(),
            token: token.into(),
            http: reqwest::blocking::Client::new(),
        }
    }

    /// Reads CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN.
    pub fn from_env() -> Result<Self> {
        let get = |k: &str| std::env::var(k).with_context(|| format!("{k} is not set"));
        Ok(Self::new(get("CLOUDFLARE_ACCOUNT_ID")?, get("CLOUDFLARE_API_TOKEN")?))
    }

    fn run(&self, model: &str, body: Value) -> Result<Value> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/ai/run/@cf/cloudflare/{model}",
            self.account_id
        );
        let resp = self.http.post(url).bearer_auth(&self.token).json(&body).send()?;
        let status = resp.status();
        let text = resp.text()?;
        if !status.is_success() {
            bail!("{model} returned {status}: {text}");
        }
        let v: Value = serde_json::from_str(&text).with_context(|| format!("{model} returned non-JSON: {text}"))?;
        if v["success"] == false {
            bail!("{model} failed: {}", v["errors"]);
        }
        // The REST API wraps the model output in a Cloudflare envelope.
        Ok(v.get("result").cloned().unwrap_or(v))
    }
}

/// One `Fact` per requested field, from Clairic's raw answer.
fn facts(extracted: &Value, fields: &Value) -> Result<BTreeMap<String, Fact>> {
    let answers = extracted["answers"].as_object().context("Clairic returned no answers")?;
    let span = |v: &Value| -> Option<[usize; 2]> { Some([v.get(0)?.as_u64()? as usize, v.get(1)?.as_u64()? as usize]) };
    let mut out = BTreeMap::new();
    for name in fields.as_object().context("fields must be an object")?.keys() {
        let a = answers.get(name).cloned().unwrap_or(Value::Null);
        let value = a.get("value").cloned().unwrap_or(Value::Null);
        // A null value still carries a guess and its span, which point at nothing.
        let spans = if value.is_null() {
            vec![]
        } else if let Some(list) = a.get("spans").and_then(Value::as_array) {
            list.iter().filter_map(span).collect()
        } else {
            a.get("span").and_then(span).into_iter().collect()
        };
        out.insert(name.clone(), Fact { confidence: a.get("confidence").and_then(Value::as_f64), value, spans });
    }
    Ok(out)
}

/// The filled-in values, which is all Clef gets to see.
fn values(facts: &BTreeMap<String, Fact>, empty: &str) -> Value {
    let state: serde_json::Map<String, Value> = facts
        .iter()
        .filter(|(_, f)| !f.value.is_null())
        .map(|(k, f)| (k.clone(), f.value.clone()))
        .collect();
    if state.is_empty() { Value::String(empty.to_string()) } else { Value::Object(state) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_clairic_answers() {
        // Shapes from the example response in Cloudflare's Clairic docs.
        let fields = json!({ "objection": {}, "disputed_amount": {} });
        let r = json!({ "answers": {
            "objection": {"type": "text", "p_null": 3.8e-10, "span": [8, 16], "guess": "INV-2291", "confidence": 1, "value": "INV-2291"},
            "disputed_amount": {"type": "number", "p_null": 1, "guess": 3588, "confidence": 0.94, "value": null}
        }});
        let f = facts(&r, &fields).unwrap();
        assert_eq!(f["objection"].spans, vec![[8, 16]]);
        assert!(f["disputed_amount"].value.is_null() && f["disputed_amount"].spans.is_empty());
        assert_eq!(values(&f, "none"), json!({ "objection": "INV-2291" }));
    }
}
