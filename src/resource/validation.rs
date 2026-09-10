// resource/validation.rs

//! # Manifest Validation Module
//!
//! Validates a parsed manifest against a set of rules before any command
//! (build, test, teardown) proceeds.  Each rule is a standalone function
//! that returns a list of validation errors.  New rules can be added by
//! implementing a function with the signature
//! `fn(manifest: &Manifest) -> Vec<ValidationError>` and appending it to
//! the `RULES` array in [`validate_manifest`].

use std::collections::HashMap;

use crate::resource::manifest::Manifest;

/// A single validation error with a rule name and human-readable message.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    /// Machine-readable rule identifier (e.g. `"unique_resource_names"`).
    pub rule: String,
    /// Human-readable description of the violation.
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.rule, self.message)
    }
}

/// Validate a manifest against all registered rules.
///
/// Returns `Ok(())` when the manifest is valid, or `Err(Vec<ValidationError>)`
/// containing every violation found (rules are not short-circuited).
pub fn validate_manifest(manifest: &Manifest) -> Result<(), Vec<ValidationError>> {
    // Register rules here.  Each entry is a function that accepts a &Manifest
    // and returns a Vec<ValidationError>.  Adding a new rule is as simple as
    // appending another entry to this list.
    let rules: Vec<fn(&Manifest) -> Vec<ValidationError>> = vec![rule_unique_resource_names];

    let errors: Vec<ValidationError> = rules.iter().flat_map(|rule| rule(manifest)).collect();

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// Resource names within a manifest must be unique.
///
/// Because resource-scoped exports use the resource name as a namespace
/// (e.g. `{{ my_resource.var }}`), duplicate names would create ambiguous
/// references and silently overwrite immutable scoped exports.
fn rule_unique_resource_names(manifest: &Manifest) -> Vec<ValidationError> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut errors = Vec::new();

    for (idx, resource) in manifest.resources.iter().enumerate() {
        if let Some(&first_idx) = seen.get(resource.name.as_str()) {
            errors.push(ValidationError {
                rule: "unique_resource_names".to_string(),
                message: format!(
                    "Duplicate resource name '{}' at index {} (first seen at index {})",
                    resource.name, idx, first_idx
                ),
            });
        } else {
            seen.insert(&resource.name, idx);
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::manifest::{Manifest, Resource};
    use std::fs;

    /// Helper to build a minimal valid manifest with the given resource names.
    fn manifest_with_resources(names: &[&str]) -> Manifest {
        Manifest {
            version: 1,
            name: "test-stack".to_string(),
            description: String::new(),
            providers: vec!["aws".to_string()],
            globals: vec![],
            resources: names
                .iter()
                .map(|n| Resource {
                    name: n.to_string(),
                    r#type: "resource".to_string(),
                    file: None,
                    sql: None,
                    run: None,
                    props: vec![],
                    exports: vec![],
                    protected: vec![],
                    description: String::new(),
                    r#if: None,
                    skip_validation: None,
                    skip_on_delete: false,
                    auth: None,
                    return_vals: None,
                })
                .collect(),
            exports: vec![],
        }
    }

    // --------------------------------------------------
    // rule_unique_resource_names
    // --------------------------------------------------

    #[test]
    fn test_unique_resource_names_valid() {
        let manifest = manifest_with_resources(&["vpc", "subnet", "security_group"]);
        let result = validate_manifest(&manifest);
        assert!(result.is_ok(), "Expected valid manifest, got: {:?}", result);
    }

    #[test]
    fn test_unique_resource_names_empty_resources() {
        let manifest = manifest_with_resources(&[]);
        let result = validate_manifest(&manifest);
        assert!(
            result.is_ok(),
            "Empty resources list should be valid, got: {:?}",
            result
        );
    }

    #[test]
    fn test_unique_resource_names_single_resource() {
        let manifest = manifest_with_resources(&["only_one"]);
        let result = validate_manifest(&manifest);
        assert!(result.is_ok());
    }

    #[test]
    fn test_unique_resource_names_duplicate() {
        let manifest = manifest_with_resources(&["vpc", "subnet", "vpc"]);
        let result = validate_manifest(&manifest);
        assert!(result.is_err(), "Expected duplicate to be detected");

        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].rule, "unique_resource_names");
        assert!(
            errors[0].message.contains("vpc"),
            "Error should mention the duplicate name, got: {}",
            errors[0].message
        );
    }

    #[test]
    fn test_unique_resource_names_multiple_duplicates() {
        let manifest = manifest_with_resources(&["a", "b", "a", "c", "b", "a"]);
        let result = validate_manifest(&manifest);
        assert!(result.is_err());

        let errors = result.unwrap_err();
        // "a" appears at indices 0, 2, 5 → 2 errors
        // "b" appears at indices 1, 4 → 1 error
        assert_eq!(
            errors.len(),
            3,
            "Expected 3 duplicate errors, got: {:?}",
            errors
        );
    }

    // --------------------------------------------------
    // validate_manifest integration
    // --------------------------------------------------

    #[test]
    fn test_validate_manifest_reports_all_rule_violations() {
        // Currently only one rule, but this test verifies the aggregation logic
        let manifest = manifest_with_resources(&["dup", "dup"]);
        let errors = validate_manifest(&manifest).unwrap_err();
        assert!(!errors.is_empty());
        assert_eq!(errors[0].rule, "unique_resource_names");
    }

    // --------------------------------------------------
    // YAML file-based tests (positive & negative)
    // --------------------------------------------------

    /// Helper: create a temp stack directory with a manifest and empty resources/.
    fn write_manifest_file(content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("resources")).unwrap();
        fs::write(dir.path().join("stackql_manifest.yml"), content).unwrap();
        dir
    }

    #[test]
    fn test_valid_manifest_file_passes_validation() {
        let dir = write_manifest_file(
            r#"
version: 1
name: valid-stack
description: a valid manifest
providers:
  - aws
resources:
  - name: vpc
    props:
      - name: cidr
        value: "10.0.0.0/16"
  - name: subnet
    props:
      - name: cidr
        value: "10.0.1.0/24"
  - name: security_group
    props:
      - name: description
        value: "web traffic"
"#,
        );

        let manifest = Manifest::load_from_stack_dir(dir.path()).unwrap();
        let result = validate_manifest(&manifest);
        assert!(
            result.is_ok(),
            "Valid manifest should pass, got: {:?}",
            result
        );
    }

    #[test]
    fn test_duplicate_names_manifest_file_fails_validation() {
        let dir = write_manifest_file(
            r#"
version: 1
name: bad-stack
description: manifest with duplicate resource names
providers:
  - aws
resources:
  - name: my_bucket
    props:
      - name: bucket_name
        value: "bucket-one"
  - name: my_role
    props:
      - name: role_name
        value: "role-one"
  - name: my_bucket
    props:
      - name: bucket_name
        value: "bucket-two"
"#,
        );

        // load_from_stack_dir already runs validate_manifest internally,
        // so a manifest with duplicate names should fail to load.
        let result = Manifest::load_from_stack_dir(dir.path());
        assert!(result.is_err(), "Duplicate names should fail to load");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("my_bucket"),
            "Error should mention the duplicate name, got: {}",
            err_msg
        );
    }
}
