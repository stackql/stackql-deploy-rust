use clap::{Command, Arg, ArgAction, ArgMatches};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::collections::HashSet;
use colored::*;
use tera::{Tera, Context};
use crate::utils::display::print_unicode_box;

// AWS templates
const AWS_RESOURCE_TEMPLATE: &str = include_str!("../../templates/aws/resources/example_vpc.iql.template");
const AWS_MANIFEST_TEMPLATE: &str = include_str!("../../templates/aws/stackql_manifest.yml.template");
const AWS_README_TEMPLATE: &str = include_str!("../../templates/aws/README.md.template");

// Azure templates
const AZURE_RESOURCE_TEMPLATE: &str = include_str!("../../templates/azure/resources/example_res_grp.iql.template");
const AZURE_MANIFEST_TEMPLATE: &str = include_str!("../../templates/azure/stackql_manifest.yml.template");
const AZURE_README_TEMPLATE: &str = include_str!("../../templates/azure/README.md.template");

// Google templates
const GOOGLE_RESOURCE_TEMPLATE: &str = include_str!("../../templates/google/resources/example_vpc.iql.template");
const GOOGLE_MANIFEST_TEMPLATE: &str = include_str!("../../templates/google/stackql_manifest.yml.template");
const GOOGLE_README_TEMPLATE: &str = include_str!("../../templates/google/README.md.template");

const DEFAULT_PROVIDER: &str = "azure";
const SUPPORTED_PROVIDERS: [&str; 3] = ["aws", "google", "azure"];

pub fn command() -> Command {
    Command::new("init")
        .about("Initialize a new stackql-deploy project structure")
        .arg(
            Arg::new("stack_name")
                .help("Name of the new stack project")
                .required(true)
                .index(1)
        )
        .arg(
            Arg::new("provider")
                .short('p')
                .long("provider")
                .help("Specify a provider (aws, azure, google)")
                .action(ArgAction::Set)
        )
        .arg(
            Arg::new("env")
                .short('e')
                .long("env")
                .help("Environment name (dev, test, prod)")
                .default_value("dev")
                .action(ArgAction::Set)
        )
}

pub fn execute(matches: &ArgMatches) {
    print_unicode_box("🚀 Initializing new project...");
    
    let stack_name = matches.get_one::<String>("stack_name")
        .expect("Stack name is required");
    
    let stack_name = stack_name.replace('_', "-").to_lowercase();
    
    let env = matches.get_one::<String>("env")
        .expect("Environment defaulted to dev")
        .to_string();
    
    // Get the provider with validation
    let provider = validate_provider(matches.get_one::<String>("provider").map(|s| s.as_str()));
    
    // Create project structure
    match create_project_structure(&stack_name, &provider, &env) {
        Ok(_) => {
            println!("{}", format!("Project {} initialized successfully.", stack_name).green());
        },
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
        },
        _none => {
            // Silently default to DEFAULT_PROVIDER
            DEFAULT_PROVIDER.to_string()
        }
    }
}

fn create_project_structure(stack_name: &str, provider: &str, env: &str) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?;
    let base_path = cwd.join(stack_name);
    
    // Check if directory already exists
    if base_path.exists() {
        return Err(format!("Directory '{}' already exists", stack_name));
    }
    
    // Create necessary directories
    let resource_dir = base_path.join("resources");
    fs::create_dir_all(&resource_dir).map_err(|e| format!("Failed to create directories: {}", e))?;
    
    // Determine sample resource name based on provider
    let sample_res_name = match provider {
        "google" => "example_vpc",
        "azure" => "example_res_grp",
        "aws" => "example_vpc",
        _ => "example_resource",
    };
    
    // Set up template context
    let mut context = Context::new();
    context.insert("stack_name", stack_name);
    context.insert("stack_env", env);
    
    // Create files
    create_manifest_file(&base_path, provider, &context)?;
    create_readme_file(&base_path, provider, &context)?;
    create_resource_file(&resource_dir, sample_res_name, provider, &context)?;
    
    Ok(())
}

fn create_resource_file(resource_dir: &Path, sample_res_name: &str, provider: &str, context: &Context) -> Result<(), String> {
    let template_str = match provider {
        "aws" => AWS_RESOURCE_TEMPLATE,
        "azure" => AZURE_RESOURCE_TEMPLATE,
        "google" => GOOGLE_RESOURCE_TEMPLATE,
        _ => "-- Example resource\n",
    };
    
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

fn create_manifest_file(base_path: &Path, provider: &str, context: &Context) -> Result<(), String> {
    let template_str = match provider {
        "aws" => AWS_MANIFEST_TEMPLATE,
        "azure" => AZURE_MANIFEST_TEMPLATE,
        "google" => GOOGLE_MANIFEST_TEMPLATE,
        _ => "name: {{stack_name}}\nversion: 0.1.0\ndescription: StackQL IaC project\n",
    };
    
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

fn create_readme_file(base_path: &Path, provider: &str, context: &Context) -> Result<(), String> {
    let template_str = match provider {
        "aws" => AWS_README_TEMPLATE,
        "azure" => AZURE_README_TEMPLATE,
        "google" => GOOGLE_README_TEMPLATE, 
        _ => "# {{stack_name}}\n\nInfrastructure as Code project\n",
    };
    
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