pub const NUM_SIZE_CLASSES: usize = 5;
/// Size classes must be sub powers of two
pub const SIZE_CLASSES: [u32; NUM_SIZE_CLASSES] = [256, 512, 1024, 2048, 4096];
pub const MIN_SIZE: u32 = SIZE_CLASSES[0];
pub const MAX_SIZE: u32 = SIZE_CLASSES[NUM_SIZE_CLASSES - 1];
const BASE_SHIFT: u32 = SIZE_CLASSES[0].trailing_zeros() as u32;

const _: () = assert!(
    NUM_SIZE_CLASSES < 256,
    "NUM_SIZE_CLASSES must be less than 256"
);

pub fn size_class_index(size: u32) -> u8 {
    debug_assert!(size <= MAX_SIZE);
    size.next_power_of_two()
        .trailing_zeros()
        .saturating_sub(BASE_SHIFT) as u8
}
