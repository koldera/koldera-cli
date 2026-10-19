# Koldera CLI

`koldera` runs accessibility jobs against a Koldera Core server from a terminal or a script. It can analyze or repair a PDF, render HTML or a structured document to a tagged PDF, and scan one web page. The CLI uses the Core HTTP API; it does not include the Core server or a local PDF engine.

**Status:** source preview for the 0.1 API. No CLI binary or package has been released yet. Koldera Core is an
AGPL-3.0-only, self-hosted server; you need a running Core instance and a server token.

Automated PDF/UA and web checks do not establish that a document or page is accessible. Review the findings and the output with a person before making a conformance claim.

## Build and install from source

Rust 1.85 or later is required. From this repository:

```sh
cargo build --locked --release
cargo install --path . --locked
koldera --version
```

`cargo install` places the executable in Cargo's bin directory. If you only want to try it from the checkout, use `cargo run --locked --` before the commands below.

## Connect to Core

Set the server origin and bearer token in your shell or secret manager:

```sh
export KOLDERA_URL=https://core.example.com
export KOLDERA_TOKEN=your-server-token
koldera pdf report.pdf
```

You can pass `--server https://core.example.com` instead of `KOLDERA_URL`. The token is read only from `KOLDERA_TOKEN`, never from a command-line flag. Use HTTPS outside a trusted local network. Both `--server` and `KOLDERA_URL` are omitted from output; errors do not print the token.

The CLI requires a running server. The reserved `--local` and `--image` options currently return a usage error; they do not start Docker. For jobs inside the Core image, use the CLI supplied with that image.

## Commands

```sh
koldera pdf report.pdf                         # machine-check the original
koldera pdf report.pdf --fix --out results      # write a separate repaired copy
koldera html page.html --out results            # render static HTML, then check
koldera scan https://example.com --out results  # scan one public web page
koldera document invoice.json --out results     # render semantic JSON to PDF
```

The original PDF or HTML file is never overwritten. `--out` selects the output directory; by default artifacts go to the current directory. `--json` writes one machine-readable object containing the job, findings, actions and artifacts to stdout. Run `koldera --help` or `koldera <command> --help` for all options, including PDF language and title overrides and the HTML content selector.

Structured-document images use paths relative to the JSON file. Paths outside that directory are refused. The document schema is in [`contracts/document.schema.json`](contracts/document.schema.json).

Exit status is `0` for a completed passing machine check, `2` for completed checks with failed rules, `3` for a failed job or no machine verdict, `64` for usage errors, and `130` when interrupted. Treat status `0` as a machine result that still needs human review.

## Develop and verify

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
node --test scripts/check-source-hygiene.test.mjs
node scripts/check-source-hygiene.mjs
```

The tests use synthetic files and recorded HTTP fixtures; a Core deployment is not needed. To run the optional live smoke test, set `KOLDERA_URL` and `KOLDERA_TOKEN`, then run `node scripts/e2e-live.mjs`.

[`contracts/`](contracts) is a vendored copy of the shared API contract. `contracts/CORE_VERSION` identifies the
[Core](https://github.com/koldera/koldera-core) source revision and `contracts/KIT_VERSION` identifies the
[`sdk-contract-kit`](https://github.com/koldera/sdk-contract-kit) revision used to update the copy.

## Licence and support

This CLI is licensed under [Apache-2.0](LICENSE). Koldera Core is a separate server licensed under
[AGPL-3.0-only](https://github.com/koldera/koldera-core/blob/main/LICENSE). You can build and run Core from its source;
follow the terms of its licence when using or modifying it.

See [CONTRIBUTING.md](CONTRIBUTING.md) for changes and [SECURITY.md](SECURITY.md) for private vulnerability reports. Do not put credentials, documents or signed download URLs in a public issue.
