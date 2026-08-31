#![allow(clippy::arc_with_non_send_sync)]
//! Example Ferron configuration adapter: `ferron-config-toml`
//!
//! This crate shows how to write a **configuration adapter** (a component that
//! loads `ServerConfiguration` from an external source). Adapters implement the
//! [`ConfigurationAdapter`] trait and are selected via `--config-adapter`
//! or by file extension.
//!
//! ## What this module does
//!
//! - Provides an adapter named `"toml"` that reads a JSON representation of
//!   `ServerConfiguration` from a TOML file. For simplicity, the TOML file is
//!   expected to contain a single key `json` with a JSON string that matches
//!   the `ServerConfiguration` schema. A real adapter would parse TOML into
//!   the Ferron config model directly or translate its own schema.
//! - For the purpose of this example we instead support a very small custom
//!   format: a TOML file with `global` and `hosts` tables that is converted
//!   into the minimal `ServerConfiguration` needed to pass validation.
//!
//!   The key point of the example is to demonstrate the **adapter API**, not
//!   the TOML parsing itself.
//!
//! ## Configuration example (TOML input file)
//!
//! ```toml
//! [global]
//! # This example adapter just logs the TOML and returns an empty config.
//! # A real adapter would map TOML keys to `ServerConfiguration`.
//! message = "hello"
//! ```
//!
//! Run Ferron with:
//!
//! ```bash
//! ferron run --config-adapter toml --config-params file=./ferron.toml
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use ferron_core::config::adapter::{
    AdaptResult, ConfigurationAdapter, ConfigurationAdapterError, ConfigurationMetadata,
    ConfigurationWatcher,
};
use ferron_core::config::{ServerConfiguration, ServerConfigurationBlock};
use ferron_core::loader::ModuleLoader;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use rustc_hash::FxHashMap;
use tokio::sync::mpsc;

// ============================================================================
// Adapter
// ============================================================================

/// Configuration adapter that loads a Ferron config from a TOML file.
///
/// The adapter reads the file, computes a content hash and mtime for drift
/// detection, and returns a minimal `ServerConfiguration`. The watcher
/// monitors the file for changes when `watch` is enabled.
struct TomlConfigurationAdapter;

impl ConfigurationAdapter for TomlConfigurationAdapter {
    fn adapt(&self, params: &HashMap<String, String>) -> AdaptResult {
        // `params` comes from `--config-params file=...;watch=1` or from the
        // default `file` insertion in `load_config_adapters`.
        let filename = params
            .get("file")
            .ok_or_else(|| ConfigurationAdapterError {
                inner: anyhow::anyhow!("'file' parameter is required for 'toml' adapter")
                    .into_boxed_dyn_error(),
                span: None,
            })?;

        let contents =
            std::fs::read_to_string(filename).map_err(|e| ConfigurationAdapterError {
                inner: anyhow::anyhow!("Failed to read TOML file '{filename}': {e}")
                    .into_boxed_dyn_error(),
                span: None,
            })?;

        // Parse as TOML to validate syntax. We use `toml::Value` as a generic
        // container. A real adapter would map this into `ServerConfiguration`.
        let toml_value: toml::Value =
            contents
                .parse()
                .map_err(|e: toml::de::Error| ConfigurationAdapterError {
                    inner: anyhow::anyhow!("Failed to parse TOML: {e}").into_boxed_dyn_error(),
                    span: None,
                })?;

        // For this example, we simply log the parsed keys and produce an
        // *empty* but valid configuration. An empty config will fail validation
        // later (Ferron requires at least one host), but that is expected for
        // the example. See the inline comment below for how to build a real
        // `ServerConfiguration` from TOML.
        ferron_core::log_info!(
            "TOML adapter parsed keys: {:?}",
            toml_value
                .as_table()
                .map(|t| t.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        );

        // Compute metadata for drift detection (same pattern as `config-json`).
        let hash = xxhash_rust::xxh3::xxh3_64(contents.as_bytes());
        let config_hash = format!("{:016x}", hash);
        let config_mtime = std::fs::metadata(filename)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // If the TOML contains a `json` key, treat it as a JSON-encoded
        // `ServerConfiguration` and deserialize it. This lets the example work
        // with a real Ferron configuration while still demonstrating TOML.
        let config = if let Some(toml::Value::String(json_str)) = toml_value.get("json") {
            serde_json::from_str(json_str).map_err(|e| ConfigurationAdapterError {
                inner: anyhow::anyhow!("Failed to parse `json` key as ServerConfiguration: {e}")
                    .into_boxed_dyn_error(),
                span: None,
            })?
        } else {
            // Minimal empty config. In a real adapter you would map TOML tables
            // to `ServerConfigurationPort` and `ServerConfigurationBlock`.
            //
            // Example of building a real config:
            // ```
            // let mut ports = BTreeMap::new();
            // let block = ServerConfigurationBlock {
            //     directives: Arc::new(map),
            //     matchers: FxHashMap::default(),
            //     span: None,
            // };
            // ports.insert("http".to_string(), vec![ServerConfigurationPort { port: Some(80), hosts: vec![(filters, block)] }]);
            // ServerConfiguration { global_config: Arc::new(global_block), ports }
            // ```
            ServerConfiguration {
                global_config: Arc::new(ServerConfigurationBlock {
                    directives: Arc::new(FxHashMap::default()),
                    matchers: FxHashMap::default(),
                    span: None,
                }),
                ports: Default::default(),
            }
        };

        let watch_enabled = params
            .get("watch")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let watcher: Box<dyn ConfigurationWatcher> = if watch_enabled {
            Box::new(TomlConfigurationWatcher::new(PathBuf::from(filename))?)
        } else {
            Box::new(DisabledWatcher)
        };

        let metadata = ConfigurationMetadata {
            config_hash,
            config_mtime,
            config_files: vec![PathBuf::from(filename)],
        };

        Ok((config, watcher, metadata))
    }

    fn file_extension(&self) -> Vec<&'static str> {
        vec!["toml"]
    }
}

// ============================================================================
// Watchers
// ============================================================================

struct DisabledWatcher;

#[async_trait]
impl ConfigurationWatcher for DisabledWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        std::future::pending().await
    }
}

struct TomlConfigurationWatcher {
    _debouncer: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
    change_rx: mpsc::Receiver<DebounceEventResult>,
    path: PathBuf,
}

impl TomlConfigurationWatcher {
    fn new(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel(32);
        let mut debouncer = new_debouncer(
            Duration::from_millis(100),
            move |result: DebounceEventResult| {
                let _ = tx.blocking_send(result);
            },
        )?;
        debouncer
            .watcher()
            .watch(&path, RecursiveMode::NonRecursive)?;
        Ok(Self {
            _debouncer: debouncer,
            change_rx: rx,
            path,
        })
    }
}

#[async_trait]
impl ConfigurationWatcher for TomlConfigurationWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        match self.change_rx.recv().await {
            Some(Ok(_)) => Ok(()),
            Some(Err(e)) => Err(Box::new(e)),
            None => Err("Watcher channel closed".into()),
        }
    }

    fn check_drift(&self, metadata: &ferron_core::config::adapter::ConfigurationMetadata) -> bool {
        let current = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        current != metadata.config_mtime
    }
}

// ============================================================================
// Loader
// ============================================================================

/// Loader that registers the TOML configuration adapter.
#[derive(Default)]
pub struct TomlConfigModuleLoader;

impl ModuleLoader for TomlConfigModuleLoader {
    fn register_configuration_adapters(
        &mut self,
        registry: &mut HashMap<&'static str, Box<dyn ConfigurationAdapter>>,
    ) {
        registry.insert("toml", Box::new(TomlConfigurationAdapter));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn adapt_parses_toml() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "message = \"hello\"").unwrap();
        let mut params = HashMap::new();
        params.insert("file".to_string(), f.path().to_string_lossy().to_string());
        let adapter = TomlConfigurationAdapter;
        let (config, _, metadata) = adapter.adapt(&params).unwrap();
        assert!(metadata.config_files.len() == 1);
        // Empty config has no ports
        assert!(config.ports.is_empty());
    }
}
