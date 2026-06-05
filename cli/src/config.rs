use directories::ProjectDirs;
use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct S3Config {
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EiderConfig {
    pub output_format: Option<String>,
    pub default_out: Option<String>,
    pub local_stac: Option<String>,
    pub s3: Option<S3Config>,
}

impl EiderConfig {
    pub fn load() -> color_eyre::eyre::Result<Self> {
        let mut figment = Figment::new();

        // Global config
        if let Some(proj_dirs) = ProjectDirs::from("", "", "eider") {
            let global_config = proj_dirs.config_dir().join("config.toml");
            if global_config.exists() {
                figment = figment.merge(Toml::file(global_config));
            }
        }

        // Local config
        let local_config = std::env::current_dir()
            .unwrap_or_default()
            .join(".eider.toml");
        if local_config.exists() {
            figment = figment.merge(Toml::file(local_config));
        }

        figment = figment.merge(Env::prefixed("EIDER_"));

        let config: EiderConfig = figment.extract()?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    struct EnvGuard(&'static str);
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }

    #[test]
    #[serial]
    fn test_parse_local_stac_from_env() {
        std::env::set_var("EIDER_LOCAL_STAC", "http://test-local-stac:8080");
        let _guard = EnvGuard("EIDER_LOCAL_STAC");

        let config = EiderConfig::load().unwrap();
        assert_eq!(
            config.local_stac.as_deref(),
            Some("http://test-local-stac:8080")
        );
    }

    #[test]
    #[serial]
    fn test_local_toml_sets_output_format() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".eider.toml"), "output_format = \"json\"\n").unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let config = EiderConfig::load().unwrap();
        std::env::set_current_dir(prev).unwrap();

        assert_eq!(config.output_format.as_deref(), Some("json"));
    }
}
