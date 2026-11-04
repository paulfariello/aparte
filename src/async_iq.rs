use tokio::sync::oneshot;
use tokio_xmpp::{IqRequest, IqResponseToken};
use xmpp_parsers::jid::Jid;

pub struct IqEnvelope {
    pub to: Option<Jid>,
    pub request: IqRequest,
    pub token_tx: oneshot::Sender<IqResponseToken>,
}
