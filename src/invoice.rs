//! Clairic reads a photo of an invoice and pulls out what a payment needs. Clef decides from
//! those facts, never from the photo, whether it is a new invoice to pay. Plain code checks
//! the account number and the KID, and only then marks the invoice ready to pay.

use std::{collections::BTreeMap, sync::LazyLock};

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Client, Config, Fact, facts, values};

/// The fields, question and bar, from `config/invoice.toml`.
static CONFIG: LazyLock<Config> = LazyLock::new(|| Config::parse(include_str!("../config/invoice.toml")));

#[derive(Debug, Serialize)]
pub struct Invoice {
    pub facts: BTreeMap<String, Fact>,
    /// Clef's answer to the question in the config, the probability of yes.
    pub score: f64,
    /// The account number and KID as digits only, when they pass their check digit.
    pub account: Option<String>,
    pub kid: Option<String>,
    pub ready: bool,
    /// Why it is not ready. Empty when `ready` is true.
    pub reasons: Vec<String>,
    /// The full Clairic and Clef responses, kept so every result can be checked later.
    pub raw: Value,
}

impl Client {
    /// Read one photo or scan of an invoice (PNG, JPEG or WebP).
    pub fn read_invoice(&self, image: &[u8]) -> Result<Invoice> {
        let content_type = image_type(image).context("not a PNG, JPEG or WebP image")?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(image);
        // The text part stays empty. With any text there, Clairic copies text answers from it
        // instead of reading them from the image.
        let req = json!({
            "model": "clairic",
            "state": "",
            "images": [{ "content_type": content_type, "base64": b64 }],
            "fields": CONFIG.fields,
        });
        let extracted = self.run("clairic", req)?;
        let facts = facts(&extracted, &CONFIG.fields)?;

        let state = values(&facts, "Nothing could be read from the image.");
        let decided = self.run("clef", json!({ "model": "clef", "state": state, "questions": CONFIG.questions() }))?;
        let score = CONFIG.answer(&decided)?;

        let value = |k: &str| facts.get(k).map_or(&Value::Null, |f| &f.value);
        let text = |k: &str| value(k).as_str().map(digits);
        let account = text("account_number");
        let kid = text("kid");
        let mut reasons = Vec::new();
        if score < CONFIG.bar {
            reasons.push(format!("may not be a new invoice ({score:.2})"));
        }
        if value("amount_due").as_f64().is_none_or(|a| a <= 0.0) {
            reasons.push("no amount to pay".to_string());
        }
        if value("due_date").is_null() {
            reasons.push("no due date".to_string());
        }
        match &account {
            None => reasons.push("no account number".to_string()),
            Some(a) if a.len() != 11 => reasons.push(format!("account number {a} has {} digits, expected 11", a.len())),
            Some(a) if !account_ok(a) => reasons.push(format!("account number {a} fails its check digit")),
            _ => {}
        }
        match &kid {
            None => reasons.push("no KID".to_string()),
            Some(k) if !kid_ok(k) => reasons.push(format!("KID {k} fails its check digit")),
            _ => {}
        }

        Ok(Invoice {
            account: account.filter(|a| account_ok(a)),
            kid: kid.filter(|k| kid_ok(k)),
            facts,
            score,
            ready: reasons.is_empty(),
            reasons,
            raw: json!({ "clairic": extracted, "clef": decided }),
        })
    }
}

fn image_type(b: &[u8]) -> Result<&'static str> {
    Ok(match b {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => bail!("unknown image type"),
    })
}

fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// Norwegian account numbers: 11 digits, the last one a MOD11 check digit.
fn account_ok(a: &str) -> bool {
    a.len() == 11 && mod11(&a[..10], &[5, 4, 3, 2, 7, 6, 5, 4, 3, 2]) == a[10..].parse().ok()
}

/// A KID is 2 to 25 digits, the last one a MOD10 (Luhn) or MOD11 check digit. The payer can't
/// tell which of the two the company uses, so either is accepted. That lets about 9% of
/// single-digit misreads through, where one scheme alone would catch them all. A MOD11 KID
/// whose check character is `-` always fails here, because only digits are kept.
fn kid_ok(k: &str) -> bool {
    if !(2..=25).contains(&k.len()) {
        return false;
    }
    let (body, check) = k.split_at(k.len() - 1);
    let check: u32 = check.parse().unwrap_or(99);
    let luhn: u32 = body
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, d)| {
            let n = u32::from(d - b'0') * if i % 2 == 0 { 2 } else { 1 };
            if n > 9 { n - 9 } else { n }
        })
        .sum();
    let weights: Vec<u32> = (0..body.len()).map(|i| 2 + (i as u32 % 6)).collect::<Vec<_>>().into_iter().rev().collect();
    (10 - luhn % 10) % 10 == check || mod11(body, &weights) == Some(check)
}

/// The MOD11 check digit for `body` with the given weights, or None when there is none.
fn mod11(body: &str, weights: &[u32]) -> Option<u32> {
    let sum: u32 = body.bytes().zip(weights).map(|(d, w)| u32::from(d - b'0') * w).sum();
    match 11 - sum % 11 {
        11 => Some(0),
        10 => None,
        c => Some(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_config() {
        assert!(CONFIG.fields.get("account_number").is_some());
        assert!((0.0..=1.0).contains(&CONFIG.bar));
    }

    #[test]
    fn checks_account_numbers() {
        assert!(account_ok("15032748594"));
        assert!(!account_ok("97100512348"));
        assert!(!account_ok("1503274859"));
    }

    #[test]
    fn checks_kids() {
        assert!(kid_ok("0001234566"));
        assert!(kid_ok("2300471188"));
        assert!(!kid_ok("2300471189"));
    }

    #[test]
    fn detects_image_types() {
        assert_eq!(image_type(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap(), "image/jpeg");
        assert!(image_type(b"From: someone").is_err());
    }
}
