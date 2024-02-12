use anyhow::Result;
use xmpp_parsers::legacy_omemo;

use crate::account::Account;
use crate::core::Aparte;
use crate::message::Message;

pub type CryptoEngine = Box<dyn CryptoEngineTrait + Send>;

pub trait CryptoEngineTrait {
    fn encrypt(
        &mut self,
        aparte: &Aparte,
        account: &Account,
        message: &Message,
    ) -> Result<xmpp_parsers::Element>;
    fn decrypt(
        &mut self,
        aparte: &Aparte,
        account: &Account,
        message: &mut Message,
    ) -> Result<Message>;
}
