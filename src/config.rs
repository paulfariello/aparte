use serde::Deserialize;
use std::collections::HashMap;

use crate::account::Account;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub accounts: HashMap<String, Account>,
}
