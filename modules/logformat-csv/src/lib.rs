#![allow(clippy::arc_with_non_send_sync)]

//! Example Ferron log formatter: `ferron-logformat-csv`
//!
//! This crate shows how to write a **log formatter** (a provider for
//! `LogFormatterContext` / `ApplicationLogFormatterContext`). Formatters are
//! used by observability sinks to serialize access events into strings.
//!
//! ## What this module does
//!
//! - Provides a formatter named `"csv"` that serializes access log fields as a
//!   CSV line: `timestamp,method,path,status,bytes`.
//! - Reads `fields <name>...` from the observability config to allow users to
//!   choose which columns appear.
//! - Demonstrates both `LogFormatterContext` (access logs) and
//!   `ApplicationLogFormatterContext` (application logs).
//!
//! ## Configuration example
//!
//! ```ferron
//! example.com {
//!     observability {
//!         provider console
//!         console {
//!             format csv
//!             csv {
//!                 fields method path status
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! The `format csv` directive selects this formatter, and `csv { fields ... }`
//! customizes its output.

use std::sync::Arc;

use ferron_core::config::validator::{ConfigurationValidationError, ConfigurationValidator};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::config_validator_scoped_key;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_observability::{
    AccessVisitor, ApplicationLogFormatterContext, LogAttributeValue, LogFormatterContext,
};

// ============================================================================
// Helpers
// ============================================================================

/// Parse the `fields` list from the log config block.
///
/// `fields` is an optional directive that takes multiple string arguments.
/// If absent, we return an empty vec meaning "all fields" (handled by the
/// caller).
fn parse_fields(log_config: &ferron_core::config::ServerConfigurationBlock) -> Vec<String> {
    log_config
        .directives
        .get("fields")
        .map(|entries| {
            entries
                .iter()
                .flat_map(|e| e.args.iter())
                .filter_map(|arg| {
                    arg.as_string_with_interpolations(&std::collections::HashMap::new())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Escape a CSV field according to RFC 4180: wrap in double quotes if it
/// contains a comma, quote, or newline, and double any internal quotes.
fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

// ============================================================================
// Access log visitor that collects fields into a map
// ============================================================================

/// Visitor that records access fields into a `HashMap` for later CSV rendering.
struct CsvVisitor {
    /// Ordered map of field name -> string value as visited.
    fields: Vec<(String, String)>,
    /// If non-empty, only these fields are kept.
    enabled: Vec<String>,
}

impl CsvVisitor {
    fn is_enabled(&self, name: &str) -> bool {
        self.enabled.is_empty() || self.enabled.iter().any(|f| f == name)
    }
}

impl AccessVisitor for CsvVisitor {
    fn field_string(&mut self, name: &str, value: &str) {
        if self.is_enabled(name) {
            self.fields.push((name.to_string(), value.to_string()));
        }
    }

    fn field_u64(&mut self, name: &str, value: u64) {
        if self.is_enabled(name) {
            self.fields.push((name.to_string(), value.to_string()));
        }
    }

    fn field_f64(&mut self, name: &str, value: f64) {
        if self.is_enabled(name) {
            self.fields.push((name.to_string(), value.to_string()));
        }
    }

    fn field_bool(&mut self, name: &str, value: bool) {
        if self.is_enabled(name) {
            self.fields.push((name.to_string(), value.to_string()));
        }
    }
}

// ============================================================================
// Formatters
// ============================================================================

/// Formatter for access events (request logs) as CSV.
struct CsvAccessFormatter;

impl Provider<LogFormatterContext> for CsvAccessFormatter {
    fn name(&self) -> &str {
        "csv"
    }

    fn execute(&self, ctx: &mut LogFormatterContext) -> Result<(), Box<dyn std::error::Error>> {
        // `ctx.log_config` is the `csv { ... }` block.
        let enabled_fields = parse_fields(&ctx.log_config);
        let mut visitor = CsvVisitor {
            fields: Vec::new(),
            enabled: enabled_fields,
        };
        // `visit` calls the `AccessVisitor` methods for each field in the event.
        ctx.access_event.visit(&mut visitor);

        // If no fields were visited (e.g. unknown `fields` list), produce an empty line.
        // Otherwise, join the values with commas, escaping as needed.
        if visitor.fields.is_empty() {
            ctx.output = Some(String::new());
        } else {
            // Preserve insertion order from `visit`. The order is the field
            // declaration order in the access event type, which is stable.
            let line = visitor
                .fields
                .iter()
                .map(|(_, v)| csv_escape(v))
                .collect::<Vec<_>>()
                .join(",");
            ctx.output = Some(line);
        }
        Ok(())
    }
}

/// Formatter for application logs as CSV (target, level, message, fields).
struct CsvApplicationFormatter;

impl Provider<ApplicationLogFormatterContext<'static>> for CsvApplicationFormatter {
    fn name(&self) -> &str {
        "csv"
    }

    fn execute(
        &self,
        ctx: &mut ApplicationLogFormatterContext<'static>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // For application logs we produce a simple CSV line with a few fixed
        // columns. A production formatter would handle quoting more carefully.
        let level = match ctx.log_event.level {
            ferron_observability::LogLevel::Error => "ERROR",
            ferron_observability::LogLevel::Warn => "WARN",
            ferron_observability::LogLevel::Info => "INFO",
            ferron_observability::LogLevel::Debug => "DEBUG",
        };
        // Collect attribute fields as `k=v` pairs.
        let attrs = ctx
            .log_event
            .attributes
            .iter()
            .map(|(k, v)| {
                let val = match v {
                    LogAttributeValue::Bool(b) => b.to_string(),
                    LogAttributeValue::String(s) => s.clone(),
                    LogAttributeValue::StaticStr(s) => s.to_string(),
                    LogAttributeValue::I64(i) => i.to_string(),
                    LogAttributeValue::F64(f) => f.to_string(),
                };
                format!("{}={}", csv_escape(k), csv_escape(&val))
            })
            .collect::<Vec<_>>()
            .join(";");

        let line = format!(
            "{},{},{},{},{}",
            csv_escape(ctx.log_event.target),
            csv_escape(level),
            csv_escape(&ctx.log_event.summary),
            csv_escape(&ctx.log_event.message),
            attrs
        );
        ctx.output = Some(line);
        Ok(())
    }
}

// ============================================================================
// Validator
// ============================================================================

struct CsvValidator;

impl ConfigurationValidator for CsvValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        ferron_core::validate_directive!(
            config,
            ctx.used_directives,
            fields,
            optional args(*) => [
                ServerConfigurationValue::String(_, _) |
                ServerConfigurationValue::InterpolatedString(_, _)
            ],
            {}
        );
        Ok(())
    }
}

// ============================================================================
// Loader
// ============================================================================

/// Loader that registers the CSV formatters.
///
/// Formatters are providers for `LogFormatterContext` (access) and
/// `ApplicationLogFormatterContext` (application). They are selected by the
/// `format <name>` directive inside an observability backend block.
///
/// Example:
///
/// ```ferron
/// observability {
///     provider console
///     console {
///         format csv
///         csv { fields method path status }
///     }
/// }
/// ```
#[derive(Default)]
pub struct CsvLogFormatterModuleLoader;

impl ModuleLoader for CsvLogFormatterModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry
            .with_provider::<LogFormatterContext, _>(|| Arc::new(CsvAccessFormatter))
            .with_provider::<ApplicationLogFormatterContext<'static>, _>(|| {
                Arc::new(CsvApplicationFormatter)
            })
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ConfigurationValidator>,
        >,
    ) {
        // Log format validators are scoped to `logformat` and `logformat_application`.
        // The provider name (`csv`) matches `format csv`.
        registry.insert(
            config_validator_scoped_key!("logformat", "csv"),
            Box::new(CsvValidator),
        );
        registry.insert(
            config_validator_scoped_key!("logformat_application", "csv"),
            Box::new(CsvValidator),
        );
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
    fn csv_escape_quotes() {
        assert_eq!(csv_escape("hello, world"), "\"hello, world\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_escape("plain"), "plain");
    }

    #[test]
    fn validator_accepts_fields() {
        let mut directives = FxHashMap::default();
        directives.insert(
            "fields".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![
                    ServerConfigurationValue::String("method".into(), None),
                    ServerConfigurationValue::String("path".into(), None),
                ],
                children: None,
                span: None,
            }],
        );
        let block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: FxHashMap::default(),
            span: None,
        };
        let v = CsvValidator;
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
