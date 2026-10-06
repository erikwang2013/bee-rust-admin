// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[proc_macro_derive(Config, attributes(config))]
pub fn derive_config(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let expanded = quote! {
        impl bee_config::ConfigSource for #name {
            fn load<P: AsRef<std::path::Path>>(path: P) -> Result<Self, bee_config::ConfigError> {
                let path = path.as_ref();
                let content = std::fs::read_to_string(path)
                    .map_err(bee_config::ConfigError::from)?;
                let sections = bee_config::ini::IniParser::parse(&content);
                let default = sections
                    .get("default")
                    .ok_or_else(|| bee_config::ConfigError::MissingKey("default section".into()))?;

                // Build a serde_json::Map, parsing each value as JSON so that
                // numbers, booleans, and null are coerced to their proper types.
                // Values that are not valid JSON (e.g. bare strings like "dev"
                // or "localhost") are treated as JSON strings. If a value
                // contains a typo (e.g. "8o8o" for a port number) it will be
                // stored as a string and produce a clear deserialization error
                // from serde_json::from_value below.
                let mut map = serde_json::Map::new();
                for (k, v) in default {
                    let val: serde_json::Value = serde_json::from_str(v)
                        .unwrap_or_else(|_| serde_json::Value::String(v.clone()));
                    map.insert(k.clone(), val);
                }
                let json_value = serde_json::Value::Object(map);

                let cfg: Self = serde_json::from_value(json_value)
                    .map_err(|e| bee_config::ConfigError::Deserialize(e.to_string()))?;
                bee_config::paths::record::<Self>(path)?;
                Ok(cfg)
            }

            fn reload(&mut self) -> Result<(), bee_config::ConfigError> {
                let path = bee_config::paths::path_of::<Self>()?;
                // Retry: an editor writing in place leaves the file truncated
                // behind the change event, and the writer may be slow (or the
                // machine loaded) — so retry until a deadline, not a fixed
                // attempt count. A fixed 3x20ms window loses the race on a
                // busy box; 200ms is ~20x a local editor's rewrite.
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_millis(200);
                let mut last_err = None;
                loop {
                    match Self::load(&path) {
                        Ok(cfg) => {
                            *self = cfg;
                            return Ok(());
                        }
                        Err(e) => last_err = Some(e),
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(last_err.unwrap());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }

            fn watch(&self) -> Result<(), bee_config::ConfigError> {
                let path = bee_config::paths::path_of::<Self>()?;
                bee_config::paths::watch_path(&path)
            }
        }
    };

    TokenStream::from(expanded)
}
