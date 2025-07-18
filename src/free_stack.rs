use core::ptr::NonNull;
use core::sync::atomic::{AtomicU16, Ordering};

/// A lock-free stack that allows pushing and popping of indices.
///
/// The stack is a trailing array of `u16` indices.
///
/// The stack is NOT thread-safe and should only be accessed by a single
/// thread at a time.
pub struct FreeStack<'a> {
    /// The current number of items in the stack - i.e. the top index.
    top: &'a AtomicU16,
    /// The current capacity of the stack.
    capacity: &'a AtomicU16,
    /// Trailing array of `u16` indices.
    stack: NonNull<AtomicU16>,
}

impl<'a> FreeStack<'a> {
    /// Creates a new free stack.
    ///
    /// # Safety
    /// - `top` must be a valid reference to an `AtomicU16` that
    ///   represents the current top of the stack.
    /// - `capacity` must be a valid reference to an `AtomicU16` that
    ///   represents the current capacity of the stack.
    /// - `stack` must be a valid pointer to the trailing array of `AtomicU16`.
    ///   The stack must have enough space for at least `capacity` items.
    pub unsafe fn new(
        top: &'a AtomicU16,
        capacity: &'a AtomicU16,
        stack: NonNull<AtomicU16>,
    ) -> Self {
        Self {
            top,
            capacity,
            stack,
        }
    }

    /// Returns the size in bytes of a `FreeStack` with the given `capacity`.
    pub const fn byte_size(capacity: u16) -> usize {
        core::mem::size_of::<AtomicU16>() * 2
            + (capacity as usize * core::mem::size_of::<AtomicU16>())
    }

    /// Sets up the free stack with all items free.
    ///
    /// # Safety
    /// - The trailing `stack` must be initialized correctly, with space for
    ///   at least `capacity` items.
    pub unsafe fn reset(&self, capacity: u16) {
        self.capacity.store(capacity, Ordering::Relaxed);
        self.top.store(capacity, Ordering::Relaxed);
        // Initialize the stack in reverse sequential order.
        for index in 0..capacity {
            self.stack
                .add(usize::from(index))
                .as_ref()
                .store(capacity - index - 1, Ordering::Relaxed);
        }
    }

    /// Pops an item from the free stack.
    /// Returns `None` if the stack is empty.
    ///
    /// # Safety
    /// - The trailing `stack` must be initialized correctly.
    pub unsafe fn pop(&self) -> Option<u16> {
        let top = self.top.load(Ordering::Relaxed);
        if top == 0 {
            return None;
        }

        // Only a single thread should be accessing the free-stack at a time.
        // This allows it to be extremely simple and lock free.
        let new_top = top - 1;

        // Read the value at the top of the stack.
        // Safety: The trailing stack is initialized correctly.
        let popped_value = unsafe {
            self.stack
                .add(usize::from(new_top))
                .as_ref()
                .load(Ordering::Relaxed)
        };

        // Update the top of the stack.
        self.top.store(new_top, Ordering::Relaxed);

        Some(popped_value)
    }

    /// Pushes an item onto the free stack.
    ///
    /// # Safety
    /// - The trailing `stack` must be initialized correctly.
    /// - The item must be a valid index into the stack.
    /// - The stack must not be full.
    pub unsafe fn push(&self, item: u16) {
        let top = self.top.load(Ordering::Relaxed);
        self.stack
            .add(usize::from(top))
            .as_ref()
            .store(item, Ordering::Relaxed);
        self.top.store(top + 1, Ordering::Relaxed);
    }

    /// Returns the current top value - i.e. the size.
    pub fn len(&self) -> u16 {
        self.top.load(Ordering::Relaxed)
    }

    /// Returns true if the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns true if the stack if full.
    pub fn is_full(&self) -> bool {
        self.len() == self.capacity.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_free_stack() {
        const MAX_CAPACITY: u16 = 1024;
        let top = AtomicU16::new(0);
        let capacity = AtomicU16::new(MAX_CAPACITY);
        let mut buffer = (0..MAX_CAPACITY)
            .map(|_| AtomicU16::new(0))
            .collect::<Vec<_>>();
        let stack =
            unsafe { FreeStack::new(&top, &capacity, NonNull::new(buffer.as_mut_ptr()).unwrap()) };
        unsafe {
            stack.reset(MAX_CAPACITY);
        }
        assert!(stack.is_full());

        // Pop until empty.
        for index in 0..MAX_CAPACITY {
            // Safety: stack initialized with space for `MAX_CAPACITY` items.
            assert_eq!(unsafe { stack.pop() }, Some(index));
            assert!(!stack.is_full());
        }
        // Safety: stack initialized with space for `MAX_CAPACITY` items.
        assert_eq!(unsafe { stack.pop() }, None);
        assert!(!stack.is_full());

        // Push back all items.
        for index in 0..MAX_CAPACITY {
            unsafe {
                stack.push(index);
            }
        }
        assert!(stack.is_full());

        // Pop until empty again, this time items are in reverse order.
        for index in (0..MAX_CAPACITY).rev() {
            // Safety: stack initialized with space for `MAX_CAPACITY` items.
            assert_eq!(unsafe { stack.pop() }, Some(index));
        }

        // Safety: stack initialized with space for `MAX_CAPACITY` items.
        assert_eq!(unsafe { stack.pop() }, None);
    }
}
