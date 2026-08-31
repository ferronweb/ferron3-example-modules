#![allow(clippy::arc_with_non_send_sync)]
//! Example Ferron HTTP stage: `ferron-http-header-append`
//!
//! This crate shows how to write a minimal HTTP stage that appends a
//! configurable response header. It is intended as a learning example for
//! newcomers to Ferron module development.
//!
//! ## What this module does
//!
//! - Reads a single directive `example_header <value>` from the host or
//!   global configuration.
//! - Stores the header value in the request extensions during `run`.
//! - Appends the header `X-Example-Header: <value>` to every response in
//!   `run_inverse`. Using `run_inverse` makes sure the header is added even
//!   when a later stage produces a custom response or error page.
//!
//! ## Configuration example
//!
//! ```ferron
//! example.com {
//!     // Append X-Example-Header: hello-world to every response for this host
//!     example_header hello-world
//! }
//! ```
//!
//! ## Module wiring
//!
//! Ferron loads modules through the [`ModuleLoader`] trait. Each method has a
//! default no-op implementation, so you only override what you need.
//!
//! ```text
//! ModuleLoader::register_directives          -> advertise config directives
//! ModuleLoader::register_*_validators        -> validate config blocks
//! ModuleLoader::register_stages              -> register pipeline stages
//! ```
//!
//! Stages are ordered by [`StageConstraint::Before`] / [`StageConstraint::After`].
//! The registry performs a topological sort (Kahn's algorithm) and panics on cycles.

use std::sync::Arc;

use async_trait::async_trait;
use ferron_core::config::validator::{ConfigurationValidationError, ConfigurationValidator};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::directives::{Directive, DirectiveRegistry, DirectiveSubblock};
use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::{RegistryBuilder, StageConstraint};
use ferron_http::HttpContext;

// ============================================================================
// Configuration validator
// ============================================================================

/// Validates the `example_header` directive.
///
/// The directive accepts one string argument: the value to append as
/// `X-Example-Header`. It is optional and defaults to `"example"`.
struct ExampleHeaderValidator;

impl ConfigurationValidator for ExampleHeaderValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        // `validate_directive!` is a helper macro exported by `ferron-core`.
        // It checks directive arity and argument types, and marks the directive
        // as "used" so the global validator can detect unknown directives.
        ferron_core::validate_directive!(
            config,
            ctx.used_directives,
            example_header,
            optional args(1) => [
                ServerConfigurationValue::String(_, _) |
                ServerConfigurationValue::InterpolatedString(_, _)
            ],
            {}
        );
        Ok(())
    }
}

// ============================================================================
// Pipeline context extension
// ============================================================================

/// Per-request storage for the header value.
///
/// Stages cannot keep per-request state in `self` (they are shared across
/// threads). Instead they store typed values in `ctx.extensions`, which is a
/// `TypeMap`. The value is inserted in `run` and removed in `run_inverse`.
struct ExampleHeaderCtx(String);

impl typemap_rev::TypeMapKey for ExampleHeaderCtx {
    type Value = ExampleHeaderCtx;
}

// ============================================================================
// Pipeline stage
// ============================================================================

/// Stage that appends `X-Example-Header` to responses.
///
/// This is a minimal stage. It:
///
/// 1. Reads `example_header` from the layered configuration (host + location +
///    global). `LayeredConfiguration` merges parent and child blocks, so a
///    location block can override the host value.
/// 2. Stores the value in `ctx.extensions`.
/// 3. In `run_inverse`, reads the stored value and appends the header to the
///    response (whether it is a `Custom` response or a `BuiltinError` with
///    header overrides).
struct ExampleHeaderStage;

#[async_trait(?Send)]
impl Stage<HttpContext> for ExampleHeaderStage {
    fn name(&self) -> &str {
        // The name is used for `StageConstraint::Before` / `After` ordering.
        // It must be unique within a single `HttpContext` pipeline.
        "example_header"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        // Run after the built-in routing stages but before the response is
        // finalized. `reverse_proxy` is a common late stage, so we run before it.
        vec![StageConstraint::Before("reverse_proxy".to_string())]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        // Called once per stage when building a pipeline with
        // `StageRegistry::build_with_config`. If no host block uses
        // `example_header`, we skip this stage entirely (performance).
        config.is_some_and(|c| c.has_directive("example_header"))
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        // Read the directive from the layered configuration. `get_value` looks
        // up the directive by name and interpolates variables if needed.
        let value = ctx
            .configuration
            .get_value("example_header", true)
            .and_then(|v| v.as_string_with_interpolations(ctx))
            .unwrap_or_else(|| "example".to_string());

        // Store in extensions for `run_inverse`.
        ctx.extensions
            .insert::<ExampleHeaderCtx>(ExampleHeaderCtx(value));
        // `Ok(true)` means continue to the next stage.
        Ok(true)
    }

    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        // Retrieve and remove the stored value.
        let Some(ExampleHeaderCtx(value)) = ctx.extensions.remove::<ExampleHeaderCtx>() else {
            return Ok(());
        };

        // Header values must be valid HTTP header values.
        let header_value = http::HeaderValue::from_str(&value)
            .map_err(|e| PipelineError::Custom(format!("invalid example_header value: {e}")))?;

        // There are two response variants:
        // - `Custom` holds a full `http::Response`.
        // - `BuiltinError` holds a status code and optional header overrides.
        // We handle both, and if no response exists yet, we create a 404 stub
        // so headers still have a place to live (similar to the built-in headers module).
        if ctx.res.is_none() {
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(404, None));
        }

        match ctx.res.as_mut() {
            Some(ferron_http::HttpResponse::Custom(res)) => {
                res.headers_mut().append("x-example-header", header_value);
            }
            Some(ferron_http::HttpResponse::BuiltinError(_, headers)) => {
                let headers = headers.get_or_insert_with(http::HeaderMap::new);
                headers.append("x-example-header", header_value);
            }
            Some(ferron_http::HttpResponse::Abort) | None => {}
        }

        Ok(())
    }
}

// ============================================================================
// Module loader
// ============================================================================

/// [`ModuleLoader`] for the example header module.
///
/// This is the entry point that Ferron calls during startup. The loader
/// registers directives, validators, and stages. All methods have default
/// no-op impls, so we only override the three we need.
#[derive(Default)]
pub struct ExampleHeaderModuleLoader;

impl ModuleLoader for ExampleHeaderModuleLoader {
    fn register_directives(&mut self, registry: &mut DirectiveRegistry) {
        registry.register(
            Directive {
                name: "example_header",
                usage: "example_header <value>",
                description: "Example directive: appends X-Example-Header with the given value to every response.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::default(),
        );
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(ExampleHeaderValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            &'static str,
            Vec<Box<dyn ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(ExampleHeaderValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(ExampleHeaderStage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use ferron_http::HttpRequest;
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use http_body_util::{BodyExt, Empty};
    use rustc_hash::FxHashMap;

    fn make_context(header: Option<&str>) -> HttpContext {
        let req: HttpRequest = Request::builder()
            .uri("/")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();
        let mut ctx = HttpContext::default();
        ctx.req = Some(req);
        ctx.events = CompositeEventSink::new(Vec::new());
        if let Some(val) = header {
            let mut directives = FxHashMap::default();
            directives.insert(
                "example_header".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String(val.to_string(), None)],
                    children: None,
                    span: None,
                }],
            );
            let mut layered = LayeredConfiguration::new();
            layered.add_layer(Arc::new(ServerConfigurationBlock {
                directives: Arc::new(directives),
                matchers: FxHashMap::default(),
                span: None,
            }));
            ctx.configuration = layered;
        }
        ctx
    }

    #[tokio::test]
    async fn appends_header() {
        let stage = ExampleHeaderStage;
        let mut ctx = make_context(Some("hello"));
        stage.run(&mut ctx).await.unwrap();
        stage.run_inverse(&mut ctx).await.unwrap();
        match ctx.res.unwrap() {
            ferron_http::HttpResponse::BuiltinError(_, Some(headers)) => {
                assert_eq!(headers.get("x-example-header").unwrap(), "hello");
            }
            _ => panic!("expected BuiltinError with headers"),
        }
    }
}
