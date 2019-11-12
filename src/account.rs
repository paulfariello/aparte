use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub login: String,
    pub server: Option<String>,
    pub port: Option<u16>,
    pub autoconnect: bool,
}
