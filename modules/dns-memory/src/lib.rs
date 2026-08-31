#![allow(clippy::arc_with_non_send_sync)]
#![allow(clippy::type_complexity)]

//! Example Ferron DNS provider: `ferron-dns-memory`
//!
//! This crate shows how to write a **DNS provider** for ACME DNS-01 challenges.
//! DNS providers implement [`DnsClient`] (async CRUD for DNS records) and a
//! [`Provider<DnsContext>`] that creates the client from configuration.
//!
//! ## What this module does
//!
//! - Provides a provider named `"memory"` that stores DNS records in a global
//!   `HashMap` in memory. No external API is called.
//! - Supports `TXT` records (used for `_acme-challenge` validation) and any
//!   other type via a string.
//! - Enforces a minimum TTL of 1 second.
//!
//! ## Configuration example
//!
//! ```ferron
//! example.com {
//!     tls {
//!         provider acme
//!         acme {
//!             dns memory
//!             memory {
//!                 # no additional config needed
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! In practice, the ACME module resolves `dns` scoped validators via the
//! `dns` namespace. This example registers `dns.memory`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use ferron_core::config::validator::{ConfigurationValidationError, ConfigurationValidator};
use ferron_core::config_validator_scoped_key;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_dns::{DnsClient, DnsContext, DnsProviderError, DnsRecord, DnsRecordType};

// ============================================================================
// In-memory DNS client
// ============================================================================

/// Global in-memory store: (name, record_type) -> (value, ttl)
///
/// A real DNS provider would call an HTTP API (Cloudflare, Route53, etc.).
/// Here we keep it simple with a `Mutex<HashMap>`.
static MEMORY_DNS_STORE: OnceLock<Arc<Mutex<HashMap<(String, String), (String, u32)>>>> =
    OnceLock::new();

fn dns_store() -> Arc<Mutex<HashMap<(String, String), (String, u32)>>> {
    MEMORY_DNS_STORE
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

/// A DNS client that stores records in memory.
///
/// This is only for demonstration. It has no persistence and no network calls.
pub struct MemoryDnsClient {
    store: Arc<Mutex<HashMap<(String, String), (String, u32)>>>,
}

impl MemoryDnsClient {
    fn new() -> Self {
        Self { store: dns_store() }
    }

    /// List all stored records (for debugging / tests).
    pub fn list_records(&self) -> HashMap<(String, String), (String, u32)> {
        self.store.lock().unwrap().clone()
    }
}

#[async_trait]
impl DnsClient for MemoryDnsClient {
    fn minimum_ttl(&self) -> u32 {
        // Real providers enforce a lower bound (e.g. 60s for Cloudflare).
        // For the example we allow 1 second to make E2E tests fast.
        1
    }

    async fn update_record(&self, record: &DnsRecord) -> Result<(), DnsProviderError> {
        if record.ttl < self.minimum_ttl() {
            return Err(DnsProviderError::new(format!(
                "TTL {} is below minimum {}",
                record.ttl,
                self.minimum_ttl()
            )));
        }
        let mut store = self.store.lock().unwrap();
        store.insert(
            (record.name.clone(), record.record_type.to_string()),
            (record.value.clone(), record.ttl),
        );
        ferron_core::log_info!(
            "Memory DNS: set {} {} -> {} (TTL {})",
            record.record_type,
            record.name,
            record.value,
            record.ttl
        );
        Ok(())
    }

    async fn delete_record(
        &self,
        name: &str,
        record_type: DnsRecordType,
    ) -> Result<(), DnsProviderError> {
        let mut store = self.store.lock().unwrap();
        store.remove(&(name.to_string(), record_type.to_string()));
        ferron_core::log_info!("Memory DNS: deleted {} {}", record_type, name);
        Ok(())
    }
}

// ============================================================================
// Provider
// ============================================================================

/// Provider that creates the in-memory DNS client.
///
/// The `DnsContext` gives access to the configuration block:
///
/// ```ferron
/// dns memory
/// memory {
///     # no keys needed, but you could add e.g. `ttl 60`
/// }
/// ```
struct MemoryDnsProvider;

impl Provider<DnsContext<'_>> for MemoryDnsProvider {
    fn name(&self) -> &str {
        "memory"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        // In a real provider you would read `ctx.config` for API tokens:
        //
        // let token = ctx.config.get_value("token")
        //     .and_then(|v| v.as_str())
        //     .ok_or("missing token")?;
        //
        // Here we ignore config and just create the in-memory client.
        let _ = ctx.config; // mark as used
        ctx.client = Some(Arc::new(MemoryDnsClient::new()));
        Ok(())
    }
}

// ============================================================================
// Validator
// ============================================================================

struct MemoryDnsValidator;

impl ConfigurationValidator for MemoryDnsValidator {
    fn validate_block(
        &self,
        _config: &ferron_core::config::ServerConfigurationBlock,
        _ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        // No configuration is required for the memory provider. We accept any
        // block (including empty) and report no errors. A real provider would
        // validate `token`, `zone`, etc. here with `validate_directive!`.
        Ok(())
    }
}

// ============================================================================
// Module loader
// ============================================================================

/// Loader that registers the memory DNS provider.
#[derive(Default)]
pub struct MemoryDnsModuleLoader;

impl ModuleLoader for MemoryDnsModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry.with_provider::<DnsContext, _>(|| Arc::new(MemoryDnsProvider))
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("dns", "memory"),
            Box::new(MemoryDnsValidator),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::ServerConfigurationBlock;
    use rustc_hash::FxHashMap;
    use std::sync::Arc as StdArc;

    #[tokio::test]
    async fn update_and_delete_record() {
        let client = MemoryDnsClient::new();
        let record = DnsRecord {
            name: "_acme-challenge.example.com".to_string(),
            record_type: DnsRecordType::TXT,
            value: "challenge-token".to_string(),
            ttl: 60,
        };
        client.update_record(&record).await.unwrap();
        let stored = client.list_records();
        assert_eq!(
            stored
                .get(&("_acme-challenge.example.com".to_string(), "TXT".to_string()))
                .unwrap()
                .0,
            "challenge-token"
        );
        client
            .delete_record("_acme-challenge.example.com", DnsRecordType::TXT)
            .await
            .unwrap();
        assert!(client.list_records().is_empty());
    }

    #[test]
    fn validator_accepts_empty() {
        let block = ServerConfigurationBlock {
            directives: StdArc::new(FxHashMap::default()),
            matchers: FxHashMap::default(),
            span: None,
        };
        let v = MemoryDnsValidator;
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
