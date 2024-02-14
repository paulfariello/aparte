use anyhow::Result;

use crate::account::Account;
use crate::core::Aparte;
use crate::message::Message;

pub type CryptoEngine = Box<dyn CryptoEngineTrait + Send>;

pub trait CryptoEngineTrait {
    fn ns(&self) -> &'static str;

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
        message: &xmpp_parsers::message::Message,
    ) -> Result<xmpp_parsers::message::Message>;
}
