use std::sync::{Arc, Mutex};

/// A thread-safe wrapper that manages access to the underlying `T` payload.
#[derive(Debug)]
pub struct ThreadSafeWrapper<T> {
    debug_label: &'static str,
    inner: Arc<Mutex<T>>,
}

impl<T> Clone for ThreadSafeWrapper<T> {
    /// Auto-derived Clone doesn't play well, so we're just being explicit.
    fn clone(&self) -> Self {
        Self {
            debug_label: self.debug_label,
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> ThreadSafeWrapper<T> {
    /// Creates and returns a blank wrapped `T`.
    pub fn new(debug_label: &'static str, inner: T) -> Self {
        Self {
            debug_label,
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Sets the underlying info payload.
    pub fn set(&self, inner: T) {
        match self.inner.lock() {
            Ok(mut lock) => {
                (*lock) = inner;
            },

            Err(error) => {
                tracing::error!(?error, ?self.debug_label, "Unable to lock for setting");
            },
        }
    }

    /// A helper method for setting an arbitrary field on the underlying user info
    /// payload. If the user info payload is `None`, this will lock and then exit.
    pub fn with_mut<F>(&self, handler: F)
    where
        F: FnOnce(&mut T),
    {
        match self.inner.lock() {
            Ok(mut lock) => {
                handler(&mut lock);
            },

            Err(error) => {
                tracing::error!(?error, ?self.debug_label, "Unable to lock for transformation");
            },
        }
    }

    /// A helper method that handles locking and returning.
    pub fn get<F, R>(&self, handler: F) -> Option<R>
    where
        F: FnOnce(&T) -> R,
    {
        match self.inner.lock() {
            Ok(lock) => Some(handler(&lock)),

            Err(error) => {
                tracing::error!(?error, ?self.debug_label, "Unable to lock for getting");
                None
            },
        }
    }
}
