use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub node: NodeConfig,
    pub roles: RolesConfig,
    pub network: NetworkConfig,
    pub discovery: DiscoveryConfig,
    pub security: SecurityConfig,
    pub executor: ExecutorConfig,
    pub capabilities: CapabilitiesConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeConfig {
    pub tier: u8,
    pub availability: String,
    #[serde(default)]
    pub quorum_participant: bool,
    pub data_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RolesConfig {
    #[serde(default = "default_true")]
    pub scheduler: bool,
    #[serde(default = "default_true")]
    pub executor: bool,
    #[serde(default)]
    pub storage: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_bind")]
    pub bind_address: String,
    #[serde(default = "default_grpc_port")]
    pub grpc_port: u16,
    #[serde(default = "default_rest_port")]
    pub rest_port: u16,
    #[serde(default = "default_metrics_port")]
    pub metrics_port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default = "default_true")]
    pub mdns: bool,
    #[serde(default)]
    pub seeds: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    pub mode: String,
    #[serde(default)]
    pub shared_secret: String,
    #[serde(default)]
    pub ca_cert_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutorConfig {
    #[serde(default = "default_max_jobs")]
    pub max_concurrent_jobs: u32,
    #[serde(default = "default_docker_socket")]
    pub docker_socket: String,
    #[serde(default = "default_true")]
    pub cgroups_enabled: bool,
    #[serde(default = "default_output_max")]
    pub output_max_bytes: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CapabilitiesConfig {
    #[serde(default)]
    pub docker: bool,
    #[serde(default)]
    pub java: Option<String>,
    #[serde(default)]
    pub python: Option<String>,
    #[serde(default)]
    pub wasm: bool,
    #[serde(default)]
    pub gpu_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: String,
}

fn default_true() -> bool {
    true
}

fn default_bind() -> String {
    "0.0.0.0".to_string()
}

fn default_grpc_port() -> u16 {
    7947
}

fn default_rest_port() -> u16 {
    7946
}

fn default_metrics_port() -> u16 {
    9090
}

fn default_max_jobs() -> u32 {
    8
}

fn default_docker_socket() -> String {
    "/var/run/docker.sock".to_string()
}

fn default_output_max() -> usize {
    10 * 1024 * 1024
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> String {
    "text".to_string()
}
