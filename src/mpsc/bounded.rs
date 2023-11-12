use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use crate::{
    buffer::{BufferSlice, IntoValueIter, VecBuffer},
    mpsc::{
        error,
        error::{RecvError, SendError, TryRecvError, TrySendError},
        ChannelIter, SelfRef,
    },
    SynchronizedNotifier, SynchronizedQueue,
};

pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    assert!(capacity > 0, "capacity must be strictly positive");
    let channel = Arc::new(SynchronizedQueue::with_capacity(capacity));
    let iterator = None;
    (Sender(channel.clone()), Receiver { channel, iterator })
}

type Channel<T> = Arc<SynchronizedQueue<VecBuffer<T>>>;

#[derive(Debug, Clone)]
pub struct Sender<T>(Channel<T>);

impl<T> Sender<T> {
    pub fn same_channel(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        self.0.try_enqueue([value]).map_err(error::try_send_error)
    }

    pub fn try_send_iter<I>(&self, iter: I) -> Result<(), TrySendError<I::IntoIter>>
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        self.0
            .try_enqueue(iter.into_value_iter())
            .map_err(error::try_send_error)
    }

    pub fn send_timeout(&self, value: T, timeout: Duration) -> Result<(), TrySendError<T>> {
        self.0
            .enqueue_timeout([value], timeout)
            .map_err(error::try_send_error)
    }

    pub fn send_iter_timeout<I>(
        &self,
        iter: I,
        timeout: Duration,
    ) -> Result<(), TrySendError<I::IntoIter>>
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        self.0
            .enqueue_timeout(iter.into_value_iter(), timeout)
            .map_err(error::try_send_error)
    }

    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        self.0.enqueue([value]).map_err(error::send_error)
    }

    pub fn send_iter<I>(&self, iter: I) -> Result<(), SendError<I::IntoIter>>
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        self.0
            .enqueue(iter.into_value_iter())
            .map_err(error::send_error)
    }

    pub async fn send_async(&self, value: T) -> Result<(), SendError<T>> {
        self.0
            .enqueue_async([value])
            .await
            .map_err(error::send_error)
    }

    pub async fn send_iter_async<I>(&self, iter: I) -> Result<(), SendError<I::IntoIter>>
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        self.0
            .enqueue_async(iter.into_value_iter())
            .await
            .map_err(error::send_error)
    }

    pub fn downgrade(&self) -> WeakSender<T> {
        WeakSender(Arc::downgrade(&self.0))
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 1 {
            self.0.close();
        }
    }
}

#[derive(Debug)]
pub struct WeakSender<T>(Weak<SynchronizedQueue<VecBuffer<T>>>);

impl<T> WeakSender<T> {
    pub fn upgrade(&self) -> Option<Sender<T>> {
        self.0.upgrade().map(Sender)
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    channel: Channel<T>,
    iterator: Option<ChannelIter<T>>,
}

impl<T> Receiver<T> {
    pub fn capacity(&self) -> usize {
        self.channel.capacity()
    }

    pub fn len(&self) -> usize {
        self.channel.len()
    }

    pub fn is_empty(&self) -> bool {
        self.channel.is_empty()
    }

    pub fn close(&self) {
        self.channel.close();
    }

    fn next(&mut self) -> Option<T> {
        let value = self.iterator.as_mut().and_then(Iterator::next);
        if value.is_none() {
            self.iterator = None;
        }
        value
    }

    fn slice(&mut self) -> Option<BufferSlice<VecBuffer<T>, SynchronizedNotifier>> {
        let iter = self.iterator.take()?;
        Some(iter.with_owned(self.channel.as_ref()).into_slice())
    }

    fn set_iter_and_next(&mut self, iter: ChannelIter<T>) -> T {
        self.iterator.replace(iter);
        self.iterator
            .as_mut()
            .unwrap()
            .next()
            .expect("dequeued iterator cannot be empty")
    }

    fn recv_sync<E>(
        &mut self,
        get_slice: impl FnOnce(&Self) -> Result<BufferSlice<VecBuffer<T>, SynchronizedNotifier>, E>,
    ) -> Result<T, E> {
        if let Some(value) = self.next() {
            return Ok(value);
        }
        let iter = get_slice(self)?
            .into_iter()
            // SAFETY: `self.channel` outlive the iterator
            .with_owned(unsafe { SelfRef::new(self.channel.as_ref()) });
        Ok(self.set_iter_and_next(iter))
    }

    fn recv_slice_sync<E>(
        &mut self,
        get_slice: impl FnOnce(&Self) -> Result<BufferSlice<VecBuffer<T>, SynchronizedNotifier>, E>,
    ) -> Result<BufferSlice<VecBuffer<T>, SynchronizedNotifier>, E> {
        // SAFETY: trick to solve false-positive
        if let Some(slice) = unsafe { &mut *(self as *mut Self) }.slice() {
            return Ok(slice);
        }
        get_slice(self)
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.recv_sync(|this| this.channel.try_dequeue().map_err(error::try_recv_err))
    }

    pub fn try_recv_slice(
        &mut self,
    ) -> Result<BufferSlice<VecBuffer<T>, SynchronizedNotifier>, TryRecvError> {
        self.recv_slice_sync(|this| this.channel.try_dequeue().map_err(error::try_recv_err))
    }

    pub fn recv(&mut self) -> Result<T, RecvError> {
        self.recv_sync(|this| this.channel.dequeue().map_err(error::recv_err))
    }

    pub fn recv_slice(
        &mut self,
    ) -> Result<BufferSlice<VecBuffer<T>, SynchronizedNotifier>, RecvError> {
        self.recv_slice_sync(|this| this.channel.dequeue().map_err(error::recv_err))
    }

    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<T, TryRecvError> {
        self.recv_sync(|this| {
            this.channel
                .dequeue_timeout(timeout)
                .map_err(error::try_recv_err)
        })
    }

    pub fn recv_slice_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<BufferSlice<VecBuffer<T>, SynchronizedNotifier>, TryRecvError> {
        self.recv_slice_sync(|this| {
            this.channel
                .dequeue_timeout(timeout)
                .map_err(error::try_recv_err)
        })
    }

    pub async fn recv_async(&mut self) -> Result<T, RecvError> {
        if let Some(value) = self.next() {
            return Ok(value);
        }
        let slice = self
            .channel
            .dequeue_async()
            .await
            .map_err(error::recv_err)?;
        let iter = slice
            .into_iter()
            // SAFETY: `self.channel` outlive the iterator
            .with_owned(unsafe { SelfRef::new(self.channel.as_ref()) });
        Ok(self.set_iter_and_next(iter))
    }

    pub async fn recv_slice_async(
        &mut self,
    ) -> Result<BufferSlice<VecBuffer<T>, SynchronizedNotifier>, RecvError> {
        // SAFETY: trick to solve false-positive
        if let Some(slice) = unsafe { &mut *(self as *mut Self) }.slice() {
            return Ok(slice);
        }
        self.channel.dequeue_async().await.map_err(error::recv_err)
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.iterator = None;
        self.close();
    }
}
