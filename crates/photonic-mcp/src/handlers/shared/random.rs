/// Simple xorshift64 PRNG returning values in [-1.0, 1.0].
pub(crate) fn xorshift64(state: &mut u64) -> f64 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    *state = s;
    // Map to [-1, 1]
    (s as f64 / u64::MAX as f64) * 2.0 - 1.0
}
