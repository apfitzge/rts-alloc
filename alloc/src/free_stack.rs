use crate::cache_aligned::CacheAlignedU16;
use std::sync::atomic::Ordering;

/// A lock-free stack that allows pushing and popping of indices.
///
/// The stack is a trailing array of `u16` indices.
///
/// The stack is NOT thread-safe and should only be accessed by a single
/// thread at a time.
#[repr(C)]
pub struct FreeStack {
    /// The current number of items in the stack - i.e. the top index.
    top: CacheAlignedU16,
    /// Trailing array of `u16` indices.
    stack: [u16; 0],
}

impl FreeStack {
    /// Returns the size in bytes of a `FreeStack` with the given `capacity`.
    pub const fn byte_size(capacity: u16) -> usize {
        core::mem::size_of::<FreeStack>() + (capacity as usize * core::mem::size_of::<u16>())
    }

    /// Sets up the free stack with all items free.
    ///
    /// # Safety
    /// - The trailing `stack` must be initialized correctly, with space for
    ///   at least `capacity` items.
    pub unsafe fn reset(&mut self, capacity: u16) {
        self.top.store(capacity, Ordering::Relaxed);
        // Initialize the stack in reverse sequential order.
        let stack = self.stack.as_mut_ptr();
        for index in 0..capacity {
            stack.add(usize::from(index)).write(capacity - index - 1);
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
        let popped_value = unsafe { self.stack.as_ptr().add(usize::from(new_top)).read() };

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
    pub unsafe fn push(&mut self, item: u16) {
        let top = self.top.load(Ordering::Relaxed);
        self.stack.as_mut_ptr().add(usize::from(top)).write(item);
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

    /// Return a value at a specific index.
    ///
    /// # Safety
    /// - The index must be less than the current length of the stack.
    /// - Trailing memory must have been initialized correctly.
    pub unsafe fn get(&self, index: u16) -> u16 {
        self.stack.as_ptr().add(usize::from(index)).read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_free_stack() {
        let mut buffer = [0u8; 1024];
        let max_capacity = ((buffer.len() - core::mem::size_of::<FreeStack>())
            / core::mem::size_of::<u16>()) as u16;

        let stack = unsafe { buffer.as_mut_ptr().cast::<FreeStack>().as_mut() }.unwrap();
        unsafe {
            stack.reset(max_capacity as u16);
        }

        // Pop until empty.
        for index in 0..max_capacity {
            // Safety: stack initialized with space for `max_capacity` items.
            assert_eq!(unsafe { stack.pop() }, Some(index));
        }
        // Safety: stack initialized with space for `max_capacity` items.
        assert_eq!(unsafe { stack.pop() }, None);

        // Push back all items.
        for index in 0..max_capacity {
            unsafe {
                stack.push(index);
            }
        }

        // Pop until empty again, this time items are in reverse order.
        for index in (0..max_capacity).rev() {
            // Safety: stack initialized with space for `max_capacity` items.
            assert_eq!(unsafe { stack.pop() }, Some(index));
        }

        // Safety: stack initialized with space for `max_capacity` items.
        assert_eq!(unsafe { stack.pop() }, None);
    }
}
