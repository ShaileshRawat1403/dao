use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub model: ModelConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: ModelConfig::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ModelConfig {
    pub default_model: Option<String>,
    pub default_provider: Option<String>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            default_model: None,
            default_provider: None,
        }
    }
}
