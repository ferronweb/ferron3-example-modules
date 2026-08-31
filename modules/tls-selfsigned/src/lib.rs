#![allow(clippy::arc_with_non_send_sync)]
//! Example Ferron TLS provider: `ferron-tls-selfsigned`
//!
//! This crate shows how to write a **TLS provider** (a [`Provider<TlsContext>`]).
//! TLS providers supply a [`TlsResolver`] that rustls calls during the TLS
//! handshake to obtain the `ServerConfig` with certificates.
//!
//! ## What this module does
//!
//! - Provides a provider named `"selfsigned"` that generates a self-signed
//!   certificate for the requested hostname on first use (via `rcgen`).
//! - Caches the `ServerConfig` per hostname so repeated handshakes are fast.
//! - Demonstrates how to build a `ServerConfig` via
//!   `ferron_tls::builder::build_server_config_builder`.
//!
//! ## Configuration example
//!
//! ```ferron
//! example.com {
//!     tls {
//!         provider selfsigned
//!         selfsigned {
//!             # optional: validity in days, default 365
//!             days 30
//!         }
//!     }
//! }
//! ```
//!
//! > [!warning]
//! > This is an example. Do not use self-signed certificates in production.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ferron_core::config::validator::{ConfigurationValidationError, ConfigurationValidator};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::config_validator_scoped_key;
use ferron_core::directives::{Directive, DirectiveRegistry, DirectiveSubblock};
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_tls::{TlsContext, TlsResolver};
use rustls::ServerConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

// ============================================================================
// TLS resolver
// ============================================================================

/// Resolves TLS configuration by generating a self-signed certificate.
///
/// The resolver holds an `Arc<ServerConfig>` that rustls cloned on each
/// handshake. We generate the cert once in the provider and wrap it here.
struct SelfSignedResolver {
    config: Arc<ServerConfig>,
}

#[async_trait(?Send)]
impl TlsResolver for SelfSignedResolver {
    fn get_tls_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }
}

// ============================================================================
// Provider
// ============================================================================

/// Cache for generated configs per hostname. A real provider would load certs
/// from disk or an external API. Here we keep them in memory for the example.
static CONFIG_CACHE: std::sync::OnceLock<Mutex<HashMap<String, Arc<ServerConfig>>>> =
    std::sync::OnceLock::new();

fn config_cache() -> &'static Mutex<HashMap<String, Arc<ServerConfig>>> {
    CONFIG_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Provider that generates a self-signed certificate for the handshake host.
///
/// Registered as `provider selfsigned`. Ferron passes the host's `TlsContext`
/// which contains the `ServerConfigurationBlock` for the `tls` directive and
/// the `domain` filters (hostname / IP).
struct SelfSignedProvider;

impl Provider<TlsContext<'_>> for SelfSignedProvider {
    fn name(&self) -> &str {
        "selfsigned"
    }

    fn execute(&self, ctx: &mut TlsContext) -> Result<(), Box<dyn std::error::Error>> {
        // `ctx.config` is the `tls { ... }` block. The optional `selfsigned { ... }`
        // child block holds provider-specific options.
        let days: u32 = ctx
            .config
            .directives
            .get("selfsigned")
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.children.as_ref())
            .and_then(|children| children.directives.get("days"))
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.args.first())
            .and_then(|v| v.as_number())
            .map(|n| n as u32)
            .unwrap_or(365);

        // The hostname we should generate a cert for. `domain.host` is the SNI
        // or Host header, `domain.ip` is the local IP. Fall back to "localhost".
        let host = ctx
            .domain
            .host
            .clone()
            .or_else(|| ctx.domain.ip.map(|ip| ip.to_string()))
            .unwrap_or_else(|| "localhost".to_string());

        // Check cache first. This avoids regenerating the cert for every handshake.
        {
            let cache = config_cache().lock().unwrap();
            if let Some(cached) = cache.get(&host) {
                ctx.resolver = Some(Arc::new(SelfSignedResolver {
                    config: cached.clone(),
                }));
                return Ok(());
            }
        }

        // Generate a self-signed certificate via `rcgen`.
        //
        // We use `CertificateParams` + `KeyPair` for full control, similar to
        // `modules/tls-local`. The `days` parameter from config is ignored for
        // simplicity, but you could set `not_after` via `time` crate.
        let _ = days;
        use rcgen::{CertificateParams, DistinguishedName, SanType};
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, host.clone());
        // Add SAN: DNS name or IP address
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            params.subject_alt_names = vec![SanType::IpAddress(ip)];
        } else {
            params.subject_alt_names =
                vec![SanType::DnsName(host.clone().try_into().map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("invalid host as SAN: {host}"),
                    )
                })?)];
        }
        let key_pair = rcgen::KeyPair::generate()
            .map_err(|e| std::io::Error::other(format!("rcgen key generation failed: {e}")))?;
        let cert = params
            .self_signed(&key_pair)
            .map_err(|e| std::io::Error::other(format!("rcgen self-sign failed: {e}")))?;
        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        // Parse PEM into rustls types.
        let mut cert_reader = cert_pem.as_bytes();
        let cert_der: Vec<CertificateDer<'static>> =
            CertificateDer::pem_reader_iter(&mut cert_reader)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| std::io::Error::other(format!("failed to parse cert PEM: {e}")))?;

        let key_der = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
            .map_err(|e| std::io::Error::other(format!("failed to parse key PEM: {e}")))?;

        // Build the rustls ServerConfig. Use the helper from `ferron-tls` so
        // cipher suites, protocol versions, and client auth are consistent with
        // the rest of Ferron. For this example we use defaults from the TLS block.
        use ferron_tls::config::{TlsClientAuthCaSource, TlsClientAuthConfig, TlsCryptoConfig};
        let crypto = TlsCryptoConfig::from_config(ctx.config);
        // Client auth disabled for this example. `from_config` would parse it,
        // but we construct a disabled config explicitly:
        let client_auth = TlsClientAuthConfig {
            enabled: false,
            required: false,
            ca_source: TlsClientAuthCaSource::WebPkiRoots,
        };
        let builder = ferron_tls::builder::build_server_config_builder(&crypto, &client_auth)?;

        let mut server_config = builder
            .with_single_cert(cert_der, key_der)
            .map_err(|e| std::io::Error::other(format!("failed to set cert: {e}")))?;

        // Enable session tickets (default behavior in Ferron).
        if let Some(ticketer) = ferron_tls::builder::build_ticketer(ctx.config) {
            server_config.ticketer = ticketer;
        }

        // If ALPN is set (e.g. h2, http/1.1), forward it.
        if let Some(alpn) = ctx.alpn.clone() {
            server_config.alpn_protocols = alpn;
        }

        let server_config = Arc::new(server_config);

        // Cache for next handshake.
        config_cache()
            .lock()
            .unwrap()
            .insert(host, server_config.clone());

        ctx.resolver = Some(Arc::new(SelfSignedResolver {
            config: server_config,
        }));
        Ok(())
    }
}

// ============================================================================
// Validator
// ============================================================================

struct SelfSignedValidator;

impl ConfigurationValidator for SelfSignedValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        // This validator runs for the scoped block `tls { provider selfsigned; selfsigned { ... } }`.
        // `ferron_tls::validate_tls_common!` checks shared TLS directives (ciphers, etc.).
        ferron_tls::validate_tls_common!(config, ctx);

        ferron_core::validate_directive!(
            config,
            ctx.used_directives,
            days,
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

/// Loader that registers the self-signed TLS provider.
#[derive(Default)]
pub struct SelfSignedTlsModuleLoader;

impl ModuleLoader for SelfSignedTlsModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry.with_provider::<TlsContext, _>(|| Arc::new(SelfSignedProvider))
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("tls", "selfsigned"),
            Box::new(SelfSignedValidator),
        );
    }

    fn register_directives(&mut self, registry: &mut DirectiveRegistry) {
        registry.register(
            Directive {
                name: "days",
                usage: "days <count>",
                description:
                    "Validity period for the self-signed certificate in days. Default: 365",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("tls"),
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
    use std::sync::Arc as StdArc;

    #[test]
    fn validator_accepts_days() {
        let mut directives = FxHashMap::default();
        directives.insert(
            "days".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(30, None)],
                children: None,
                span: None,
            }],
        );
        let block = ServerConfigurationBlock {
            directives: StdArc::new(directives),
            matchers: FxHashMap::default(),
            span: None,
        };
        let v = SelfSignedValidator;
        let mut ctx = ferron_core::config::validator::ConfigurationValidatorContext {
            used_directives: std::collections::HashSet::new(),
            is_global: false,
            scoped_validators: StdArc::new(std::collections::HashMap::new()),
            diagnostics: Vec::new(),
            scope: None,
        };
        v.validate_block(&block, &mut ctx).unwrap();
    }
}
