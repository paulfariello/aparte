use std::future::Future;
use std::pin::Pin;
use std::task::{Poll, Context};

use xmpp_parsers::iq::Iq;

use crate::account::Account;
use crate::core::AparteAsync;

pub struct IqFuture {
    #[allow(dead_code)]
    aparte: AparteAsync,
    #[allow(dead_code)]
    account: Account,
    #[allow(dead_code)]
    id: String,
}

impl IqFuture {
    pub fn new(aparte: AparteAsync, account: Account, iq: Iq) -> Self {
        Self {
            aparte,
            account,
            id: iq.id,
        }
    }
}

impl Future for IqFuture {
    type Output = Iq;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Self::Output> {
        todo!()
        //match self.aparte.get_iq_response(&self.id) {
        //    None => {
        //        self.aparte.set_pending_iq_waker(self.request.uuid, cx.waker().clone());
        //        Poll::Pending
        //    },
        //    Some(iq) => Poll::Ready(iq),
        //}
    }
}
