#![allow(clippy::arc_with_non_send_sync)]
//! Example Ferron HTTP stage: `ferron-http-hello`
//!
//! This crate shows how to write a stage that **short-circuits** the pipeline
//! by producing a response and returning `Ok(false)`. It responds with a plain
//! text greeting for requests that match a configurable path.
//!
//! ## What this module does
//!
//! - Reads `hello_path <path>` (default `"/hello"`) and `hello_message <text>`
//!   (default `"Hello from Ferron!"`) from configuration.
//! - In `run`, checks if the request path matches `hello_path`. If it does, it
//!   builds a `200 OK` response with the greeting and returns `Ok(false)` to
//!   stop the pipeline. No later stage runs, but `run_inverse` still runs for
//!   stages that already executed.
//! - If the path does not match, returns `Ok(true)` to continue.
//!
//! ## Configuration example
//!
//! ```ferron
//! example.com {
//!     hello_path /greet
//!     hello_message "Greetings, traveler!"
//! }
//! ```
//!
//! Requesting `GET /greet` returns `Greetings, traveler!`.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::config::validator::{ConfigurationValidationError, ConfigurationValidator};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::directives::{Directive, DirectiveRegistry, DirectiveSubblock};
use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::{RegistryBuilder, StageConstraint};
use ferron_http::{HttpContext, HttpResponse};
use http::StatusCode;
use http_body_util::{BodyExt, Full};

// ============================================================================
// Validator
// ============================================================================

struct HelloValidator;

impl ConfigurationValidator for HelloValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        ferron_core::validate_directive!(
            config,
            ctx.used_directives,
            hello_path,
            optional args(1) => [
                ServerConfigurationValue::String(_, _) |
                ServerConfigurationValue::InterpolatedString(_, _)
            ],
            {}
        );
        ferron_core::validate_directive!(
            config,
            ctx.used_directives,
            hello_message,
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
// Stage
// ============================================================================

/// Stage that serves a greeting for a configurable path.
///
/// This stage demonstrates:
///
/// - How to read configuration via `LayeredConfiguration`.
/// - How to inspect the request (`ctx.req`).
/// - How to produce a response via `ctx.res` and short-circuit with `Ok(false)`.
/// - Ordering constraints via `constraints()`.
struct HelloStage;

#[async_trait(?Send)]
impl Stage<HttpContext> for HelloStage {
    fn name(&self) -> &str {
        "hello"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        // Run early, after routing but before static file serving or proxy.
        // `client_ip_from_header` and `https_redirect` are very early stages,
        // so we run after them and before the handler stages.
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
            StageConstraint::After("https_redirect".to_string()),
            StageConstraint::Before("static_file".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        // Only include this stage if the config mentions one of our directives.
        // This keeps the default pipeline lean when the example is not used.
        config.is_some_and(|c| c.has_directive("hello_path") || c.has_directive("hello_message"))
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        // Read configuration values. `get_value` returns the first directive entry
        // for the name, merged across global/host/location layers.
        let hello_path = ctx
            .configuration
            .get_value("hello_path", true)
            .and_then(|v| v.as_string_with_interpolations(ctx))
            .unwrap_or_else(|| "/hello".to_string());

        let hello_message = ctx
            .configuration
            .get_value("hello_message", true)
            .and_then(|v| v.as_string_with_interpolations(ctx))
            .unwrap_or_else(|| "Hello from Ferron!".to_string());

        // `ctx.req` is `Option<HttpRequest>`. It is `None` only in synthetic
        // contexts (e.g. error page rendering). If missing, let the pipeline continue.
        let Some(req) = ctx.req.as_ref() else {
            return Ok(true);
        };

        // Compare the request path (without query string) to the configured path.
        // Ferron strips the matched `location` prefix before stages run, so this
        // check sees the post-location path.
        let path = req.uri().path();
        if path != hello_path {
            return Ok(true); // Not our path -> continue pipeline
        }

        // Build a 200 OK response. Ferron uses `UnsyncBoxBody<Bytes, io::Error>`
        // as the body type. `Full` is a simple in-memory body.
        let body = Full::new(Bytes::from(hello_message))
            .map_err(|e| match e {})
            .boxed_unsync();

        let response = http::Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(body)
            .map_err(|e| PipelineError::Custom(e.to_string()))?;

        // Store the response and stop the forward pipeline.
        // `Ok(false)` means "graceful stop, no error" -- remaining forward stages
        // are skipped, but `run_inverse` is still called for stages that already ran.
        ctx.res = Some(HttpResponse::Custom(response));
        Ok(false)
    }
}

// ============================================================================
// Module loader
// ============================================================================

#[derive(Default)]
pub struct HelloModuleLoader;

impl ModuleLoader for HelloModuleLoader {
    fn register_directives(&mut self, registry: &mut DirectiveRegistry) {
        registry
            .register(
                Directive {
                    name: "hello_path",
                    usage: "hello_path <path>",
                    description: "Example directive: path that returns a hello message. Default: /hello",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "hello_message",
                    usage: "hello_message <text>",
                    description: "Example directive: message returned for hello_path. Default: Hello from Ferron!",
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
        registry.push(Box::new(HelloValidator));
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
            .push(Box::new(HelloValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(HelloStage))
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

    fn make_ctx(path: &str, hello_path: Option<&str>) -> HttpContext {
        let req: HttpRequest = Request::builder()
            .uri(path)
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();
        let mut ctx = HttpContext::default();
        ctx.req = Some(req);
        ctx.events = CompositeEventSink::new(Vec::new());
        if let Some(p) = hello_path {
            let mut directives = FxHashMap::default();
            directives.insert(
                "hello_path".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String(p.to_string(), None)],
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
    async fn matches_hello_path() {
        let stage = HelloStage;
        let mut ctx = make_ctx("/hello", None);
        let cont = stage.run(&mut ctx).await.unwrap();
        assert!(!cont); // stopped pipeline
        assert!(ctx.res.is_some());
    }

    #[tokio::test]
    async fn non_matching_path_continues() {
        let stage = HelloStage;
        let mut ctx = make_ctx("/other", None);
        let cont = stage.run(&mut ctx).await.unwrap();
        assert!(cont);
        assert!(ctx.res.is_none());
    }
}
