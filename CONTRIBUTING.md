# Contributing

Bug reports and small pull requests are welcome. For a new command or a change to the public interface, open an issue first to discuss the shape of the change. Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## Local checks

Install Rust 1.85 or later and Node.js 20 or later, then run:

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
node --test scripts/check-source-hygiene.test.mjs
node scripts/check-source-hygiene.mjs
```

Tests should use synthetic inputs and recorded HTTP fixtures. A live Core smoke test is optional and requires your own `KOLDERA_URL` and `KOLDERA_TOKEN`; never commit these values or real customer files. Keep the CLI's output honest about machine checks and human review.

By submitting a contribution, you agree to license it under this repository's [Apache-2.0 licence](LICENSE) (section 5). Koldera Core is a separate product and accepts contributions under its own terms.
