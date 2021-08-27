use std::future::Future;
use std::pin::Pin;
use std::task::{Poll, Context, Waker};
use std::str::FromStr;

use xmpp_parsers::iq::Iq;
use uuid::Uuid;

use crate::account::Account;
use crate::core::AparteAsync;

#[derive(Clone, Debug)]
pub enum PendingIqState {
    Waiting(Option<Waker>),
    Finished(Iq),
}

pub struct IqFuture {
    aparte: AparteAsync,
    uuid: Uuid,
}

impl IqFuture {
    pub fn new(mut aparte: AparteAsync, account: &Account, iq: Iq) -> Self {
        // TODO generate uuid in here
        let uuid = Uuid::from_str(&iq.id).unwrap();
        aparte.send(account, iq.into());
        aparte.pending_iq.lock().unwrap().insert(uuid, PendingIqState::Waiting(None));

        Self {
            aparte,
            uuid,
        }
    }
}

impl Future for IqFuture {
    type Output = Iq;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let mut pending_iq = self.aparte.pending_iq.lock().unwrap();
        match pending_iq.remove(&self.uuid) {
            None => panic!("Iq response has already been consumed"),
            Some(PendingIqState::Waiting(_)) => {
                pending_iq.insert(self.uuid.clone(), PendingIqState::Waiting(Some(cx.waker().clone())));
                Poll::Pending
            },
            Some(PendingIqState::Finished(iq)) => Poll::Ready(iq),
        }
    }
}
