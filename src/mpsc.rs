use crate::{
    buffer::{OwnedBufferIter, VecBuffer},
    SynchronizedNotifier, SynchronizedQueue,
};

mod bounded;
pub mod error;
mod unbounded;

pub use bounded::{channel, Receiver, Sender, WeakSender};
pub use unbounded::{unbounded_channel, UnboundedReceiver, UnboundedSender, WeakUnboundedSender};

type ChannelIter<T> =
    OwnedBufferIter<SelfRef<SynchronizedQueue<VecBuffer<T>>>, VecBuffer<T>, SynchronizedNotifier>;

pub(crate) struct SelfRef<T>(*const T);

impl<T> SelfRef<T> {
    /// # Safety
    /// `self_ref` must outlive `SelfRef` instance
    pub(crate) unsafe fn new(self_ref: &T) -> Self {
        Self(self_ref)
    }
}

impl<T> AsRef<T> for SelfRef<T> {
    fn as_ref(&self) -> &T {
        // SAFETY: `SelfRef::new` contract ensure the dereference safety
        unsafe { &*self.0 }
    }
}
