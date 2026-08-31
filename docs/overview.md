# Example modules overview

This document describes what each example crate does and which Ferron concepts
it demonstrates.

## HTTP stages

### `ferron-http-header-append`

A minimal `Stage<HttpContext>` that appends `X-Example-Header` in
`run_inverse`. It shows:

- `Stage::name`, `constraints`, `is_applicable`
- Reading `LayeredConfiguration` via `ctx.configuration.get_value`
- Per-request storage via `ctx.extensions` (TypeMap)
- Modifying responses in `run_inverse` for both `Custom` and `BuiltinError`

### `ferron-http-hello`

A stage that short-circuits the pipeline. It checks `ctx.req.uri().path()` and
if it matches `hello_path`, builds a `200 OK` response and returns `Ok(false)`.
It demonstrates `Ok(true)` vs `Ok(false)` vs `Err`.

## Custom server

### `ferron-echo-server`

A `Module` that spawns a TCP echo server on the secondary (tokio) runtime. It
shows:

- `ModuleLoader::register_modules` for creating `Module` instances
- `Module::start` with `runtime.spawn_secondary_task` / `spawn_primary_task`
- Reading global configuration in `register_modules`

## Observability sink

### `ferron-observability-memory`

An `EventSink` that stores events in a global `Mutex<Vec<Event>>`. It shows:

- `Provider<ObservabilityContext>` with `name() == "memory"`
- Scoped validator `observability.memory`
- `EventSink::processes_traces` / `processes_access`

## TLS provider

### `ferron-tls-selfsigned`

A `Provider<TlsContext>` that generates a self-signed certificate via `rcgen`
and returns a `TlsResolver`. It shows:

- Building `ServerConfig` via `ferron_tls::builder::build_server_config_builder`
- Caching per-host configs
- Scoped validator `tls.selfsigned`

## DNS provider

### `ferron-dns-memory`

A `Provider<DnsContext>` that sets `ctx.client` to an in-memory `DnsClient`.
It shows:

- `DnsClient::minimum_ttl`, `update_record`, `delete_record`
- Scoped validator `dns.memory`

## Configuration adapter

### `ferron-config-toml`

A `ConfigurationAdapter` named `"toml"` that parses a TOML file and produces a
`ServerConfiguration`. It shows:

- `adapt(params)` and `file_extension()`
- `ConfigurationWatcher` with `notify-debouncer-mini`
- `ConfigurationMetadata` for drift detection

## Log formatter

### `ferron-logformat-csv`

A `Provider<LogFormatterContext>` and `Provider<ApplicationLogFormatterContext>`
named `"csv"` that serializes access events as CSV. It shows:

- `LogFormatterContext::output`
- `AccessVisitor` to read access fields
- Scoped validators `logformat.csv` / `logformat_application.csv`
