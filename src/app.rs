/// Application name
pub const APP_NAME: &str = "stackql-deploy";

/// Application version
pub const APP_VERSION: &str = "0.1.0";

/// Application author
pub const APP_AUTHOR: &str = "Jeffrey Aven <javen@stackql.io>";

/// Application description
pub const APP_DESCRIPTION: &str = "Model driven IaC using stackql";

/// Default server host
pub const DEFAULT_SERVER_HOST: &str = "localhost";

/// Default PostgreSQL server port
pub const DEFAULT_SERVER_PORT: u16 = 5444;

/// Local server addresses
pub const LOCAL_SERVER_ADDRESSES: [&str; 3] = ["localhost", "0.0.0.0", "127.0.0.1"];

/// Default log file name
pub const DEFAULT_LOG_FILE: &str = "stackql.log";

/// Supported cloud providers for init command
pub const SUPPORTED_PROVIDERS: [&str; 3] = ["aws", "google", "azure"];

/// Default provider for init command
pub const DEFAULT_PROVIDER: &str = "azure";

/// StackQL Rust binary name (platform dependent)
#[cfg(target_os = "windows")]
pub const STACKQL_BINARY_NAME: &str = "stackql.exe";

#[cfg(not(target_os = "windows"))]
pub const STACKQL_BINARY_NAME: &str = "stackql";

/// StackQL download URLs by platform
#[cfg(target_os = "windows")]
pub const STACKQL_DOWNLOAD_URL: &str =
    "https://releases.stackql.io/stackql/latest/stackql_windows_amd64.zip";

#[cfg(target_os = "linux")]
pub const STACKQL_DOWNLOAD_URL: &str =
    "https://releases.stackql.io/stackql/latest/stackql_linux_amd64.zip";

#[cfg(target_os = "macos")]
pub const STACKQL_DOWNLOAD_URL: &str =
    "https://storage.googleapis.com/stackql-public-releases/latest/stackql_darwin_multiarch.pkg";

/// Commands that require server management
pub const SERVER_COMMANDS: [&str; 5] = ["build", "test", "plan", "teardown", "shell"];

/// Commands exempt from binary check
pub const EXEMPT_COMMANDS: [&str; 1] = ["init"];

/// The base URL for GitHub template repository
pub const GITHUB_TEMPLATE_BASE: &str =
    "https://raw.githubusercontent.com/stackql/stackql-deploy-rust/main/template-hub/";

/// Template constants for AWS
pub mod aws_templates {
    pub const RESOURCE_TEMPLATE: &str =
        include_str!("../template-hub/aws/starter/resources/example_vpc.iql.template");
    pub const MANIFEST_TEMPLATE: &str =
        include_str!("../template-hub/aws/starter/stackql_manifest.yml.template");
    pub const README_TEMPLATE: &str =
        include_str!("../template-hub/aws/starter/README.md.template");
}

/// Template constants for Azure
pub mod azure_templates {
    pub const RESOURCE_TEMPLATE: &str =
        include_str!("../template-hub/azure/starter/resources/example_res_grp.iql.template");
    pub const MANIFEST_TEMPLATE: &str =
        include_str!("../template-hub/azure/starter/stackql_manifest.yml.template");
    pub const README_TEMPLATE: &str =
        include_str!("../template-hub/azure/starter/README.md.template");
}

/// Template constants for Google
pub mod google_templates {
    pub const RESOURCE_TEMPLATE: &str =
        include_str!("../template-hub/google/starter/resources/example_vpc.iql.template");
    pub const MANIFEST_TEMPLATE: &str =
        include_str!("../template-hub/google/starter/stackql_manifest.yml.template");
    pub const README_TEMPLATE: &str =
        include_str!("../template-hub/google/starter/README.md.template");
}
