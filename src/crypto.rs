use anyhow::Result;

use crate::message::Message;

pub type CryptoEngine = Box<dyn CryptoEngineTrait + Send>;

pub trait CryptoEngineTrait {
    fn encrypt(&mut self, message: &mut Message) -> Result<()>;
    fn decrypt(&mut self, message: &mut Message) -> Result<()>;
}
