//! Clairic reads an email to a debt collector and pulls out the objection and the disputed
//! amount. Clef decides from those two facts, never from the email, whether the sender
//! objects to the claim. Plain code then decides whether the email is flagged as a possible
//! dispute for the person who reads it.

use std::{collections::BTreeMap, sync::LazyLock};

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Client, Config, Fact, facts, values};

/// The fields, question and bar, from `config/dispute.toml`.
static CONFIG: LazyLock<Config> = LazyLock::new(|| Config::parse(include_str!("../config/dispute.toml")));

#[derive(Debug, Serialize)]
pub struct Triage {
    pub subject: String,
    pub facts: BTreeMap<String, Fact>,
    /// Clef's answer to the question in the config, the probability of yes.
    pub score: f64,
    /// True when the email looks like a dispute. `reasons` says why.
    pub flag: bool,
    pub reasons: Vec<String>,
    /// The full Clairic and Clef responses, kept so every result can be checked later.
    pub raw: Value,
}

impl Client {
    /// Triage one raw email (RFC 822, as an .eml file or an IMAP fetch returns it).
    pub fn triage_eml(&self, raw: &[u8]) -> Result<Triage> {
        let (subject, text, date) = read_eml(raw)?;
        self.triage(&subject, &text, date.as_deref())
    }

    /// Triage plain text. `reference_date` (YYYY-MM-DD) anchors dates written without a year.
    pub fn triage(&self, subject: &str, text: &str, reference_date: Option<&str>) -> Result<Triage> {
        let mut req = json!({
            "model": "clairic",
            "state": text,
            "fields": CONFIG.fields,
            "options": { "return_state_text": true },
        });
        if let Some(d) = reference_date {
            req["reference_date"] = json!(d);
        }
        let extracted = self.run("clairic", req)?;
        let facts = facts(&extracted, &CONFIG.fields)?;

        // Clef sees the typed values only. `objection` is still the sender's own words, so this
        // narrows what reaches the decision, it does not seal it.
        let state = values(&facts, "Nothing was extracted from the email.");
        let decided = self.run("clef", json!({ "model": "clef", "state": state, "questions": CONFIG.questions() }))?;
        let score = CONFIG.answer(&decided)?;

        let disputed = facts.get("disputed_amount").map_or(&Value::Null, |f| &f.value);
        let mut reasons = Vec::new();
        if extracted["truncated"] == true {
            reasons.push("email too long, part of it was not read".to_string());
        }
        if score >= CONFIG.bar {
            reasons.push(format!("objects {score:.2}"));
        }
        if !disputed.is_null() {
            reasons.push(format!("disputed amount {disputed} kr"));
        }

        Ok(Triage {
            subject: subject.to_string(),
            facts,
            score,
            flag: !reasons.is_empty(),
            reasons,
            raw: json!({ "clairic": extracted, "clef": decided }),
        })
    }
}

/// Subject, the text Clairic reads, and the sent date as YYYY-MM-DD.
fn read_eml(raw: &[u8]) -> Result<(String, String, Option<String>)> {
    let msg = mail_parser::MessageParser::default()
        .parse(raw)
        .ok_or_else(|| anyhow!("not a readable email"))?;
    let subject = msg.subject().unwrap_or("").to_string();
    let from = msg.from().and_then(|a| a.first()).and_then(|a| a.address()).unwrap_or("");
    let body = msg.body_text(0).unwrap_or_default();
    let date = msg.date().map(|d| format!("{:04}-{:02}-{:02}", d.year, d.month, d.day));
    let text = format!("From: {from}\nSubject: {subject}\n\n{body}");
    Ok((subject, text, date))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_config() {
        assert!(CONFIG.fields.get("objection").is_some());
        assert!((0.0..=1.0).contains(&CONFIG.bar));
    }

    #[test]
    fn reads_sample_email() {
        let (subject, text, date) = read_eml(include_bytes!("../emails/z13-to-purregebyr.eml")).unwrap();
        assert_eq!(subject, "Sak 2026-341170");
        assert!(text.starts_with("From: tuva.eide@example.no\n"));
        assert!(text.contains("to purregebyrer"));
        assert_eq!(date.as_deref(), Some("2026-10-24"));
    }
}
