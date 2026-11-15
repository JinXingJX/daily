use super::{BUILTIN_AGENT_HOOKS_ID, BUILTIN_CHART_ID, BUILTIN_WEATHER_ID};
use crate::plugins::types::{
    PluginActivation, PluginAgentToolContribution, PluginContributions, PluginHookContribution,
    PluginManifestView, PluginRendererContribution, PluginSourceKind,
};
use serde_json::json;

pub(crate) fn known_plugin_ids() -> Vec<&'static str> {
    vec![
        BUILTIN_CHART_ID,
        BUILTIN_WEATHER_ID,
        BUILTIN_AGENT_HOOKS_ID,
        "market.mermaid",
        "market.csv_inspector",
        "market.prompt_linter",
    ]
}

pub(crate) fn is_builtin_plugin(plugin_id: &str) -> bool {
    matches!(
        plugin_id,
        BUILTIN_CHART_ID | BUILTIN_WEATHER_ID | BUILTIN_AGENT_HOOKS_ID
    )
}

pub(crate) fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|item| item == &value) {
        values.push(value);
    }
}

pub(crate) fn marketplace_registry() -> Vec<MarketplaceSeed> {
    vec![
        MarketplaceSeed {
            plugin_id: "market.mermaid".into(),
            latest_version: "0.1.0".into(),
            repository: "https://github.com/daily-plugins/mermaid".into(),
            tarball_url: "https://github.com/daily-plugins/mermaid/releases/download/v0.1.0/plugin.tar.gz".into(),
            sha256: "registry-managed-placeholder".into(),
        },
        MarketplaceSeed {
            plugin_id: "market.csv_inspector".into(),
            latest_version: "0.1.0".into(),
            repository: "https://github.com/daily-plugins/csv-inspector".into(),
            tarball_url: "https://github.com/daily-plugins/csv-inspector/releases/download/v0.1.0/plugin.tar.gz".into(),
            sha256: "registry-managed-placeholder".into(),
        },
        MarketplaceSeed {
            plugin_id: "market.prompt_linter".into(),
            latest_version: "0.1.0".into(),
            repository: "https://github.com/daily-plugins/prompt-linter".into(),
            tarball_url: "https://github.com/daily-plugins/prompt-linter/releases/download/v0.1.0/plugin.tar.gz".into(),
            sha256: "registry-managed-placeholder".into(),
        },
    ]
}

pub(crate) struct MarketplaceSeed {
    pub(crate) plugin_id: String,
    pub(crate) latest_version: String,
    pub(crate) repository: String,
    pub(crate) tarball_url: String,
    pub(crate) sha256: String,
}

pub(crate) fn base_manifest(plugin_id: &str) -> PluginManifestView {
    match plugin_id {
        BUILTIN_CHART_ID => PluginManifestView {
            identifier: BUILTIN_CHART_ID.into(),
            name: "Chart".into(),
            description: "Render bar, line, or area charts inside the conversation.".into(),
            payload_hint: r#"{"title":"Weekly focus","type":"bar","labels":["Mon","Tue"],"series":[{"name":"Output","data":[3,5]}]}"#.into(),
            version: "1.0.0".into(),
            author: "Daily Core".into(),
            source: PluginSourceKind::BuiltIn,
            entry: None,
            dev_entry: None,
            dev_url: None,
            activation: PluginActivation::OnView,
            installed: true,
            enabled: true,
            built_in: true,
            contributes: PluginContributions {
                renderers: vec![PluginRendererContribution {
                    id: "chart".into(),
                    name: "Chart".into(),
                    description: "Renders a compact quantitative chart from JSON data.".into(),
                    payload_hint: r#"{"title":"Weekly focus","type":"bar","labels":["Mon","Tue"],"series":[{"name":"Output","data":[3,5]}]}"#.into(),
                    legacy_identifier: Some("plugin:chart".into()),
                }],
                ..PluginContributions::default()
            },
            settings_schema: json!({}),
            settings: json!({}),
            runtime_errors: Vec::new(),
        },
        BUILTIN_WEATHER_ID => PluginManifestView {
            identifier: BUILTIN_WEATHER_ID.into(),
            name: "Weather".into(),
            description: "Render a compact weather card with conditions and forecast.".into(),
            payload_hint: r#"{"location":"Shanghai","condition":"Sunny","temperatureC":21,"forecast":[{"label":"Now","value":"21 C"},{"label":"PM","value":"24 C"}]}"#.into(),
            version: "1.0.0".into(),
            author: "Daily Core".into(),
            source: PluginSourceKind::BuiltIn,
            entry: None,
            dev_entry: None,
            dev_url: None,
            activation: PluginActivation::OnView,
            installed: true,
            enabled: true,
            built_in: true,
            contributes: PluginContributions {
                renderers: vec![PluginRendererContribution {
                    id: "weather".into(),
                    name: "Weather".into(),
                    description: "Renders current weather conditions and short forecasts.".into(),
                    payload_hint: r#"{"location":"Shanghai","condition":"Sunny","temperatureC":21,"forecast":[{"label":"Now","value":"21 C"},{"label":"PM","value":"24 C"}]}"#.into(),
                    legacy_identifier: Some("plugin:weather".into()),
                }],
                ..PluginContributions::default()
            },
            settings_schema: json!({}),
            settings: json!({}),
            runtime_errors: Vec::new(),
        },
        BUILTIN_AGENT_HOOKS_ID => PluginManifestView {
            identifier: BUILTIN_AGENT_HOOKS_ID.into(),
            name: "Agent Hooks".into(),
            description: "Render start/end hook runs across one or more agents.".into(),
            payload_hint: r#"{"title":"On Start Hooks","phase":"on_start","mode":"serial","appendPreviousOutput":true,"agents":[{"agentId":1,"agentName":"Research","status":"completed","model":"gpt-4.1-mini","output":"Answer A"}]}"#.into(),
            version: "1.0.0".into(),
            author: "Daily Core".into(),
            source: PluginSourceKind::BuiltIn,
            entry: None,
            dev_entry: None,
            dev_url: None,
            activation: PluginActivation::OnView,
            installed: true,
            enabled: true,
            built_in: true,
            contributes: PluginContributions {
                renderers: vec![PluginRendererContribution {
                    id: "agent_hooks".into(),
                    name: "Agent Hooks".into(),
                    description: "Renders lifecycle hook output inline with the active turn.".into(),
                    payload_hint: r#"{"title":"On Start Hooks","phase":"on_start","mode":"serial","appendPreviousOutput":true,"agents":[{"agentId":1,"agentName":"Research","status":"completed","model":"gpt-4.1-mini","output":"Answer A"}]}"#.into(),
                    legacy_identifier: Some("plugin:agent_hooks".into()),
                }],
                hooks: vec![PluginHookContribution {
                    id: "agent-hooks-report".into(),
                    phase: "on_start,on_finish".into(),
                    description: "Displays hook run summaries after lifecycle execution.".into(),
                }],
                ..PluginContributions::default()
            },
            settings_schema: json!({}),
            settings: json!({}),
            runtime_errors: Vec::new(),
        },
        "market.mermaid" => marketplace_manifest(
            "market.mermaid",
            "Mermaid Blocks",
            "Render Mermaid diagrams from agent output.",
            "mermaid",
            "diagram",
            r#"{"code":"graph TD; A-->B;"}"#,
        ),
        "market.csv_inspector" => marketplace_manifest(
            "market.csv_inspector",
            "CSV Inspector",
            "Render small CSV payloads as sortable tables.",
            "csv_table",
            "CSV Table",
            r#"{"columns":["Name","Value"],"rows":[["Focus","5"],["Risk","2"]]}"#,
        ),
        "market.prompt_linter" => {
            let mut manifest = marketplace_manifest(
                "market.prompt_linter",
                "Prompt Linter",
                "Adds an agent tool that critiques ambiguous prompts before execution.",
                "lint_report",
                "Lint Report",
                r#"{"issues":[{"severity":"medium","message":"Clarify the target audience."}]}"#,
            );
            manifest.contributes.agent_tools.push(PluginAgentToolContribution {
                id: "lint_prompt".into(),
                name: "Lint Prompt".into(),
                description: "Checks the current prompt for ambiguity and missing constraints.".into(),
                input_hint: r#"{"prompt":"..."}"#.into(),
            });
            manifest
        }
        _ => marketplace_manifest(
            plugin_id,
            plugin_id,
            "Registry plugin",
            "generic",
            "Generic Renderer",
            r#"{}"#,
        ),
    }
}

pub(crate) fn marketplace_manifest(
    id: &str,
    name: &str,
    description: &str,
    renderer_id: &str,
    renderer_name: &str,
    payload_hint: &str,
) -> PluginManifestView {
    PluginManifestView {
        identifier: id.into(),
        name: name.into(),
        description: description.into(),
        payload_hint: payload_hint.into(),
        version: "0.1.0".into(),
        author: "Daily Marketplace".into(),
        source: PluginSourceKind::Marketplace,
        entry: Some("dist/plugin.js".into()),
        dev_entry: None,
        dev_url: None,
        activation: PluginActivation::OnView,
        installed: false,
        enabled: false,
        built_in: false,
        contributes: PluginContributions {
            renderers: vec![PluginRendererContribution {
                id: renderer_id.into(),
                name: renderer_name.into(),
                description: description.into(),
                payload_hint: payload_hint.into(),
                legacy_identifier: None,
            }],
            ..PluginContributions::default()
        },
        settings_schema: json!({}),
        settings: json!({}),
        runtime_errors: Vec::new(),
    }
}
