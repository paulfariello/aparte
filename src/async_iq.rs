use std::future::Future;
use std::pin::Pin;
use std::task::{Poll, Context};

use xmpp_parsers::iq::Iq;

use crate::account::Account;

pub struct IqFuture {
    account: Account,
    id: String,
    aparte: Aparte,
}

impl IqFuture {
    pub fn new(account: Account, iq: Iq) -> Self {
        Self {
            account,
            iq
        }
    }
}

impl Future for IqFuture {
    type Output = Iq;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        match self.aparte.get_iq_response(&self.id) {
            None => {
                self.aparte.set_pending_iq_waker(self.request.uuid, cx.waker().clone());
                Poll::Pending
            },
            Some(iq) => Poll::Ready(iq),
        }
    }
}
