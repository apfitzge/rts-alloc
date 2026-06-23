pub type CacheAlignedU64 = CacheAligned<crate::sync::AtomicU64>;

#[repr(C, align(64))]
pub struct CacheAligned<T>(pub T);

impl<T> core::ops::Deref for CacheAligned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
