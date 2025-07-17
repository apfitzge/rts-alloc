pub const fn round_to_next_alignment_of<const N: usize>(value: usize) -> usize {
    debug_assert!(N.is_power_of_two(), "N must be a power of two");
    (value + N - 1) & !(N - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_to_next_alignment_of() {
        assert_eq!(round_to_next_alignment_of::<8>(7), 8);
        assert_eq!(round_to_next_alignment_of::<8>(8), 8);
        assert_eq!(round_to_next_alignment_of::<8>(9), 16);
        assert_eq!(round_to_next_alignment_of::<64>(63), 64);
        assert_eq!(round_to_next_alignment_of::<64>(64), 64);
        assert_eq!(round_to_next_alignment_of::<64>(65), 128);
    }
}
