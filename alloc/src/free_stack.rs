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
    top: CacheAlignedU16,
    stack: [u16; 0],
}

impl FreeStack {
    /// Sets up the free stack with all items free.
    /// # Safety
    /// Trailing array must be allocated with at least `capacity` elements.
    pub unsafe fn reset(&mut self, capacity: u16) {
        self.top.store(capacity, Ordering::Release);
        // Initialize the stack in reverse sequential order.
        let stack = self.stack.as_mut_ptr();
        for index in 0..capacity {
            stack.add(usize::from(index)).write(capacity - index - 1);
        }
    }

    /// Pops an item from the free stack.
    /// Returns `None` if the stack is empty.
    pub fn pop(&self) -> Option<u16> {
        let top = self.top.load(Ordering::Acquire);
        if top == 0 {
            return None;
        }

        // Only a single thread should be accessing the free-stack at a time.
        // This allows it to be extremely simple and lock free.
        let new_top = top - 1;

        // Read the value at the top of the stack.
        let popped_value = unsafe { self.stack.as_ptr().add(usize::from(new_top)).read() };

        // Update the top of the stack.
        self.top.store(new_top, Ordering::Release);

        Some(popped_value)
    }

    /// Pushes an item onto the free stack.
    /// # Safety
    /// The item must be a valid index into the stack.
    /// The stack must not be full.
    pub unsafe fn push(&mut self, item: u16) {
        let top = self.top.load(Ordering::Acquire);
        self.stack.as_mut_ptr().add(usize::from(top)).write(item);
        self.top.store(top + 1, Ordering::Release);
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
            assert_eq!(stack.pop(), Some(index));
        }
        assert_eq!(stack.pop(), None);

        // Push back all items.
        for index in 0..max_capacity {
            unsafe {
                stack.push(index);
            }
        }

        // Pop until empty again, this time items are in reverse order.
        for index in (0..max_capacity).rev() {
            assert_eq!(stack.pop(), Some(index));
        }
        assert_eq!(stack.pop(), None);
    }
}
