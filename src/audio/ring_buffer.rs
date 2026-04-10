use std::collections::VecDeque;

#[derive(Debug)]
pub struct RingBuffer<T> {
    capacity: usize,
    queue: VecDeque<T>,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queue: VecDeque::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, value: T) -> Result<(), T> {
        if self.queue.len() >= self.capacity {
            return Err(value);
        }

        self.queue.push_back(value);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        self.queue.pop_front()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.queue.len() >= self.capacity
    }
}