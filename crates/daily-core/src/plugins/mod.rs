pub(crate) mod agent_runs;
pub(crate) mod agent_sessions;
mod builtin;
mod manifest;
pub(crate) mod rpc;
mod state;
mod types;

pub(crate) use state::*;
pub use types::*;

pub(crate) const APP_DIR_NAME: &str = ".daily";
pub(crate) const PLUGINS_FILE_NAME: &str = "plugins.json";
pub(crate) const MAX_PLUGIN_CONFIG_BYTES: usize = 1024 * 1024;

pub const BUILTIN_CHART_ID: &str = "builtin.chart";
pub const BUILTIN_WEATHER_ID: &str = "builtin.weather";
pub const BUILTIN_AGENT_HOOKS_ID: &str = "builtin.agent_hooks";

pub fn canonical_plugin_id(input: &str) -> String {
    match input.trim() {
        "plugin:chart" => BUILTIN_CHART_ID.into(),
        "plugin:weather" => BUILTIN_WEATHER_ID.into(),
        "plugin:agent_hooks" => BUILTIN_AGENT_HOOKS_ID.into(),
        value => value.to_string(),
    }
}

pub fn canonical_plugin_ids(input: Vec<String>) -> Vec<String> {
    let mut values = input
        .into_iter()
        .map(|value| canonical_plugin_id(&value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::unix_timestamp;

    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn install_does_not_reenable_disabled_builtin_plugins() {
        let mut state = PluginState::default();
        state.disable(BUILTIN_CHART_ID).unwrap();

        state.install("market.mermaid").unwrap();

        assert!(!state.is_enabled(BUILTIN_CHART_ID));
        assert!(state.is_installed(BUILTIN_CHART_ID));
    }

    #[test]
    fn load_preserves_disabled_builtin_plugins() {
        let root =
            std::env::temp_dir().join(format!("daily-plugin-state-test-{}", unix_timestamp()));
        let mut state = PluginState::default();
        state.disable(BUILTIN_WEATHER_ID).unwrap();
        state.persist(&root).unwrap();

        let loaded = PluginState::load(&root).unwrap();

        assert!(!loaded.is_enabled(BUILTIN_WEATHER_ID));
        assert!(loaded.is_installed(BUILTIN_WEATHER_ID));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prompt_manifests_include_enabled_mounted_marketplace_plugins() {
        let mut state = PluginState::default();
        state.install("market.mermaid").unwrap();
        state.enable("market.mermaid").unwrap();

        let manifests =
            state.prompt_manifests(&[BUILTIN_CHART_ID.to_string(), "market.mermaid".to_string()]);

        assert!(manifests
            .iter()
            .any(|manifest| manifest.identifier == BUILTIN_CHART_ID));
        assert!(manifests
            .iter()
            .any(|manifest| manifest.identifier == "market.mermaid"));
    }

    #[test]
    fn install_local_dev_plugin_loads_views() {
        let root = unique_test_dir("daily-local-plugin-test");
        let manifest_dir = root.join("fiction-plugin").join(".daily-plugin");
        fs::create_dir_all(&manifest_dir).unwrap();
        fs::write(
            manifest_dir.join("plugin.json"),
            r#"{
              "identifier": "local.fiction",
              "name": "Fiction Workbench",
              "contributes": {
                "views": [
                  {
                    "id": "fiction.canvas",
                    "title": "Fiction Canvas",
                    "location": "main",
                    "entry": "view.js",
                    "devEntry": "src/main.tsx"
                  },
                  {
                    "id": "fiction.worker",
                    "title": "Fiction Worker",
                    "location": "background",
                    "entry": "worker.js",
                    "devEntry": "src/worker.ts"
                  }
                ],
                "eventViewParsers": [
                  {
                    "id": "fiction.writer-events",
                    "name": "Fiction Writer Events",
                    "description": "Render fiction-specific writer tool events.",
                    "renderer": "fiction.timeline",
                    "support": {
                      "eventKinds": ["tool_call", "tool_result"],
                      "toolIds": ["writer.applyPatchOperations"],
                      "statuses": ["in_progress", "completed"]
                    }
                  }
                ]
              }
            }"#,
        )
        .unwrap();

        let mut state = PluginState::default();
        state
            .install_local_dev(InstallLocalDevPluginInput {
                manifest_path: root.join("fiction-plugin").display().to_string(),
                dev_url: Some(" http://127.0.0.1:5174/ ".into()),
            })
            .unwrap();

        let manifest = state.manifest_for("local.fiction").unwrap();

        assert!(matches!(manifest.source, PluginSourceKind::LocalDev));
        assert!(manifest.installed);
        assert!(manifest.enabled);
        assert_eq!(manifest.contributes.views[0].runtime, "iframe");
        assert_eq!(
            manifest.contributes.views[0].dev_entry.as_deref(),
            Some("src/main.tsx")
        );
        assert_eq!(manifest.contributes.views[1].location, "background");
        assert_eq!(
            manifest.contributes.views[1].dev_entry.as_deref(),
            Some("src/worker.ts")
        );
        assert_eq!(
            manifest.contributes.views[0].dev_url.as_deref(),
            Some("http://127.0.0.1:5174/")
        );
        assert_eq!(manifest.contributes.event_view_parsers.len(), 1);
        assert_eq!(
            manifest.contributes.event_view_parsers[0].support.tool_ids[0],
            "writer.applyPatchOperations"
        );
        assert!(manifest.contributes.event_view_parsers[0].support.supports(
            "tool_call",
            Some("writer.applyPatchOperations"),
            "completed"
        ));
        assert!(
            !manifest.contributes.event_view_parsers[0].support.supports(
                "llm_thought",
                Some("writer.applyPatchOperations"),
                "completed"
            )
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_dev_plugins_reject_remote_dev_urls_and_relative_paths() {
        let root = unique_test_dir("daily-local-plugin-trust-test");
        let manifest_dir = root.join(".daily-plugin");
        fs::create_dir_all(&manifest_dir).unwrap();
        fs::write(
            manifest_dir.join("plugin.json"),
            r#"{ "identifier": "local.trust", "name": "Trust" }"#,
        )
        .unwrap();
        let mut state = PluginState::default();

        for dev_url in ["https://example.com/", "http://10.0.0.2:5174/"] {
            let error = state
                .install_local_dev(InstallLocalDevPluginInput {
                    manifest_path: root.display().to_string(),
                    dev_url: Some(dev_url.into()),
                })
                .unwrap_err();
            assert!(error.contains("localhost"), "{dev_url}: {error}");
        }
        for dev_url in [
            "http://localhost:5174/",
            "http://[::1]:5174/",
            "http://127.0.0.1:5174/",
        ] {
            state
                .install_local_dev(InstallLocalDevPluginInput {
                    manifest_path: root.display().to_string(),
                    dev_url: Some(dev_url.into()),
                })
                .unwrap();
        }

        let relative = state
            .install_local_dev(InstallLocalDevPluginInput {
                manifest_path: ".daily-plugin/plugin.json".into(),
                dev_url: None,
            })
            .unwrap_err();
        assert!(relative.contains("absolute"), "{relative}");

        // A workspace-provided plugin state cannot smuggle in a relative manifest path.
        let smuggled = LocalDevPluginRef {
            plugin_id: "local.trust".into(),
            manifest_path: ".daily-plugin/plugin.json".into(),
            dev_url: None,
        };
        assert!(state.load_dev_manifest(&smuggled).is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_dev_manifest_errors_surface_in_installed_list() {
        let root = unique_test_dir("daily-local-plugin-error-test");
        let manifest_dir = root.join(".daily-plugin");
        fs::create_dir_all(&manifest_dir).unwrap();
        let manifest_path = manifest_dir.join("plugin.json");
        fs::write(
            &manifest_path,
            r#"{"identifier":"local.broken","name":"Broken Plugin"}"#,
        )
        .unwrap();

        let mut state = PluginState::default();
        state
            .install_local_dev(InstallLocalDevPluginInput {
                manifest_path: root.display().to_string(),
                dev_url: None,
            })
            .unwrap();
        fs::write(&manifest_path, "{not-json").unwrap();

        let manifest = state
            .installed_manifests()
            .into_iter()
            .find(|manifest| manifest.identifier == "local.broken")
            .unwrap();

        assert!(matches!(manifest.source, PluginSourceKind::LocalDev));
        assert!(manifest.runtime_errors[0].contains("Failed to parse"));

        let _ = fs::remove_dir_all(root);
    }

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let nanos = crate::util::unique_stamp();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}
