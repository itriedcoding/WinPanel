//! Server templates: built-in definitions plus user-supplied templates that are
//! dropped into `<data>/templates/*.json`. Adding a new application never
//! requires touching the core installer.

use crate::error::{AppError, Result};
use crate::models::EnvVar;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub install: InstallSpec,
    #[serde(default)]
    pub runtime: Vec<RuntimeRequirement>,
    #[serde(default)]
    pub startup_exe: String,
    #[serde(default)]
    pub startup_args: String,
    #[serde(default)]
    pub stop_command: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub env: Vec<EnvVar>,
    #[serde(default)]
    pub memory_mb: u64,
    #[serde(default)]
    pub storage_mb: u64,
    #[serde(default)]
    pub options: Vec<TemplateOption>,
    #[serde(default)]
    pub files: Vec<TemplateFile>,
    #[serde(default)]
    pub notes: String,
}

fn default_protocol() -> String {
    "tcp".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallSpec {
    /// none | download | minecraft_vanilla | paper | bedrock | fivem
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub target: String,
    /// "none" keeps the download as a single file, "zip" extracts it.
    #[serde(default = "default_archive")]
    pub archive: String,
}

fn default_method() -> String {
    "none".to_string()
}

fn default_archive() -> String {
    "none".to_string()
}

impl Default for InstallSpec {
    fn default() -> Self {
        Self {
            method: default_method(),
            url: String::new(),
            target: String::new(),
            archive: default_archive(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRequirement {
    pub kind: String,
    #[serde(default)]
    pub minimum_major: u32,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateOption {
    pub id: String,
    pub label: String,
    /// text | bool | select | number | password
    #[serde(default = "default_option_kind")]
    pub kind: String,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub choices: Vec<TemplateChoice>,
}

fn default_option_kind() -> String {
    "text".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateChoice {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateFile {
    pub path: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    #[serde(flatten)]
    pub template: Template,
    pub runtime_status: Vec<crate::runtime::RuntimeStatus>,
    pub source: String,
}

#[derive(Default)]
pub struct TemplateCollection {
    templates: Vec<(Template, String)>,
    warnings: Vec<String>,
}

impl TemplateCollection {
    pub fn load(user_dir: &Path) -> Self {
        let mut collection = TemplateCollection::default();
        match serde_json::from_str::<BuiltinFile>(include_str!("../templates/builtin.json")) {
            Ok(builtin) => {
                for template in builtin.templates {
                    collection.templates.push((template, "builtin".to_string()));
                }
            }
            Err(err) => {
                // The built-in file is compiled in, so this is a build defect.
                tracing::error!("built-in templates are invalid: {err}");
                collection
                    .warnings
                    .push(format!("Built-in templates failed to load: {err}"));
            }
        }

        if let Ok(entries) = std::fs::read_dir(user_dir) {
            let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();
            for path in paths {
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let raw = match std::fs::read_to_string(&path) {
                    Ok(raw) => raw,
                    Err(err) => {
                        collection.warnings.push(format!(
                            "{}: could not be read ({err})",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        continue;
                    }
                };
                let parsed = if raw.trim_start().starts_with('[') {
                    serde_json::from_str::<Vec<Template>>(&raw)
                } else {
                    serde_json::from_str::<BuiltinFile>(&raw).map(|file| file.templates)
                };
                match parsed {
                    Ok(templates) => {
                        for template in templates {
                            if let Err(err) = validate(&template) {
                                collection.warnings.push(format!("{}: {err}", template.id));
                                continue;
                            }
                            let source = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            collection
                                .templates
                                .retain(|(existing, _)| existing.id != template.id);
                            collection.templates.push((template, source));
                        }
                    }
                    Err(err) => collection.warnings.push(format!(
                        "{}: invalid JSON ({err})",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )),
                }
            }
        }

        collection
    }

    pub fn describe(&self) -> Vec<TemplateInfo> {
        let mut templates: Vec<&(Template, String)> = self.templates.iter().collect();
        templates.sort_by(|a, b| {
            a.0.category
                .cmp(&b.0.category)
                .then_with(|| a.0.name.cmp(&b.0.name))
        });
        templates
            .into_iter()
            .map(|(template, source)| TemplateInfo {
                template: template.clone(),
                runtime_status: template
                    .runtime
                    .iter()
                    .map(|requirement| crate::runtime::detect(&requirement.kind))
                    .collect(),
                source: source.clone(),
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&Template> {
        self.templates
            .iter()
            .find(|(template, _)| template.id == id)
            .map(|(template, _)| template)
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

#[derive(Deserialize)]
struct BuiltinFile {
    #[serde(default)]
    templates: Vec<Template>,
}

fn validate(template: &Template) -> std::result::Result<(), String> {
    if template.id.trim().is_empty() {
        return Err("template id is required".into());
    }
    if !template
        .id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err("template id may only contain lowercase letters, digits, '-' and '_'".into());
    }
    if template.name.trim().is_empty() {
        return Err("template name is required".into());
    }
    let valid_methods = [
        "none",
        "download",
        "minecraft_vanilla",
        "paper",
        "bedrock",
        "fivem",
    ];
    if !valid_methods.contains(&template.install.method.as_str()) {
        return Err(format!(
            "unknown install method \"{}\" (expected one of {})",
            template.install.method,
            valid_methods.join(", ")
        ));
    }
    if template.install.method == "download" && template.install.url.trim().is_empty() {
        return Err("download templates need an install.url".into());
    }
    for file in &template.files {
        if file.path.trim().is_empty() {
            return Err("template files need a path".into());
        }
        if crate::util::normalize_rel_path(&file.path).is_err() {
            return Err(format!("template file path is unsafe: {}", file.path));
        }
    }
    Ok(())
}

/// Replace `{{placeholder}}` tokens. Unknown placeholders are left untouched so
/// templates stay debuggable instead of silently losing configuration.
pub fn render(input: &str, values: &std::collections::HashMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let key = after[..end].trim();
                match values.get(key) {
                    Some(value) => out.push_str(value),
                    None => {
                        out.push_str("{{");
                        out.push_str(key);
                        out.push_str("}}");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Substitute the runtime placeholders used in template strings.
pub fn runtime_values(memory_mb: u64, port: u16) -> std::collections::HashMap<String, String> {
    let mut values = std::collections::HashMap::new();
    values.insert("memoryMb".to_string(), memory_mb.max(256).to_string());
    values.insert("port".to_string(), port.to_string());
    values
}

pub fn require_template<'a>(collection: &'a TemplateCollection, id: &str) -> Result<&'a Template> {
    collection
        .get(id)
        .ok_or_else(|| AppError::bad_request(format!("Unknown server template \"{id}\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_load_and_validate() {
        let collection = TemplateCollection::load(Path::new("./does-not-exist"));
        assert!(collection.get("minecraft-java").is_some());
        assert!(collection.get("generic-node").is_some());
        assert!(collection.describe().len() >= 8);
        assert!(collection.warnings().is_empty());
    }

    #[test]
    fn placeholders_render() {
        let mut values = std::collections::HashMap::new();
        values.insert("port".to_string(), "25565".to_string());
        assert_eq!(
            render("port={{port}} {{unknown}}", &values),
            "port=25565 {{unknown}}"
        );
    }
}
