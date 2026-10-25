# clairic

**Part of aotn**, a series of small educational example projects. This one goes with the article "Extract First, Decide Second". The code is here to learn from, not to run in production. All emails and invoices are made up, and none of it is legal advice.

It shows two ways to combine Cloudflare's System One models. Clairic extracts typed values, Clef answers a yes/no question with a probability, and plain code makes the final call.

- **Disputes.** A debt collector's staff read every incoming email. In Norway a disputed claim (*omtvistet krav*) can't continue through ordinary collection, so the helper flags the emails that look like a dispute.
- **Invoices.** Clairic reads a photo of an invoice, and the code decides whether it is ready to pay.

## Set up Cloudflare

1. Log in at dash.cloudflare.com. Your account ID is the long string in the address bar, `dash.cloudflare.com/<account-id>/...`.
2. Go to **My Profile → API Tokens → Create Token**, pick the **Workers AI** template and create it. Copy the token, it is shown once.
3. Copy `.env.example` to `.env` and fill in `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN`.

Clairic and Clef are both called through the Workers AI API with these two values. Nothing is deployed.

## Run it

You need Rust.

```sh
cargo run --release -- dispute emails      # flag the 30 test emails
cargo run --release -- dispute imap        # flag the newest emails in a real mailbox
cargo run --release -- invoice invoices    # read the 6 test invoice photos
```

Each run prints one line per file and saves every answer to `out/<unix time>/results.jsonl`. When the folder has a `key.json`, it also prints a score. For `imap`, fill in the `IMAP_` lines in `.env`. The mailbox is opened read-only and nothing is marked as read.

## Use it from C#

`cargo build --release` also builds a shared library (`.dylib`, `.so` or `.dll`) with three C functions.

```c
char *clairic_flag_dispute(const uint8_t *raw_email, size_t len);   // returns JSON
char *clairic_read_invoice(const uint8_t *image, size_t len);       // returns JSON
void  clairic_free(char *s);
```

`examples/dotnet` calls it from C#. `dotnet run` compiles and starts it. The library reads the two Cloudflare values from the environment, not from `.env`.

```sh
cargo build --release
set -a; . ./.env; set +a
cd examples/dotnet
dotnet run -- ../../emails/z03-angrerett.eml
dotnet run -- ../../invoices/f01-faktura.jpg
```

## How each example works

**Disputes** (`config/dispute.toml`, `src/dispute.rs`). Clairic extracts the sentence where the sender objects and the amount they say they don't owe. Clef answers one question from those two facts. Does the sender object to the claim, any part of it, or any fee? The email is flagged as `DISPUTE?` when the answer is 0.2 or more, when there is a disputed amount, or when the email was too long to read in full. The bar is low because the reader sees every email anyway.

**Invoices** (`config/invoice.toml`, `src/invoice.rs`). Clairic reads the title, payee, amount, due date, account number and KID from the photo. Clef decides from those values whether it is a new invoice to pay, and not a reminder, credit note or receipt. The code checks the check digits of the account number and the KID. The invoice is `READY` only when Clef says 0.8 or more and every check passes. The bar is high because a wrong payment costs money.

## Make your own example

The fields Clairic extracts, the question Clef answers and the bar are in `config/dispute.toml` and `config/invoice.toml`. Edit them and run the same command again. `cargo run` rebuilds with the new file, and the files are built into the library, so the C# example picks them up after `cargo build --release`.

The rule that uses the answer stays in code, in `src/dispute.rs` and `src/invoice.rs`, together with the check-digit checks. Keep the field names those files use, or change the rule to match. Clef reads the question's ID as part of the question, so a new ID can give different answers even with the same wording.

## Results

**Disputes**, 30 emails in `emails/`, 15 of them disputes, run on 24 October 2026.

| | disputes flagged | false flags |
|---|---|---|
| Clairic, then Clef (this code) | 14 of 15 | 2 |
| Clef on the whole email (measured once, not in the code) | 15 of 15 | 1 |

- Clef did better with the whole email. Extracted sentences lose their context, and "the letter was unnecessarily strict" looks like an objection until you read "but the amount is paid".
- Clairic was most useful for numbers. It worked out the 600 kr difference between a contract and a claim, which caught a dispute Clef missed.
- An instruction in `z10` telling the system the customer agrees to pay changed nothing in either setup.
- The question was reworded once after the first run on these emails.

**Invoices**, 6 photos in `invoices/`.

- No invoice was wrongly marked ready, but none was marked ready at all. Clairic dropped a digit from every account number, and the length check caught each one.
- KIDs and dates were read exactly. One amount was misread, 1,248.55 for 1,248.50, with confidence 0.02.
- Titles and payee names came back garbled, such as "FAKURA" and "PURURING".

## Limits

- Clef only sees what Clairic extracted. If extraction misses something, so does the decision.
- Extracted text is the sender's own words, so an instruction in an email can still reach Clef.
- Clef reads a question's ID as part of the question. With the same input, `z11` scored 0.84 under the ID `not_owed` and 0.25 under `objects`, so the ID stays `not_owed`. The same input otherwise always gives the same answer.
- Cloudflare doesn't say which languages the models support. The emails are Norwegian, nynorsk and English.
- Clairic's confidence warns about wrong amounts, but not about wrong account numbers. Always check account numbers and KIDs with their length and check digit.
- A KID can use one of two check schemes, and the payer can't tell which, so either is accepted. That lets about 9% of single-digit misreads through, where one scheme alone would catch them all.
