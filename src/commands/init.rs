use crate::utils::display::print_unicode_box;
use clap::{Arg, ArgAction, ArgMatches, Command};
use colored::*;
use reqwest::blocking::Client;
use reqwest::StatusCode;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use tera::{Context, Tera};

// The base URL for GitHub template repository
const GITHUB_TEMPLATE_BASE: &str =
    "https://raw.githubusercontent.com/stackql/stackql-deploy-rust/main/template-hub/";

// AWS templates
const AWS_RESOURCE_TEMPLATE: &str =
    include_str!("../../template-hub/aws/starter/resources/example_vpc.iql.template");
const AWS_MANIFEST_TEMPLATE: &str =
    include_str!("../../template-hub/aws/starter/stackql_manifest.yml.template");
const AWS_README_TEMPLATE: &str = include_str!("../../template-hub/aws/starter/README.md.template");

// Azure templates
const AZURE_RESOURCE_TEMPLATE: &str =
    include_str!("../../template-hub/azure/starter/resources/example_res_grp.iql.template");
const AZURE_MANIFEST_TEMPLATE: &str =
    include_str!("../../template-hub/azure/starter/stackql_manifest.yml.template");
const AZURE_README_TEMPLATE: &str =
    include_str!("../../template-hub/azure/starter/README.md.template");

// Google templates
const GOOGLE_RESOURCE_TEMPLATE: &str =
    include_str!("../../template-hub/google/starter/resources/example_vpc.iql.template");
const GOOGLE_MANIFEST_TEMPLATE: &str =
    include_str!("../../template-hub/google/starter/stackql_manifest.yml.template");
const GOOGLE_README_TEMPLATE: &str =
    include_str!("../../template-hub/google/starter/README.md.template");

const DEFAULT_PROVIDER: &str = "azure";
const SUPPORTED_PROVIDERS: [&str; 3] = ["aws", "google", "azure"];

// Define template sources
enum TemplateSource {
    Embedded(String), // Built-in template using one of the supported providers
    Custom(String),   // Custom template path or URL
}

impl TemplateSource {
    // Get provider name (for embedded) or template path (for custom)
    #[allow(dead_code)]
    fn provider_or_path(&self) -> &str {
        match self {
            TemplateSource::Embedded(provider) => provider,
            TemplateSource::Custom(path) => path,
        }
    }

    // Determine sample resource name based on provider or template
    fn get_sample_res_name(&self) -> &str {
        match self {
            TemplateSource::Embedded(provider) => match provider.as_str() {
                "google" => "example_vpc",
                "azure" => "example_res_grp",
                "aws" => "example_vpc",
                _ => "example_resource",
            },
            TemplateSource::Custom(path) => {
                // Try to determine resource name based on path
                if path.contains("aws") {
                    "example_vpc"
                } else if path.contains("azure") {
                    "example_res_grp"
                } else if path.contains("google") {
                    "example_vpc"
                } else {
                    "example_resource"
                }
            }
        }
    }
}

pub fn command() -> Command {
    Command::new("init")
        .about("Initialize a new stackql-deploy project structure")
        .arg(
            Arg::new("stack_name")
                .help("Name of the new stack project")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::new("provider")
                .short('p')
                .long("provider")
                .help("Specify a provider (aws, azure, google)")
                .action(ArgAction::Set)
                .conflicts_with("template"),
        )
        .arg(
            Arg::new("template")
                .short('t')
                .long("template")
                .help("Template path or URL (e.g., 'aws/starter' or full GitHub URL)")
                .action(ArgAction::Set)
                .conflicts_with("provider"),
        )
        .arg(
            Arg::new("env")
                .short('e')
                .long("env")
                .help("Environment name (dev, test, prod)")
                .default_value("dev")
                .action(ArgAction::Set),
        )
}

pub fn execute(matches: &ArgMatches) {
    print_unicode_box("🚀 Initializing new project...");

    let stack_name = matches
        .get_one::<String>("stack_name")
        .expect("Stack name is required");

    let stack_name = stack_name.replace('_', "-").to_lowercase();

    let env = matches
        .get_one::<String>("env")
        .expect("Environment defaulted to dev")
        .to_string();

    // Check if using custom template or provider
    let template_source = if let Some(template_path) = matches.get_one::<String>("template") {
        TemplateSource::Custom(template_path.clone())
    } else {
        // Get the provider with validation
        let provider = validate_provider(matches.get_one::<String>("provider").map(|s| s.as_str()));
        TemplateSource::Embedded(provider)
    };

    // Create project structure
    match create_project_structure(&stack_name, &template_source, &env) {
        Ok(_) => {
            println!(
                "{}",
                format!("Project {} initialized successfully.", stack_name).green()
            );
        }
        Err(e) => {
            eprintln!("{}", format!("Error initializing project: {}", e).red());
        }
    }
}

fn validate_provider(provider: Option<&str>) -> String {
    let supported: HashSet<&str> = SUPPORTED_PROVIDERS.iter().cloned().collect();

    match provider {
        Some(p) if supported.contains(p) => p.to_string(),
        Some(p) => {
            println!("{}", format!(
                "Provider '{}' is not supported for `init`, supported providers are: {}, defaulting to `{}`",
                p, SUPPORTED_PROVIDERS.join(", "), DEFAULT_PROVIDER
            ).yellow());
            DEFAULT_PROVIDER.to_string()
        }
        _none => {
            // Silently default to DEFAULT_PROVIDER
            DEFAULT_PROVIDER.to_string()
        }
    }
}

// Function to fetch template content from URL
fn fetch_template(url: &str) -> Result<String, String> {
    let client = Client::new();
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to fetch template: {}", e))?;

    // Check if response is successful (status code 200-299)
    if !response.status().is_success() {
        // Handle 404 and other error status codes
        if response.status() == StatusCode::NOT_FOUND {
            return Err(format!("Template not found at URL: {}", url));
        } else {
            return Err(format!(
                "Failed to fetch template: HTTP status {}",
                response.status()
            ));
        }
    }

    response
        .text()
        .map_err(|e| format!("Failed to read template content: {}", e))
}

// Normalize GitHub URL to raw content URL
fn normalize_github_url(url: &str) -> String {
    if url.starts_with("https://github.com") {
        // Convert github.com URL to raw.githubusercontent.com
        url.replace("https://github.com", "https://raw.githubusercontent.com")
            .replace("/tree/", "/")
    } else {
        url.to_string()
    }
}

// Build full URL or path for templates
fn build_template_url(template_path: &str, resource_name: &str, file_type: &str) -> String {
    // Check if template_path is an absolute URL
    if template_path.starts_with("http://") || template_path.starts_with("https://") {
        let base_url = normalize_github_url(template_path);

        match file_type {
            "resource" => format!("{}/resources/{}.iql.template", base_url, resource_name),
            "manifest" => format!("{}/stackql_manifest.yml.template", base_url),
            "readme" => format!("{}/README.md.template", base_url),
            _ => base_url,
        }
    } else {
        // It's a relative path, prepend with GitHub template base
        let base_url = format!("{}{}", GITHUB_TEMPLATE_BASE, template_path);

        match file_type {
            "resource" => format!("{}/resources/{}.iql.template", base_url, resource_name),
            "manifest" => format!("{}/stackql_manifest.yml.template", base_url),
            "readme" => format!("{}/README.md.template", base_url),
            _ => base_url,
        }
    }
}

fn get_template_content(
    template_source: &TemplateSource,
    template_type: &str,
    resource_name: &str,
) -> Result<String, String> {
    match template_source {
        TemplateSource::Embedded(provider) => {
            // Use embedded templates
            match (provider.as_str(), template_type) {
                ("aws", "resource") => Ok(AWS_RESOURCE_TEMPLATE.to_string()),
                ("aws", "manifest") => Ok(AWS_MANIFEST_TEMPLATE.to_string()),
                ("aws", "readme") => Ok(AWS_README_TEMPLATE.to_string()),
                ("azure", "resource") => Ok(AZURE_RESOURCE_TEMPLATE.to_string()),
                ("azure", "manifest") => Ok(AZURE_MANIFEST_TEMPLATE.to_string()),
                ("azure", "readme") => Ok(AZURE_README_TEMPLATE.to_string()),
                ("google", "resource") => Ok(GOOGLE_RESOURCE_TEMPLATE.to_string()),
                ("google", "manifest") => Ok(GOOGLE_MANIFEST_TEMPLATE.to_string()),
                ("google", "readme") => Ok(GOOGLE_README_TEMPLATE.to_string()),
                _ => Err(format!(
                    "Unsupported provider or template type: {}, {}",
                    provider, template_type
                )),
            }
        }
        TemplateSource::Custom(path) => {
            // Fetch template from URL or path
            let template_url = build_template_url(path, resource_name, template_type);

            // Fetch content from URL
            println!(
                "{}",
                format!("Fetching template from: {}", template_url).blue()
            );
            fetch_template(&template_url)
        }
    }
}

fn create_project_structure(
    stack_name: &str,
    template_source: &TemplateSource,
    env: &str,
) -> Result<(), String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?;
    let base_path = cwd.join(stack_name);

    // Check if directory already exists
    if base_path.exists() {
        return Err(format!("Directory '{}' already exists", stack_name));
    }

    // Determine sample resource name based on provider
    let sample_res_name = template_source.get_sample_res_name();

    // Set up template context
    let mut context = Context::new();
    context.insert("stack_name", stack_name);
    context.insert("stack_env", env);

    // First validate that all templates can be fetched before creating any directories
    let manifest_template = get_template_content(template_source, "manifest", "")?;
    let readme_template = get_template_content(template_source, "readme", "")?;
    let resource_template = get_template_content(template_source, "resource", sample_res_name)?;

    // Now create directories
    let resource_dir = base_path.join("resources");
    fs::create_dir_all(&resource_dir)
        .map_err(|e| format!("Failed to create directories: {}", e))?;

    // Create files
    create_manifest_file(&base_path, &manifest_template, &context)?;
    create_readme_file(&base_path, &readme_template, &context)?;
    create_resource_file(&resource_dir, sample_res_name, &resource_template, &context)?;

    Ok(())
}

fn create_resource_file(
    resource_dir: &Path,
    sample_res_name: &str,
    template_str: &str,
    context: &Context,
) -> Result<(), String> {
    // Render template with Tera
    let resource_content = render_template(template_str, context)
        .map_err(|e| format!("Template rendering error: {}", e))?;

    let resource_path = resource_dir.join(format!("{}.iql", sample_res_name));
    let mut file = fs::File::create(resource_path)
        .map_err(|e| format!("Failed to create resource file: {}", e))?;

    file.write_all(resource_content.as_bytes())
        .map_err(|e| format!("Failed to write to resource file: {}", e))?;

    Ok(())
}

fn create_manifest_file(
    base_path: &Path,
    template_str: &str,
    context: &Context,
) -> Result<(), String> {
    // Render template with Tera
    let manifest_content = render_template(template_str, context)
        .map_err(|e| format!("Template rendering error: {}", e))?;

    let manifest_path = base_path.join("stackql_manifest.yml");
    let mut file = fs::File::create(manifest_path)
        .map_err(|e| format!("Failed to create manifest file: {}", e))?;

    file.write_all(manifest_content.as_bytes())
        .map_err(|e| format!("Failed to write to manifest file: {}", e))?;

    Ok(())
}

fn create_readme_file(
    base_path: &Path,
    template_str: &str,
    context: &Context,
) -> Result<(), String> {
    // Render template with Tera
    let readme_content = render_template(template_str, context)
        .map_err(|e| format!("Template rendering error: {}", e))?;

    let readme_path = base_path.join("README.md");
    let mut file = fs::File::create(readme_path)
        .map_err(|e| format!("Failed to create README file: {}", e))?;

    file.write_all(readme_content.as_bytes())
        .map_err(|e| format!("Failed to write to README file: {}", e))?;

    Ok(())
}

fn render_template(template_str: &str, context: &Context) -> Result<String, String> {
    // Create a one-off Tera instance for rendering a single template
    let mut tera = Tera::default();
    tera.add_raw_template("template", template_str)
        .map_err(|e| format!("Failed to add template: {}", e))?;

    tera.render("template", context)
        .map_err(|e| format!("Failed to render template: {}", e))
}
