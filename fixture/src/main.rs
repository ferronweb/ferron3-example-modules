//! Fixture binary for manual testing of Ferron 3 example modules.
//!
//! This binary is similar to `bin/src/main.rs` in the main Ferron repository,
//! but it registers the example modules from this workspace alongside the
//! default profile.
//!
//! Run it with:
//!
//! ```bash
//! cargo run -p ferron-example-fixture -- run -c configs/ferron.conf
//! ```
//!
//! Or build a release binary and package it via the `just` commands in the
//! upstream Ferron repository (see `docs/packaging.md` in the main repo).

fn main() {
    // Initialize the global allocator and panic hook. This must be called
    // before `ferron_entrypoint::main`.
    ferron_entrypoint::init();

    // Start with the default set of modules (all built-in Ferron features).
    // This includes HTTP server, static files, proxy, config adapters, TLS,
    // observability, etc.
    let mut profile = ferron_entrypoint::default_profile();

    // Register each example module. Each module exposes a `*ModuleLoader`
    // struct that implements `ferron_core::loader::ModuleLoader`.
    //
    // The order in which loaders are pushed does not matter: stages are
    // ordered by `StageConstraint`, and providers are discovered by type and name
    // at runtime. You can push them in any order.
    profile.push(Box::new(
        ferron_http_header_append::ExampleHeaderModuleLoader,
    ));
    profile.push(Box::new(ferron_http_hello::HelloModuleLoader));
    profile.push(Box::new(ferron_echo_server::EchoServerModuleLoader::default()));
    profile.push(Box::new(
        ferron_observability_memory::MemoryObservabilityModuleLoader,
    ));
    profile.push(Box::new(
        ferron_tls_selfsigned::SelfSignedTlsModuleLoader::default(),
    ));
    profile.push(Box::new(ferron_dns_memory::MemoryDnsModuleLoader::default()));
    profile.push(Box::new(ferron_config_toml::TomlConfigModuleLoader::default()));
    profile.push(Box::new(
        ferron_logformat_csv::CsvLogFormatterModuleLoader::default(),
    ));

    // Hand control to the entrypoint. It parses CLI args (`run`, `validate`,
    // `adapt`, `directives`, ...), loads configuration, validates it via all
    // registered validators, and starts the lifecycle for every `Module`.
    ferron_entrypoint::main(profile);
}
