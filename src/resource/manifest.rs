// resource/manifest.rs

//! # Manifest Module
//!
//! Handles loading, parsing, and managing stack manifests.
//! A manifest describes the resources that make up a stack and their configurations.
//!
//! The primary type is `Manifest`, which represents a parsed stackql_manifest.yml file.
//! This module also provides types for resources, properties, and other manifest components.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{fs, process};

use log::{debug, error};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur when working with manifests.
#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("Failed to read manifest file: {0}")]
    FileReadError(#[from] std::io::Error),

    #[error("Failed to parse manifest: {0}")]
    ParseError(#[from] serde_yaml::Error),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid field: {0}")]
    InvalidField(String),

    #[error("Failed to resolve file() directive: {0}")]
    FileIncludeError(String),

    #[error("Manifest validation failed: {0}")]
    ValidationFailed(String),
}

/// Type alias for ManifestResult
pub type ManifestResult<T> = Result<T, ManifestError>;

/// Represents a stack manifest file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    /// Version of the manifest format
    #[serde(default = "default_version")]
    pub version: u32,

    /// Name of the stack
    pub name: String,

    /// Description of the stack
    #[serde(default)]
    pub description: String,

    /// List of providers used by the stack
    pub providers: Vec<String>,

    /// Global variables for the stack
    #[serde(default)]
    pub globals: Vec<GlobalVar>,

    /// Resources in the stack
    #[serde(default)]
    pub resources: Vec<Resource>,

    /// Stack-level exports (written to JSON output file)
    #[serde(default)]
    pub exports: Vec<String>,
}

/// Default version for manifest when not specified
fn default_version() -> u32 {
    1
}

/// Represents a global variable in the manifest.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GlobalVar {
    /// Name of the global variable
    pub name: String,

    /// Value of the global variable - can be a string or a complex structure
    #[serde(default)]
    pub value: serde_yaml::Value,

    /// Optional description
    #[serde(default)]
    pub description: String,

    /// When true, the rendered value is masked in all log output
    /// (dry-run queries, --show-queries, debug logs)
    #[serde(default)]
    pub protected: bool,
}

/// Represents a resource in the manifest.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Resource {
    /// Name of the resource
    pub name: String,

    /// Type of the resource (defaults to "resource")
    #[serde(default = "default_resource_type")]
    pub r#type: String,

    /// Custom file name for resource queries (if not derived from name)
    #[serde(default)]
    pub file: Option<String>,

    /// Inline SQL for query/command type resources
    #[serde(default)]
    pub sql: Option<String>,

    /// Script command for script type resources
    #[serde(default)]
    pub run: Option<String>,

    /// Properties for the resource
    #[serde(default)]
    pub props: Vec<Property>,

    /// Exports from the resource (can be strings or {key: value} maps)
    #[serde(default)]
    pub exports: Vec<serde_yaml::Value>,

    /// Protected exports
    #[serde(default)]
    pub protected: Vec<String>,

    /// Description of the resource
    #[serde(default)]
    pub description: String,

    /// Condition for resource processing
    #[serde(default)]
    pub r#if: Option<String>,

    /// Skip validation for this resource
    #[serde(default)]
    pub skip_validation: Option<bool>,

    /// When true, the resource is not processed during `teardown`.
    ///
    /// - `query` resources: the query is not executed and each declared
    ///   export is set to the `<unknown>` placeholder, so any downstream
    ///   query that references those exports is skipped rather than run.
    /// - `resource` / `multi` resources: exports are still collected (a
    ///   downstream `delete` may need them) but the resource's own `delete`
    ///   query is not executed, so the resource is retained.
    ///
    /// Has no effect on `build` or `test`.
    #[serde(default)]
    pub skip_on_delete: bool,

    /// Auth configuration for the resource
    #[serde(default)]
    pub auth: Option<serde_yaml::Value>,

    /// Return value mappings from mutation operations (create, update, delete).
    /// Each operation maps to a list of field specs:
    ///   - `Identifier: identifier` (rename: capture `Identifier` as `this.identifier`)
    ///   - `ErrorCode` (direct: capture as `this.ErrorCode`)
    #[serde(default)]
    pub return_vals: Option<HashMap<String, Vec<serde_yaml::Value>>>,
}

impl Resource {
    /// Parse `return_vals` for a given operation (create, update, delete).
    /// Returns a list of (source_field, target_field) pairs.
    /// - `Identifier: identifier` -> ("Identifier", "identifier")
    /// - `ErrorCode` (string) -> ("ErrorCode", "ErrorCode")
    pub fn get_return_val_mappings(&self, operation: &str) -> Vec<(String, String)> {
        let Some(ref rv) = self.return_vals else {
            return vec![];
        };
        let Some(specs) = rv.get(operation) else {
            return vec![];
        };
        let mut mappings = Vec::new();
        for spec in specs {
            match spec {
                serde_yaml::Value::String(s) => {
                    // Direct capture: field name used as-is
                    mappings.push((s.clone(), s.clone()));
                }
                serde_yaml::Value::Mapping(m) => {
                    // Rename: { SourceField: target_name }
                    for (k, v) in m {
                        if let (Some(src), Some(tgt)) = (k.as_str(), v.as_str()) {
                            mappings.push((src.to_string(), tgt.to_string()));
                        }
                    }
                }
                _ => {}
            }
        }
        mappings
    }
}

/// Default resource type value
fn default_resource_type() -> String {
    "resource".to_string()
}

/// Represents a property of a resource.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Property {
    /// Name of the property
    pub name: String,

    /// Value of the property - can be a string or a complex structure
    #[serde(default)]
    pub value: Option<serde_yaml::Value>,

    /// Environment-specific values
    #[serde(default)]
    pub values: Option<HashMap<String, PropertyValue>>,

    /// Description of the property
    #[serde(default)]
    pub description: String,

    /// Items to merge with the value
    #[serde(default)]
    pub merge: Option<Vec<String>>,

    /// When true, the rendered value is masked in all log output
    /// (dry-run queries, --show-queries, debug logs)
    #[serde(default)]
    pub protected: bool,
}

/// Represents a value for a property in a specific environment.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PropertyValue {
    /// Value for the property in this environment - can be a string or complex structure
    pub value: serde_yaml::Value,
}

/// Check if a string is a `file()` directive and extract the path.
/// Matches patterns like `file(path/to/file.json)` with optional whitespace.
fn parse_file_directive(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if trimmed.starts_with("file(") && trimmed.ends_with(')') {
        let inner = trimmed[5..trimmed.len() - 1].trim();
        if !inner.is_empty() {
            return Some(inner);
        }
    }
    None
}

/// Recursively walk a `serde_yaml::Value` tree and resolve any `file()` directives.
///
/// A `file()` directive is a string value of the form `file(relative/path.json)`.
/// When encountered, the referenced file is read, parsed (as JSON or YAML depending
/// on extension), and its contents replace the directive in the value tree.
///
/// This enables modularizing large manifest values (e.g., policy statements) into
/// separate files:
///
/// ```yaml
/// - name: policies
///   value:
///     - PolicyDocument:
///         Statement:
///           - file(policies/statement1.json)   # inserts a JSON object
///           - file(policies/statement2.json)   # inserts a JSON object
///         Version: '2012-10-17'
/// ```
fn resolve_file_directives(value: &mut serde_yaml::Value, base_dir: &Path) -> ManifestResult<()> {
    match value {
        serde_yaml::Value::String(s) => {
            if let Some(file_path) = parse_file_directive(s) {
                let resolved = load_file_contents(file_path, base_dir)?;
                *value = resolved;
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            let mut i = 0;
            while i < seq.len() {
                resolve_file_directives(&mut seq[i], base_dir)?;
                i += 1;
            }
        }
        serde_yaml::Value::Mapping(map) => {
            // Collect keys first to avoid borrow issues
            let keys: Vec<serde_yaml::Value> = map.keys().cloned().collect();
            for key in keys {
                if let Some(val) = map.get_mut(&key) {
                    resolve_file_directives(val, base_dir)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Load and parse a file referenced by a `file()` directive.
/// Supports JSON (.json) and YAML (.yml, .yaml) files.
fn load_file_contents(file_path: &str, base_dir: &Path) -> ManifestResult<serde_yaml::Value> {
    let full_path = base_dir.join(file_path);

    debug!(
        "Resolving file() directive: {} -> {:?}",
        file_path, full_path
    );

    let content = fs::read_to_string(&full_path).map_err(|e| {
        ManifestError::FileIncludeError(format!(
            "cannot read '{}' (resolved to {:?}): {}",
            file_path, full_path, e
        ))
    })?;

    let ext = full_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let mut parsed: serde_yaml::Value = match ext {
        "json" => {
            let json_val: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
                ManifestError::FileIncludeError(format!(
                    "failed to parse JSON file '{}': {}",
                    file_path, e
                ))
            })?;
            // Convert JSON -> YAML value
            serde_yaml::to_value(&json_val).map_err(|e| {
                ManifestError::FileIncludeError(format!(
                    "failed to convert JSON to YAML value for '{}': {}",
                    file_path, e
                ))
            })?
        }
        "yml" | "yaml" => serde_yaml::from_str(&content).map_err(|e| {
            ManifestError::FileIncludeError(format!(
                "failed to parse YAML file '{}': {}",
                file_path, e
            ))
        })?,
        _ => {
            // Default: try JSON first, fall back to YAML
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content) {
                serde_yaml::to_value(&json_val).map_err(|e| {
                    ManifestError::FileIncludeError(format!(
                        "failed to convert parsed content for '{}': {}",
                        file_path, e
                    ))
                })?
            } else {
                serde_yaml::from_str(&content).map_err(|e| {
                    ManifestError::FileIncludeError(format!(
                        "failed to parse file '{}' as JSON or YAML: {}",
                        file_path, e
                    ))
                })?
            }
        }
    };

    // Recursively resolve any nested file() directives in the loaded content
    resolve_file_directives(&mut parsed, base_dir)?;

    Ok(parsed)
}

/// Resolve all `file()` directives in a manifest's globals and resource properties.
fn resolve_manifest_file_directives(
    manifest: &mut Manifest,
    base_dir: &Path,
) -> ManifestResult<()> {
    // Resolve in globals
    for global in &mut manifest.globals {
        resolve_file_directives(&mut global.value, base_dir)?;
    }

    // Resolve in resource properties
    for resource in &mut manifest.resources {
        for prop in &mut resource.props {
            if let Some(ref mut value) = prop.value {
                resolve_file_directives(value, base_dir)?;
            }
            if let Some(ref mut values) = prop.values {
                for env_val in values.values_mut() {
                    resolve_file_directives(&mut env_val.value, base_dir)?;
                }
            }
        }
    }

    Ok(())
}

impl Manifest {
    /// Loads a manifest file from the specified path.
    /// After parsing, resolves any `file()` directives in property values.
    /// File paths in `file()` directives are resolved relative to the `resources/`
    /// directory under the manifest's parent directory.
    pub fn load_from_file(path: &Path) -> ManifestResult<Self> {
        let content = fs::read_to_string(path)?;
        let mut manifest: Manifest = serde_yaml::from_str(&content)?;

        // Resolve file() directives relative to <stack_dir>/resources/
        let stack_dir = path.parent().unwrap_or(Path::new("."));
        let resources_dir = stack_dir.join("resources");
        resolve_manifest_file_directives(&mut manifest, &resources_dir)?;

        // Validate the manifest
        manifest.validate()?;

        Ok(manifest)
    }

    /// Loads a manifest file from the specified stack directory.
    pub fn load_from_stack_dir(stack_dir: &Path) -> ManifestResult<Self> {
        let manifest_path = stack_dir.join("stackql_manifest.yml");
        Self::load_from_file(&manifest_path)
    }

    /// Validates the manifest for required fields and correctness.
    fn validate(&self) -> ManifestResult<()> {
        // Check required fields
        if self.name.is_empty() {
            return Err(ManifestError::MissingField("name".to_string()));
        }

        if self.providers.is_empty() {
            return Err(ManifestError::MissingField("providers".to_string()));
        }

        // Validate each resource
        for resource in &self.resources {
            if resource.name.is_empty() {
                return Err(ManifestError::MissingField("resource.name".to_string()));
            }

            // Validate properties
            for prop in &resource.props {
                if prop.name.is_empty() {
                    return Err(ManifestError::MissingField("property.name".to_string()));
                }

                // Each property must have either a value, values, or merge
                if prop.value.is_none() && prop.values.is_none() && prop.merge.is_none() {
                    return Err(ManifestError::MissingField(format!(
                        "Property '{}' in resource '{}' has no value, values, or merge",
                        prop.name, resource.name
                    )));
                }
            }
        }

        // Run the extensible validation rule-set
        if let Err(errors) = crate::resource::validation::validate_manifest(self) {
            let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            return Err(ManifestError::ValidationFailed(messages.join("; ")));
        }

        Ok(())
    }

    /// Gets the resource query file path for a resource.
    pub fn get_resource_query_path(&self, stack_dir: &Path, resource: &Resource) -> PathBuf {
        let file_name = match &resource.file {
            Some(file) => file.clone(),
            _none => format!("{}.iql", resource.name),
        };

        stack_dir.join("resources").join(file_name)
    }

    /// Gets the value of a property in a specific environment.
    pub fn get_property_value<'a>(
        property: &'a Property,
        env: &str,
    ) -> Option<&'a serde_yaml::Value> {
        // Direct value takes precedence
        if let Some(ref value) = property.value {
            return Some(value);
        }

        // Fall back to environment-specific values
        if let Some(ref values) = property.values {
            if let Some(env_value) = values.get(env) {
                return Some(&env_value.value);
            }
        }

        None
    }

    /// Finds a resource by name.
    pub fn find_resource(&self, name: &str) -> Option<&Resource> {
        self.resources.iter().find(|r| r.name == name)
    }

    /// Gets global variables as a map of name to YAML value.
    pub fn globals_as_map(&self) -> HashMap<String, serde_yaml::Value> {
        self.globals
            .iter()
            .map(|g| (g.name.clone(), g.value.clone()))
            .collect()
    }

    /// Loads a manifest file from the specified stack directory or exits with an error message.
    pub fn load_from_dir_or_exit(stack_dir: &str) -> Self {
        debug!("Loading manifest file from stack directory: {}", stack_dir);

        match Self::load_from_stack_dir(Path::new(stack_dir)) {
            Ok(manifest) => {
                debug!("Stack name: {}", manifest.name);
                debug!("Stack description: {}", manifest.description);
                debug!("Providers: {:?}", manifest.providers);
                debug!("Resources count: {}", manifest.resources.len());
                manifest
            }
            Err(err) => {
                error!("Failed to load manifest: {}", err);
                process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: create a temp directory structure for testing file() directives.
    fn setup_test_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        // Create resources/ subdirectory (file() resolves relative to this)
        fs::create_dir_all(dir.path().join("resources")).unwrap();
        dir
    }

    #[test]
    fn test_parse_file_directive() {
        assert_eq!(
            parse_file_directive("file(foo/bar.json)"),
            Some("foo/bar.json")
        );
        assert_eq!(
            parse_file_directive("  file( foo/bar.json )  "),
            Some("foo/bar.json")
        );
        assert_eq!(parse_file_directive("file()"), None);
        assert_eq!(parse_file_directive("not a directive"), None);
        assert_eq!(parse_file_directive("file("), None);
        assert_eq!(parse_file_directive("files(foo.json)"), None);
    }

    #[test]
    fn test_resolve_file_directive_json_object() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");
        fs::write(
            resources_dir.join("stmt.json"),
            r#"{"Effect": "Allow", "Action": ["s3:GetObject"], "Resource": ["*"]}"#,
        )
        .unwrap();

        let mut value = serde_yaml::Value::String("file(stmt.json)".to_string());
        resolve_file_directives(&mut value, &resources_dir).unwrap();

        // Should be a mapping now, not a string
        assert!(value.is_mapping(), "Expected mapping, got: {:?}", value);
        let map = value.as_mapping().unwrap();
        assert_eq!(
            map.get(serde_yaml::Value::String("Effect".into())),
            Some(&serde_yaml::Value::String("Allow".into()))
        );
    }

    #[test]
    fn test_resolve_file_directive_json_array() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");
        fs::write(
            resources_dir.join("statements.json"),
            r#"[{"Effect": "Allow"}, {"Effect": "Deny"}]"#,
        )
        .unwrap();

        let mut value = serde_yaml::Value::String("file(statements.json)".to_string());
        resolve_file_directives(&mut value, &resources_dir).unwrap();

        assert!(value.is_sequence(), "Expected sequence, got: {:?}", value);
        let seq = value.as_sequence().unwrap();
        assert_eq!(seq.len(), 2);
    }

    #[test]
    fn test_resolve_file_directive_yaml_file() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");
        fs::write(
            resources_dir.join("stmt.yaml"),
            "Effect: Allow\nAction:\n  - s3:GetObject\nResource:\n  - \"*\"\n",
        )
        .unwrap();

        let mut value = serde_yaml::Value::String("file(stmt.yaml)".to_string());
        resolve_file_directives(&mut value, &resources_dir).unwrap();

        assert!(value.is_mapping(), "Expected mapping, got: {:?}", value);
    }

    #[test]
    fn test_resolve_file_directive_in_sequence() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");
        fs::write(
            resources_dir.join("s1.json"),
            r#"{"Sid": "stmt1", "Effect": "Allow"}"#,
        )
        .unwrap();
        fs::write(
            resources_dir.join("s2.json"),
            r#"{"Sid": "stmt2", "Effect": "Deny"}"#,
        )
        .unwrap();

        let mut value = serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::String("file(s1.json)".to_string()),
            serde_yaml::Value::String("file(s2.json)".to_string()),
        ]);
        resolve_file_directives(&mut value, &resources_dir).unwrap();

        let seq = value.as_sequence().unwrap();
        assert_eq!(seq.len(), 2);
        assert!(seq[0].is_mapping());
        assert!(seq[1].is_mapping());
    }

    #[test]
    fn test_resolve_file_directive_nested_in_mapping() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");
        fs::write(resources_dir.join("stmts.json"), r#"[{"Effect": "Allow"}]"#).unwrap();

        let mut map = serde_yaml::Mapping::new();
        map.insert(
            serde_yaml::Value::String("Statement".into()),
            serde_yaml::Value::String("file(stmts.json)".to_string()),
        );
        map.insert(
            serde_yaml::Value::String("Version".into()),
            serde_yaml::Value::String("2012-10-17".into()),
        );
        let mut value = serde_yaml::Value::Mapping(map);

        resolve_file_directives(&mut value, &resources_dir).unwrap();

        let resolved_map = value.as_mapping().unwrap();
        let statement = resolved_map
            .get(serde_yaml::Value::String("Statement".into()))
            .unwrap();
        assert!(statement.is_sequence(), "Statement should be a sequence");
    }

    #[test]
    fn test_resolve_file_directive_subdirectory() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");
        fs::create_dir_all(resources_dir.join("policies")).unwrap();
        fs::write(
            resources_dir.join("policies/stmt.json"),
            r#"{"Effect": "Allow"}"#,
        )
        .unwrap();

        let mut value = serde_yaml::Value::String("file(policies/stmt.json)".to_string());
        resolve_file_directives(&mut value, &resources_dir).unwrap();

        assert!(value.is_mapping());
    }

    #[test]
    fn test_resolve_file_directive_missing_file() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");

        let mut value = serde_yaml::Value::String("file(nonexistent.json)".to_string());
        let result = resolve_file_directives(&mut value, &resources_dir);

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("nonexistent.json"));
    }

    #[test]
    fn test_resolve_file_directive_leaves_non_directives_alone() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");

        let mut value = serde_yaml::Value::String("just a normal string".to_string());
        resolve_file_directives(&mut value, &resources_dir).unwrap();

        assert_eq!(
            value,
            serde_yaml::Value::String("just a normal string".into())
        );
    }

    #[test]
    fn test_resolve_file_directive_leaves_template_vars_alone() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");

        let mut value =
            serde_yaml::Value::String("{{ stack_name }}-{{ stack_env }}-policy".to_string());
        resolve_file_directives(&mut value, &resources_dir).unwrap();

        assert_eq!(
            value,
            serde_yaml::Value::String("{{ stack_name }}-{{ stack_env }}-policy".into())
        );
    }

    #[test]
    fn test_load_manifest_with_file_directives() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");

        // Write a JSON file to be included
        fs::write(
            resources_dir.join("test_stmt.json"),
            r#"[{"Effect": "Allow", "Action": ["s3:*"], "Resource": ["*"]}]"#,
        )
        .unwrap();

        // Write a manifest that uses file()
        let manifest_content = r#"
version: 1
name: test-stack
description: test
providers:
  - aws
resources:
  - name: test_resource
    props:
      - name: policies
        value:
          - PolicyDocument:
              Statement: file(test_stmt.json)
              Version: '2012-10-17'
            PolicyName: test-policy
"#;
        fs::write(dir.path().join("stackql_manifest.yml"), manifest_content).unwrap();

        let manifest = Manifest::load_from_stack_dir(dir.path()).unwrap();
        let resource = manifest.find_resource("test_resource").unwrap();
        let policies_prop = resource
            .props
            .iter()
            .find(|p| p.name == "policies")
            .unwrap();
        let value = policies_prop.value.as_ref().unwrap();

        // The value should be a sequence with one policy
        let seq = value.as_sequence().unwrap();
        assert_eq!(seq.len(), 1);

        // The PolicyDocument.Statement should be a resolved array
        let policy = seq[0].as_mapping().unwrap();
        let doc = policy
            .get(serde_yaml::Value::String("PolicyDocument".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        let statement = doc
            .get(serde_yaml::Value::String("Statement".into()))
            .unwrap();
        assert!(
            statement.is_sequence(),
            "Statement should be resolved to a sequence, got: {:?}",
            statement
        );
    }

    #[test]
    fn test_skip_on_delete_defaults_false_and_parses_true() {
        let dir = setup_test_dir();
        let manifest_content = r#"
version: 1
name: test-stack
providers:
  - databricks_workspace
resources:
  - name: workspace
    props: []
  - name: workspace_ready
    type: query
    skip_on_delete: true
    props: []
    sql: SELECT 1 AS workspace_principal
    exports:
      - workspace_principal
"#;
        fs::write(dir.path().join("stackql_manifest.yml"), manifest_content).unwrap();

        let manifest = Manifest::load_from_stack_dir(dir.path()).unwrap();
        assert!(!manifest.find_resource("workspace").unwrap().skip_on_delete);
        assert!(
            manifest
                .find_resource("workspace_ready")
                .unwrap()
                .skip_on_delete
        );
    }

    #[test]
    fn test_nested_file_directives() {
        let dir = setup_test_dir();
        let resources_dir = dir.path().join("resources");

        // A JSON file that itself references nothing (nested resolution test base)
        fs::write(
            resources_dir.join("inner.json"),
            r#"{"Action": ["s3:GetObject"]}"#,
        )
        .unwrap();

        // An outer YAML file that includes the inner file
        fs::write(
            resources_dir.join("outer.yaml"),
            "Effect: Allow\nDetails: file(inner.json)\n",
        )
        .unwrap();

        let mut value = serde_yaml::Value::String("file(outer.yaml)".to_string());
        resolve_file_directives(&mut value, &resources_dir).unwrap();

        let map = value.as_mapping().unwrap();
        let details = map
            .get(serde_yaml::Value::String("Details".into()))
            .unwrap();
        assert!(
            details.is_mapping(),
            "Nested file() should be resolved, got: {:?}",
            details
        );
    }
}
