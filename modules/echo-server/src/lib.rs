#![allow(clippy::arc_with_non_send_sync)]
//! Example Ferron custom server: `ferron-echo-server`
//!
//! This crate shows how to implement a **custom server** (not an HTTP stage).
//! Custom servers are [`Module`] implementations that are started via
//! [`Module::start`] and live for the lifetime of the process. They typically
//! spawn tasks on the Ferron runtime.
//!
//! ## What this module does
//!
//! - Reads `echo_server { listen <addr> }` from the global configuration.
//!   If the directive is absent, the server does not start.
//! - Spawns a task on the **secondary runtime** (tokio) that listens on the
//!   configured address and echoes every byte back to the client.
//!
//! ## Configuration example
//!
//! ```ferron
//! {
//!     echo_server {
//!         listen "127.0.0.1:9090"
//!     }
//! }
//!
//! example.com {
//!     root /var/www/html
//! }
//! ```
//!
//! ## Architecture notes for newcomers
//!
//! Ferron has a **dual runtime**:
//!
//! - **Primary runtime**: one zincio thread per CPU, pinned, optionally with
//!   `io_uring`. Use for high-throughput I/O (HTTP listeners, QUIC).
//! - **Secondary runtime**: standard tokio multi-thread. Use for background
//!   work and simple custom servers.
//!
//! This example uses the secondary runtime for simplicity. For a production
//! TCP server you would use `runtime.spawn_primary_task` with `zincio`.

use std::sync::Arc;

use ferron_core::config::validator::{ConfigurationValidationError, ConfigurationValidator};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::directives::{Directive, DirectiveRegistry, DirectiveSubblock};
use ferron_core::loader::ModuleLoader;
use ferron_core::runtime::Runtime;
use ferron_core::Module;

// ============================================================================
// Validator
// ============================================================================

/// Validates the `echo_server` global block.
///
/// Expected shape:
///
/// ```ferron
/// echo_server {
///     listen "127.0.0.1:9090"
/// }
/// ```
struct EchoServerValidator;

impl ConfigurationValidator for EchoServerValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        // `listen` is the only sub-directive we support. It takes one string.
        ferron_core::validate_directive!(
            config,
            ctx.used_directives,
            listen,
            optional args(1) => [
                ServerConfigurationValue::String(_, _) |
                ServerConfigurationValue::InterpolatedString(_, _)
            ],
            {}
        );
        // Also mark the outer directive as used if it appears as a block.
        // The outer `echo_server` is not a directive inside this block; it's the
        // block itself. Validation of the inner `listen` above is sufficient.
        Ok(())
    }
}

// ============================================================================
// Module: the long-lived server component
// ============================================================================

/// The running echo server.
///
/// `Module` instances are created in `register_modules` and started via
/// `Module::start`. They receive `&mut Runtime` and can spawn tasks.
pub struct EchoServerModule {
    /// Address to listen on, e.g. `"127.0.0.1:9090"`.
    listen_addr: String,
}

impl EchoServerModule {
    fn new(listen_addr: String) -> Self {
        Self { listen_addr }
    }
}

impl Module for EchoServerModule {
    fn name(&self) -> &str {
        "echo_server"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        let addr = self.listen_addr.clone();

        // Spawn on the secondary (tokio) runtime. This task lives as long as
        // the server runs. Ferron does not manage its lifecycle beyond
        // spawning; use `SHUTDOWN_TOKEN` if you need graceful shutdown.
        runtime.spawn_secondary_task(async move {
            // Bind inside the task so errors are logged, not fatal to startup.
            let listener = match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => {
                    ferron_core::log_info!("Echo server listening on {}", addr);
                    l
                }
                Err(e) => {
                    ferron_core::log_warn!("Echo server failed to bind {}: {e}", addr);
                    return;
                }
            };

            loop {
                let (mut socket, peer) = match listener.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        ferron_core::log_warn!("Echo server accept error: {e}");
                        continue;
                    }
                };
                ferron_core::log_debug!("Echo server accepted connection from {peer}");

                // Spawn a child task per connection.
                tokio::spawn(async move {
                    // Simple echo: copy bytes back to the client.
                    let (mut rd, mut wr) = socket.split();
                    let _ = tokio::io::copy(&mut rd, &mut wr).await;
                    ferron_core::log_debug!("Echo connection from {peer} closed");
                });
            }
        });

        Ok(())
    }
}

// ============================================================================
// Module loader
// ============================================================================

/// Loads the echo server module.
///
/// This demonstrates the `register_modules` hook, which runs after all
/// configuration adapters, validators, stages, and providers have been
/// registered. It is the place to read finalized configuration and create
/// `Module` instances.
#[derive(Default)]
pub struct EchoServerModuleLoader;

impl ModuleLoader for EchoServerModuleLoader {
    fn register_directives(&mut self, registry: &mut DirectiveRegistry) {
        // The outer block directive. It lives in the default subblock and
        // contains a nested `listen` directive.
        registry.register(
            Directive {
                name: "echo_server",
                usage: "echo_server { listen <addr> }",
                description: "Example custom server: TCP echo server. Listens on the given address and echoes bytes.",
                applicable_protocols: None,
                global_only: true,
                subblock_link: Some(DirectiveSubblock::custom("echo_server")),
            },
            DirectiveSubblock::default(),
        );
        registry.register(
            Directive {
                name: "listen",
                usage: "listen <addr>",
                description: "Address for the echo server to listen on (e.g. 127.0.0.1:9090).",
                applicable_protocols: None,
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("echo_server"),
        );
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        // We only validate the inner block, but we need a global validator that
        // looks for `echo_server` and validates its children. A simple approach
        // is to validate `listen` inside the top-level block when `echo_server`
        // exists as a nested directive.
        registry.push(Box::new(GlobalEchoValidator));
    }

    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // `config.global_config` holds top-level directives like `echo_server`.
        let Some(entry) = config
            .global_config
            .directives
            .get("echo_server")
            .and_then(|v| v.first())
        else {
            // No `echo_server` block -> nothing to do.
            return Ok(());
        };

        let Some(children) = entry.children.as_ref() else {
            ferron_core::log_warn!("echo_server block without children, ignoring");
            return Ok(());
        };

        // Extract `listen` from the nested block.
        let Some(listen_val) = children
            .directives
            .get("listen")
            .and_then(|v| v.first())
            .and_then(|e| e.args.first())
        else {
            ferron_core::log_warn!("echo_server block missing `listen` directive, ignoring");
            return Ok(());
        };

        let listen_addr = listen_val
            .as_string_with_interpolations(&std::collections::HashMap::new())
            .unwrap_or_else(|| "127.0.0.1:9090".to_string());

        ferron_core::log_info!("Registering echo server on {listen_addr}");
        modules.push(Arc::new(EchoServerModule::new(listen_addr)));

        Ok(())
    }
}

/// Global validator that checks the `echo_server { listen ... }` outer block.
struct GlobalEchoValidator;

impl ConfigurationValidator for GlobalEchoValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        // If `echo_server` is absent, nothing to validate.
        let Some(entries) = config.directives.get("echo_server") else {
            return Ok(());
        };
        ctx.used_directives.insert("echo_server".to_string());

        for entry in entries {
            // `echo_server` must be a block, not a flag.
            let Some(children) = entry.children.as_ref() else {
                return Err(ConfigurationValidationError::from(
                    "`echo_server` must be a block: echo_server { listen <addr> }".to_string(),
                )
                .with_span(entry.span.clone()));
            };
            // Validate the inner block with `EchoServerValidator`.
            let mut inner_ctx = ferron_core::config::validator::ConfigurationValidatorContext {
                used_directives: std::collections::HashSet::new(),
                is_global: false,
                scoped_validators: ctx.scoped_validators.clone(),
                diagnostics: Vec::new(),
                scope: ctx.scope.clone(),
            };
            EchoServerValidator.validate_block(children, &mut inner_ctx)?;
            // Check for unknown directives inside the block and add diagnostics
            // to inner_ctx before extending into the parent ctx.
            for (name, spans) in children.directives.iter() {
                if !inner_ctx.used_directives.contains(name) {
                    for entry in spans {
                        let diag = inner_ctx.create_diagnostic(
                            ferron_core::config::validator::ConfigurationValidatorDiagnosticKind::UnknownDirective,
                            format!("`{name}` is unused in the block"),
                            entry.span.clone().or(children.span.clone()),
                        );
                        inner_ctx.diagnostics.push(diag);
                    }
                }
            }
            ctx.diagnostics.extend(inner_ctx.diagnostics);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    #[test]
    fn validator_accepts_listen() {
        let mut inner = FxHashMap::default();
        inner.insert(
            "listen".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "127.0.0.1:9090".into(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        let block = ServerConfigurationBlock {
            directives: Arc::new(inner),
            matchers: FxHashMap::default(),
            span: None,
        };
        let v = EchoServerValidator;
        let mut ctx = ferron_core::config::validator::ConfigurationValidatorContext {
            used_directives: std::collections::HashSet::new(),
            is_global: false,
            scoped_validators: Arc::new(std::collections::HashMap::new()),
            diagnostics: Vec::new(),
            scope: None,
        };
        v.validate_block(&block, &mut ctx).unwrap();
    }
}
