use anyhow::Result;

use crate::account::Account;
use crate::core::Aparte;
use crate::message::Message;

pub type CryptoEngine = Box<dyn CryptoEngineTrait + Send>;

pub trait CryptoEngineTrait {
    fn encrypt(&mut self, aparte: &Aparte, account: &Account, message: &mut Message) -> Result<()>;
    fn decrypt(&mut self, aparte: &Aparte, account: &Account, message: &mut Message) -> Result<()>;
}
