use std::{
    ops::Deref,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use crate::{
    buffer::{BufferSlice, IntoValueIter, ValueIter, VecBuffer},
    error::TryEnqueueError,
    mpsc::{
        error,
        error::{send_error, RecvError, SendError, TryRecvError},
        ChannelIter, SelfRef,
    },
    notify::Notify,
    SynchronizedNotifier, SynchronizedQueue,
};

pub fn unbounded_channel<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let channel = Arc::new(Inner {
        queue: SynchronizedQueue::new(),
        overflow: Mutex::new(Overflow {
            items: Vec::new(),
            len: 0,
        }),
    });
    let iterator = None;
    (
        UnboundedSender(channel.clone()),
        UnboundedReceiver { channel, iterator },
    )
}

type Channel<T> = Arc<Inner<T>>;

#[derive(Debug)]
struct Inner<T> {
    queue: SynchronizedQueue<VecBuffer<T>>,
    overflow: Mutex<Overflow<T>>,
}

impl<T> Deref for Inner<T> {
    type Target = SynchronizedQueue<VecBuffer<T>>;

    fn deref(&self) -> &Self::Target {
        &self.queue
    }
}

#[derive(Debug)]
struct Overflow<T> {
    items: Vec<OverflowItem<T>>,
    len: usize,
}

#[derive(Debug)]
enum OverflowItem<T> {
    Single(<Option<T> as IntoIterator>::IntoIter),
    Vec(<Vec<T> as IntoIterator>::IntoIter),
}

impl<T> Iterator for OverflowItem<T> {
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Single(iter) => iter.next(),
            Self::Vec(iter) => iter.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Single(iter) => iter.size_hint(),
            Self::Vec(iter) => iter.size_hint(),
        }
    }
}

impl<T> ExactSizeIterator for OverflowItem<T> {}

#[derive(Debug, Clone)]
pub struct UnboundedSender<T>(Channel<T>);

impl<T> UnboundedSender<T> {
    pub fn same_channel(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
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

    pub fn send(&self, mut value: T) -> Result<(), SendError<T>> {
        match self.0.try_enqueue([value]) {
            Err(TryEnqueueError::InsufficientCapacity([v])) => value = v,
            res => return res.map_err(send_error),
        };
        let mut overflow = self.0.overflow.lock().unwrap();
        match self.0.try_enqueue([value]) {
            Err(TryEnqueueError::InsufficientCapacity([v])) => value = v,
            res => return res.map_err(send_error),
        };
        overflow.len += 1;
        overflow
            .items
            .push(OverflowItem::Single(Some(value).into_iter()));
        self.0.notify().notify_dequeue();
        Ok(())
    }

    pub fn send_iter<I>(&self, iter: I) -> Result<(), SendError<I::IntoIter>>
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        let mut iter = iter.into_value_iter();
        match self.0.try_enqueue(iter) {
            Err(TryEnqueueError::InsufficientCapacity(i)) => iter = i,
            res => return res.map_err(send_error),
        };
        let mut overflow = self.0.overflow.lock().unwrap();
        match self.0.try_enqueue(iter) {
            Err(TryEnqueueError::InsufficientCapacity(i)) => iter = i,
            res => return res.map_err(send_error),
        };
        overflow.len += iter.0.len();
        overflow
            .items
            .push(OverflowItem::Vec(iter.0.collect::<Vec<_>>().into_iter()));
        self.0.notify().notify_dequeue();
        Ok(())
    }

    pub fn downgrade(&self) -> WeakUnboundedSender<T> {
        WeakUnboundedSender(Arc::downgrade(&self.0))
    }
}

impl<T> Drop for UnboundedSender<T> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 1 {
            self.0.close();
        }
    }
}

#[derive(Debug)]
pub struct WeakUnboundedSender<T>(Weak<Inner<T>>);

impl<T> WeakUnboundedSender<T> {
    pub fn upgrade(&self) -> Option<UnboundedSender<T>> {
        self.0.upgrade().map(UnboundedSender)
    }
}

#[derive(Debug)]
pub struct UnboundedReceiver<T> {
    channel: Channel<T>,
    iterator: Option<ChannelIter<T>>,
}

impl<T> UnboundedReceiver<T> {
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
        Some(iter.with_owned(&self.channel.queue).into_slice())
    }

    fn set_iter_and_next(&mut self, iter: ChannelIter<T>) -> T {
        self.iterator.replace(iter);
        self.iterator
            .as_mut()
            .unwrap()
            .next()
            .expect("dequeued iterator cannot be empty")
    }

    fn try_dequeue_and_resize(&self) -> Option<BufferSlice<VecBuffer<T>, SynchronizedNotifier>> {
        let mut overflow = self.channel.overflow.lock().unwrap();
        let capacity = self.channel.capacity() + overflow.len;
        let items = &mut overflow.items;
        // `{ items }` is a trick to get the correct FnOnce inference
        // https://stackoverflow.com/questions/74814588/why-does-rust-infer-fnmut-instead-of-fnonce-for-this-closure-even-though-inferr
        // https://github.com/rust-lang/rust-clippy/issues/11562#issuecomment-1750237884
        let insert = Some(move || { items }.drain(..).map(ValueIter));
        let slice = self.channel.try_dequeue_and_resize(capacity, insert).ok()?;
        if overflow.items.is_empty() {
            overflow.len = 0;
        }
        drop(overflow);
        Some(slice)
    }

    fn recv_sync<E>(
        &mut self,
        get_slice: impl FnOnce(&Self) -> Result<BufferSlice<VecBuffer<T>, SynchronizedNotifier>, E>,
    ) -> Result<T, E> {
        if let Some(value) = self.next() {
            return Ok(value);
        }
        let slice = self
            .try_dequeue_and_resize()
            .map_or_else(|| get_slice(self), Ok)?;
        let iter = slice
            .into_iter()
            // SAFETY: `self.channel` outlive the iterator
            .with_owned(unsafe { SelfRef::new(&self.channel.queue) });
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
        self.try_dequeue_and_resize()
            .map_or_else(|| get_slice(self), Ok)
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
        let slice = match self.try_dequeue_and_resize() {
            Some(s) => Ok(s),
            None => self.channel.dequeue_async().await.map_err(error::recv_err),
        }?;
        let iter = slice
            .into_iter()
            // SAFETY: `self.channel` outlive the iterator
            .with_owned(unsafe { SelfRef::new(&self.channel.queue) });
        Ok(self.set_iter_and_next(iter))
    }

    pub async fn recv_slice_async(
        &mut self,
    ) -> Result<BufferSlice<VecBuffer<T>, SynchronizedNotifier>, RecvError> {
        // SAFETY: trick to solve false-positive
        if let Some(slice) = unsafe { &mut *(self as *mut Self) }.slice() {
            return Ok(slice);
        }
        match self.try_dequeue_and_resize() {
            Some(s) => Ok(s),
            None => self.channel.dequeue_async().await.map_err(error::recv_err),
        }
    }
}

impl<T> Drop for UnboundedReceiver<T> {
    fn drop(&mut self) {
        self.close();
    }
}
