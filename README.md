# Ferron 3 Example Modules

Example Ferron 3 modules for `https://github.com/ferronweb/ferron` (`3.x` branch).
Each crate is a minimal, documented `ModuleLoader` library that you can copy
as a starting point for your own Ferron modules.

- Rust workspace of `modules/*` (excluded: `e2e`, `fixture`).
- `fixture/`: binary that wires all examples via `ferron-entrypoint` (git `3.x`).
- `e2e/`: end-to-end tests via `testcontainers` + `Dockerfile.test`.
- `docs/`: documentation for the example directives.

## Quick start

```bash
cargo build --workspace
cargo run -p ferron-example-fixture -- run -c configs/ferron.conf
```

Validate configuration with example directives:

```bash
cargo run -p ferron-example-fixture -- validate -c configs/ferron.conf
cargo run -p ferron-example-fixture -- directives | jq .
```

## Modules

| Crate                         | Kind               | Directive                                                      |
| ----------------------------- | ------------------ | -------------------------------------------------------------- |
| `ferron-http-header-append`   | HTTP stage         | `example_header <value>`                                       |
| `ferron-http-hello`           | HTTP stage         | `hello_path`, `hello_message`                                  |
| `ferron-echo-server`          | Custom server      | `echo_server { listen <addr> }`                                |
| `ferron-observability-memory` | Observability sink | `observability { provider memory; memory { max_events <n> } }` |
| `ferron-tls-selfsigned`       | TLS provider       | `tls { provider selfsigned; selfsigned { days <n> } }`         |
| `ferron-dns-memory`           | DNS provider       | `dns memory` (ACME `dns memory`)                               |
| `ferron-config-toml`          | Config adapter     | `--config-adapter toml --config-params file=...`               |
| `ferron-logformat-csv`        | Log formatter      | `format csv` + `csv { fields ... }`                            |

See the `docs/` folder for detailed configuration documentation and each
`modules/*/src/lib.rs` for inline code comments.

## Documentation

Upstream Ferron module development guide lives in the main repository:

- https://github.com/ferronweb/ferron/blob/3.x/docs/module-development/ (or https://ferron.sh/docs/module-development)
- `cargo doc --no-deps` on `3.x` for `ferron-core` and `types/*`.

Local docs for this repo:

- `docs/overview.md` — what each example does
- `docs/directives.md` — configuration reference

## Testing

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# E2E (requires Docker)
docker build -f e2e/Dockerfile.test -t e2e-test-ferron-example:latest .
cd e2e && cargo test
```
