# AGENTS.md — Ferron 3 Example Modules

This file guides AI coding agents that work on Ferron 3 example modules in this
repository (`https://github.com/ferronweb/ferron3-example-modules`).

## Project structure

- `modules/*`: example crates, each a standalone `ModuleLoader` library.
  - `http-header-append`, `http-hello`: HTTP stages (`Stage<HttpContext>`)
  - `echo-server`: custom TCP server (`Module`)
  - `observability-memory`: observability sink (`Provider<ObservabilityContext>`)
  - `tls-selfsigned`: TLS provider (`Provider<TlsContext>`)
  - `dns-memory`: DNS provider (`Provider<DnsContext>` / `DnsClient`)
  - `config-toml`: configuration adapter (`ConfigurationAdapter`)
  - `logformat-csv`: log formatter (`Provider<LogFormatterContext>`)
- `fixture/`: binary that wires all examples via `ferron-entrypoint` (excluded from workspace)
- `e2e/`: end-to-end tests via testcontainers + Dockerfile.test (excluded from workspace)
- `configs/`: example `ferron.conf` used by fixture and E2E
- `docs/`: user-facing documentation for the example modules

## Workspace

Root `Cargo.toml` is a workspace of `members = ["modules/*"]` and
`exclude = ["e2e", "fixture"]`. Run:

```bash
cargo build --workspace          # build example crates
cargo test --workspace           # unit tests
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

`e2e` and `fixture` are separate crates. Run them explicitly:

```bash
cargo test -p e2e                # needs Docker + protoc
cargo run -p ferron-example-fixture -- run -c configs/ferron.conf
```

## Fetching Ferron documentation

All Ferron module API lives in the upstream repository:

- Repo: `https://github.com/ferronweb/ferron`
- Branch: `3.x`
- User docs: `docs/` (including `docs/links.json` sidebar)
- Rust docs: `core/` and `types/*`

To generate local Rust docs for the API:

```bash
git clone https://github.com/ferronweb/ferron -b 3.x /tmp/ferron
cd /tmp/ferron
cargo doc --no-deps -p ferron-core -p ferron-http -p ferron-observability -p ferron-tls -p ferron-dns
# open target/doc/ferron_core/index.html
```

Or read docs directly on GitHub:

- `https://github.com/ferronweb/ferron/blob/3.x/core/src/loader.rs` — `ModuleLoader` trait
- `https://github.com/ferronweb/ferron/blob/3.x/core/src/pipeline.rs` — `Stage` trait and `Pipeline`
- `https://github.com/ferronweb/ferron/blob/3.x/core/src/registry.rs` — `Registry`, `StageConstraint`
- `https://github.com/ferronweb/ferron/blob/3.x/core/src/providers.rs` — `Provider` trait
- `https://github.com/ferronweb/ferron/blob/3.x/core/src/config/adapter.rs` — `ConfigurationAdapter`
- `https://github.com/ferronweb/ferron/blob/3.x/core/src/runtime.rs` — dual runtime (zincio + tokio)
- `https://github.com/ferronweb/ferron/blob/3.x/docs/module-development/` — module development guide (in upstream docs)

## Conventions for this repo

- Each crate has inline code comments that explain the Ferron concepts used.
- Naming follows upstream conventions: `ferron-http-*`, `ferron-observability-*`, `ferron-tls-*`, `ferron-dns-*`, `ferron-config-*`, `ferron-logformat-*`, `ferron-*-server`.
- Directives are lower-snake_case, module crates are kebab-case. The provider name returned by `Provider::name()` is the value users write as `provider <name>`.
- Stages declare `Before`/`After` constraints relative to built-in stages (e.g. `reverse_proxy`, `static_file`). Do not create ordering cycles.
- Validators must mark directives as used via `validate_directive!` or `ctx.used_directives.insert(...)`. Otherwise the validator reports `UnknownDirective`.

## Building a custom Ferron binary

See `fixture/src/main.rs` for the pattern:

```rust
ferron_entrypoint::init();
let mut profile = ferron_entrypoint::default_profile();
profile.push(Box::new(your_loader));
ferron_entrypoint::main(profile);
```

`Cargo.toml` for a custom binary depends on `ferron-entrypoint` via git:

```toml
[dependencies]
ferron-entrypoint = { git = "https://github.com/ferronweb/ferron", branch = "3.x", features = ["profile-default"] }
ferron-http-hello = { path = "../modules/http-hello" }
```

For a minimal binary, disable `profile-default` and list only required modules.

## Validation

Before submitting changes, run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo shear --help 2>/dev/null && cargo shear # unused deps check (optional)
```

For E2E, build the image first (requires Docker):

```bash
docker build -f e2e/Dockerfile.test -t e2e-test-ferron-example:latest .
cd e2e && cargo test
```
