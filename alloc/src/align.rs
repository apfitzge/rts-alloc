pub fn round_to_next_alignment_of<const N: usize>(value: usize) -> usize {
    (value + N - 1) & !(N - 1)
}
