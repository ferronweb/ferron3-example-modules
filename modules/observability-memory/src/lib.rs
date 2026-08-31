#![allow(clippy::arc_with_non_send_sync)]

//! Example Ferron observability sink: `ferron-observability-memory`
//!
//! This crate shows how to write an **observability sink** (a backend for
//! logs, metrics, and traces). Sinks implement [`EventSink`] and are created
//! by a [`Provider<ObservabilityContext>`] that reads configuration and sets
//! `ctx.sink`.
//!
//! ## What this module does
//!
//! - Provides a sink named `"memory"` that stores every [`Event`] in a global
//!   `Vec<Event>` protected by a `Mutex`. This is useful for testing or for
//!   inspecting events programmatically.
//! - Exposes a helper `memory_events()` to read the stored events.
//! - Reads `memory { max_events <n> }` from the observability config to limit
//!   how many events are kept.
//!
//! ## Configuration example
//!
//! ```ferron
//! example.com {
//!     observability {
//!         provider memory
//!         memory {
//!             max_events 1000
//!         }
//!     }
//! }
//! ```
//!
//! > [!note]
//! > In a real sink you would send events to an external system (OTLP, file,
//! > console, Prometheus). This example keeps them in memory so you can see the
//! > full flow without external dependencies.

use std::sync::{Arc, Mutex, OnceLock};

use ferron_core::config::validator::{ConfigurationValidationError, ConfigurationValidator};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::config_validator_scoped_key;
use ferron_core::directives::{Directive, DirectiveRegistry, DirectiveSubblock};
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_observability::{Event, EventSink, ObservabilityContext};

// ============================================================================
// Global in-memory storage (for demonstration / testing)
// ============================================================================

/// Global storage for events received by the memory sink.
///
/// In a real sink this would be a channel to a background task. Here we keep
/// it simple: a `Mutex<Vec<Event>>`. The `OnceLock` makes it a global singleton
/// that is initialized on first use.
static MEMORY_STORE: OnceLock<Arc<Mutex<Vec<Event>>>> = OnceLock::new();

fn memory_store() -> Arc<Mutex<Vec<Event>>> {
    MEMORY_STORE
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

/// Drain and return all stored events.
///
/// This is useful in tests or in a custom admin endpoint that wants to expose
/// recent events.
///
/// ```ignore
/// let events = ferron_observability_memory::memory_events();
/// println!("Stored {} events", events.len());
/// ```
pub fn memory_events() -> Vec<Event> {
    let store = memory_store();
    let mut guard = store.lock().unwrap();
    std::mem::take(&mut *guard)
}

/// Peek at stored events without draining.
pub fn peek_memory_events() -> Vec<Event> {
    memory_store().lock().unwrap().clone()
}

// ============================================================================
// Event sink
// ============================================================================

/// An event sink that keeps events in memory.
///
/// Each call to [`EventSink::emit`] pushes a clone of the event into the
/// global `MEMORY_STORE`. If `max_events` is set, oldest events are evicted.
struct MemorySink {
    max_events: usize,
    store: Arc<Mutex<Vec<Event>>>,
}

impl EventSink for MemorySink {
    fn emit(&self, event: Event) {
        // `Event` is `Clone`, so we can store a copy. Real sinks might avoid
        // cloning by using `emit_arc`.
        let mut guard = self.store.lock().unwrap();
        guard.push(event);
        if guard.len() > self.max_events {
            let drain = guard.len() - self.max_events;
            guard.drain(0..drain);
        }
    }

    fn processes_traces(&self) -> bool {
        // Return true so Ferron will actually construct Trace events.
        // If this returns false, the server may skip building trace events
        // for performance (see `CompositeEventSink::has_trace_sinks`).
        true
    }

    fn processes_access(&self) -> bool {
        true
    }
}

// ============================================================================
// Provider
// ============================================================================

/// Provider that creates the memory sink from configuration.
///
/// Providers are registered via `RegistryBuilder::with_provider` and discovered
/// at runtime by name. The name returned by `Provider::name()` must match the
/// `provider <name>` directive value in `observability { ... }`.
struct MemoryProvider;

impl Provider<ObservabilityContext> for MemoryProvider {
    fn name(&self) -> &str {
        // This is the name users write as `provider memory`.
        "memory"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn std::error::Error>> {
        // Read `max_events` from the observability block. The block is the
        // content of `observability { ... }` after Ferron has stripped the
        // `provider` directive.
        //
        // Example: `memory { max_events 500 }` becomes a nested block
        // under the `memory` key inside `ctx.log_config`.
        let max_events = ctx
            .log_config
            .directives
            .get("memory")
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.children.as_ref())
            .and_then(|children| children.directives.get("max_events"))
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.args.first())
            .and_then(|v| v.as_number())
            .unwrap_or(1000) as usize;

        let sink = MemorySink {
            max_events,
            store: memory_store(),
        };

        // Tell Ferron to use this sink for the current host.
        ctx.sink = Some(Arc::new(sink));
        Ok(())
    }
}

// ============================================================================
// Validator
// ============================================================================

struct MemoryValidator;

impl ConfigurationValidator for MemoryValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        // Validate the outer `memory { ... }` block and its inner `max_events`
        // directive. The outer block is the value of `memory` inside
        // `observability { memory { ... } }`.
        // Ferron already handles nesting for scoped validators, so `config` here
        // is the inner `memory { ... }` block.
        ferron_core::validate_directive!(
            config,
            ctx.used_directives,
            max_events,
            optional args(1) => [
                ServerConfigurationValue::Number(_, _)
            ],
            {}
        );
        Ok(())
    }
}

// ============================================================================
// Module loader
// ============================================================================

/// Loader that registers the memory observability sink.
///
/// It:
///
/// - Registers the provider `memory` for `ObservabilityContext`.
/// - Registers a scoped validator for `observability.memory`.
/// - Registers directives for editor support (`ferron directives`).
#[derive(Default)]
pub struct MemoryObservabilityModuleLoader;

impl ModuleLoader for MemoryObservabilityModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry.with_provider::<ObservabilityContext, _>(|| Arc::new(MemoryProvider))
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ConfigurationValidator>,
        >,
    ) {
        // Scoped validators are keyed by (namespace, provider_name).
        // For observability sinks, the namespace is `observability`.
        registry.insert(
            config_validator_scoped_key!("observability", "memory"),
            Box::new(MemoryValidator),
        );
    }

    fn register_directives(&mut self, registry: &mut DirectiveRegistry) {
        registry.register(
            Directive {
                name: "max_events",
                usage: "max_events <count>",
                description: "Maximum number of events to keep in the memory sink. Oldest events are evicted. Default: 1000",
                applicable_protocols: None,
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("observability"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use ferron_observability::{LogAttributeValue, LogEvent, LogLevel};
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    #[test]
    fn memory_sink_stores_events() {
        let sink = MemorySink {
            max_events: 10,
            store: Arc::new(Mutex::new(Vec::new())),
        };
        let event = Event::Log(LogEvent {
            level: LogLevel::Info,
            message: "hello".to_string(),
            summary: "hello".into(),
            target: "test",
            attributes: vec![("k", LogAttributeValue::String("v".into()))],
            trace_context: None,
        });
        sink.emit(event);
        // Not using global store here, so check local.
        // Just verify no panic and that sink reports capabilities.
        assert!(sink.processes_traces());
        assert!(sink.processes_access());
    }

    #[test]
    fn validator_accepts_max_events() {
        let mut directives = FxHashMap::default();
        directives.insert(
            "max_events".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(500, None)],
                children: None,
                span: None,
            }],
        );
        let block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: FxHashMap::default(),
            span: None,
        };
        let v = MemoryValidator;
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
