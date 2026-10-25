//! clairic dispute <folder>   flag every .eml in a folder, and score it against key.json if there is one
//! clairic dispute imap       flag the newest messages in an IMAP mailbox, read-only
//! clairic invoice <folder>   read every invoice photo in a folder, and score it against key.json
//!
//! Every run writes out/<unix time>/results.jsonl.

use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};

use anyhow::{Context, Result, bail};
use clairic::Client;
use serde_json::{Value, json};

type Files = Vec<(String, Vec<u8>)>;
type Key = Option<BTreeMap<String, Value>>;

fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: clairic dispute <folder> | clairic dispute imap | clairic invoice <folder>";
    let (cmd, src) = match args.as_slice() {
        [c, s] => (c.as_str(), s.as_str()),
        _ => bail!(usage),
    };
    let client = Client::from_env()?;
    let out_dir = PathBuf::from("out").join(
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs().to_string(),
    );
    fs::create_dir_all(&out_dir)?;
    let mut out = fs::File::create(out_dir.join("results.jsonl"))?;

    match (cmd, src) {
        ("dispute", "imap") => disputes(&client, from_imap()?, None, &mut out)?,
        ("dispute", dir) => {
            let (files, key) = from_folder(dir.into(), &["eml"])?;
            disputes(&client, files, key, &mut out)?
        }
        ("invoice", dir) => {
            let (files, key) = from_folder(dir.into(), &["jpg", "jpeg", "png", "webp"])?;
            invoices(&client, files, key, &mut out)?
        }
        _ => bail!(usage),
    }
    println!("saved {}", out_dir.join("results.jsonl").display());
    Ok(())
}

fn disputes(client: &Client, emails: Files, key: Key, out: &mut fs::File) -> Result<()> {
    let (mut scored, mut right, mut missed, mut false_flags) = (0, 0, 0, 0);
    for (name, raw) in &emails {
        let want = key.as_ref().and_then(|k| k.get(name));
        let t = match client.triage_eml(raw) {
            Ok(t) => t,
            Err(e) => {
                println!("{name}  ERROR {e:#}");
                // An email that never got an answer was never flagged either.
                if let Some(w) = want {
                    scored += 1;
                    right += (w["dispute"] == false) as usize;
                    missed += (w["dispute"] == true) as usize;
                }
                writeln!(out, "{}", json!({ "email": name, "error": format!("{e:#}") }))?;
                continue;
            }
        };
        let flag = if t.flag { "DISPUTE?" } else { "-" };
        println!("{name}  objects {:.2}  {flag}  {}", t.score, t.reasons.join(", "));
        let mut row = json!({ "email": name, "triage": t });
        if let Some(want) = want {
            scored += 1;
            let r = want["dispute"].as_bool() == Some(t.flag);
            right += r as usize;
            missed += (want["dispute"] == true && !t.flag) as usize;
            false_flags += (want["dispute"] == false && t.flag) as usize;
            if !r {
                println!("    expected {}", if want["dispute"] == true { "DISPUTE?" } else { "-" });
            }
            row["expected"] = want.clone();
        }
        writeln!(out, "{row}")?;
    }
    if key.is_some() {
        println!("\nright {right}/{scored}  missed disputes {missed}  false flags {false_flags}");
    }
    Ok(())
}

fn invoices(client: &Client, photos: Files, key: Key, out: &mut fs::File) -> Result<()> {
    let (mut scored, mut right, mut wrongly_ready, mut fields_ok, mut fields_n) = (0, 0, 0, 0, 0);
    for (name, image) in &photos {
        let want = key.as_ref().and_then(|k| k.get(name));
        let inv = match client.read_invoice(image) {
            Ok(i) => i,
            Err(e) => {
                println!("{name}  ERROR {e:#}");
                // A photo that never got an answer was never marked ready either.
                if let Some(w) = want {
                    scored += 1;
                    right += (w["ready"] == false) as usize;
                }
                writeln!(out, "{}", json!({ "photo": name, "error": format!("{e:#}") }))?;
                continue;
            }
        };
        let v = |k: &str| inv.facts.get(k).map_or("null".to_string(), |f| f.value.to_string());
        println!(
            "{name}  {}  {} kr  due {}  account {}  KID {}  new invoice {:.2}  {}",
            if inv.ready { "READY" } else { "CHECK" },
            v("amount_due"), v("due_date"), v("account_number"), v("kid"), inv.score,
            inv.reasons.join(", ")
        );
        let mut row = json!({ "photo": name, "invoice": inv });
        if let Some(want) = want {
            scored += 1;
            let r = want["ready"].as_bool() == Some(inv.ready);
            right += r as usize;
            wrongly_ready += (want["ready"] == false && inv.ready) as usize;
            if !r {
                println!("    expected {}", if want["ready"] == true { "READY" } else { "CHECK" });
            }
            // Every other key in the expected record is a field to read exactly.
            for (field, expected) in want.as_object().into_iter().flatten().filter(|(f, _)| *f != "ready") {
                let Some(fact) = inv.facts.get(field.as_str()) else {
                    println!("    {field}: not a field the invoice reader extracts, check key.json");
                    continue;
                };
                let got = &fact.value;
                let same = match (got, expected) {
                    (Value::String(g), Value::String(e)) if field == "account_number" || field == "kid" => {
                        g.chars().filter(char::is_ascii_digit).collect::<String>() == *e
                    }
                    (Value::Number(g), Value::Number(e)) => g.as_f64() == e.as_f64(),
                    _ => got == expected,
                };
                fields_n += 1;
                fields_ok += same as usize;
                if !same {
                    println!("    {field}: read {got}, expected {expected}");
                }
            }
            row["expected"] = want.clone();
        }
        writeln!(out, "{row}")?;
    }
    if key.is_some() {
        println!("\nright {right}/{scored}  wrongly ready {wrongly_ready}  fields read exactly {fields_ok}/{fields_n}");
    }
    Ok(())
}

fn from_folder(dir: PathBuf, extensions: &[&str]) -> Result<(Files, Key)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
        if ext.is_some_and(|e| extensions.contains(&e.as_str())) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            files.push((name, fs::read(&path)?));
        }
    }
    files.sort();
    let key = match fs::read_to_string(dir.join("key.json")) {
        Ok(s) => Some(serde_json::from_str(&s).context("key.json is not valid")?),
        Err(_) => None,
    };
    Ok((files, key))
}

/// Opens the folder with EXAMINE and fetches BODY.PEEK[], so nothing is marked as read.
fn from_imap() -> Result<Files> {
    let env = |k: &str| std::env::var(k).with_context(|| format!("{k} is not set"));
    let host = env("IMAP_HOST")?;
    let folder = env("IMAP_FOLDER").unwrap_or_else(|_| "INBOX".into());
    let limit: u32 = env("IMAP_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(20);

    let tls = native_tls::TlsConnector::new()?;
    let client = imap::connect((host.as_str(), 993), &host, &tls)?;
    let mut session = client.login(env("IMAP_USER")?, env("IMAP_PASSWORD")?).map_err(|e| e.0)?;
    let exists = session.examine(&folder)?.exists;
    let mut emails = Vec::new();
    if exists > 0 {
        let first = exists.saturating_sub(limit.max(1) - 1).max(1);
        for m in session.fetch(format!("{first}:{exists}"), "BODY.PEEK[]")?.iter() {
            if let Some(body) = m.body() {
                emails.push((format!("{folder}#{}", m.message), body.to_vec()));
            }
        }
    }
    session.logout()?;
    Ok(emails)
}
