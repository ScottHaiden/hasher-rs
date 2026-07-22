use std::collections::VecDeque;
use std::sync::{Mutex, Condvar};

struct ScopeNotifier<'a> {
    cv: &'a Condvar,
}

impl<'a> ScopeNotifier<'a> {
    fn new(cv: &'a Condvar) -> Self {
        Self { cv: cv }
    }
}

impl<'a> Drop for ScopeNotifier<'a> {
    fn drop(&mut self) {
        self.cv.notify_one();
    }
}

struct TransmitterState<T> {
    buffer: VecDeque<T>,
    closed: bool,
    dead: bool,
}

pub struct Transmitter<T> {
    state: Mutex<TransmitterState<T>>,
    cv: Condvar,
    capacity: usize,
}

pub struct TransmitterCloser<'a, T> {
    transmitter: &'a Transmitter<T>,
}

impl<'a, T> TransmitterCloser<'a, T> {
    fn new(transmitter: &'a Transmitter<T>) -> Self {
        Self { transmitter: transmitter }
    }
}

impl<'a, T> Drop for TransmitterCloser<'a, T> {
    fn drop(&mut self) {
        self.transmitter.close();
    }
}


impl<T> Transmitter<T> {
    pub fn new(size: usize) -> Self {
        let state = TransmitterState {
            buffer: VecDeque::with_capacity(size),
            closed: false,
            dead: false,
        };
        Self {
            state: Mutex::new(state),
            cv: Condvar::new(),
            capacity: size,
        }
    }

    pub fn closer(&self) -> TransmitterCloser<'_, T> {
        TransmitterCloser::new(self)
    }

    fn notifier(&self) -> ScopeNotifier<'_> {
        ScopeNotifier::new(&self.cv)
    }

    pub fn put(&self, item: T) -> bool {
        let _notifier = self.notifier();
        let mut lock = self.state.lock().unwrap();
        loop {
            let state = &*lock;
            if state.closed || state.dead { return false; }
            if state.buffer.len() < self.capacity { break; }
            lock = self.cv.wait(lock).unwrap();
        }

        let state = &mut *lock;
        assert!(!state.closed, "closed at time of put()!");
        state.buffer.push_back(item);
        true
    }

    pub fn close(&self) {
        let _notifier = self.notifier();
        let mut lock = self.state.lock().unwrap();
        let state = &mut *lock;
        state.closed = true;
    }

    pub fn get(&self) -> Option<T> {
        let _notifier = self.notifier();
        let mut lock = self.state.lock().unwrap();
        loop {
            let state = &*lock;
            if state.dead { return None; }
            if !state.buffer.is_empty() || state.closed { break; }
            lock = self.cv.wait(lock).unwrap();
        }
        let state = &mut *lock;
        if state.buffer.is_empty() {
            assert!(state.closed, "empty and not closed after wait!");
            return None;
        }
        Some(state.buffer.pop_front().unwrap())
    }

    pub fn kill(&self) {
        let _notifier = self.notifier();
        let mut lock = self.state.lock().unwrap();
        let state = &mut *lock;
        state.dead = true;
    }
}
