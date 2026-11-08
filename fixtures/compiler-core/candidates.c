/* Core-batch candidate batch: canonical bit-twiddling functions from the
 * superoptimization literature (Hacker's Delight via Gulwani 2011, the
 * 25-program benchmark STOKE used; STOKE's own reported wins: cycle-through-
 * 3-values, compute-parity, round-up-to-next-power-of-2). All uint64_t so
 * compiled forms stay in 64-bit GPRs. One function per line-comment.
 */
#include <stdint.h>

/* HD 2-1: turn off rightmost 1-bit (STOKE p01 family) */
uint64_t turn_off_rightmost(uint64_t x) { return x & (x - 1); }

/* HD 2-2: isolate lowest set bit */
uint64_t lowest_set_bit(uint64_t x) { return x & (-x); }

/* HD 2-13 / STOKE-reported: round up to next power of 2 */
uint64_t next_pow2(uint64_t x) {
    x = x - 1;
    x |= x >> 1; x |= x >> 2; x |= x >> 4;
    x |= x >> 8; x |= x >> 16; x |= x >> 32;
    return x + 1;
}

/* HD: overflow-safe average (unsigned) */
uint64_t avg(uint64_t a, uint64_t b) { return (a & b) + ((a ^ b) >> 1); }

/* abs for signed 64-bit */
int64_t iabs(int64_t x) { return (x ^ (x >> 63)) - (x >> 63); }

/* signed min */
int64_t smin(int64_t a, int64_t b) { return a < b ? a : b; }

/* unsigned min */
uint64_t umin(uint64_t a, uint64_t b) { return a < b ? a : b; }

/* unsigned max */
uint64_t umax(uint64_t a, uint64_t b) { return a > b ? a : b; }

/* HD: cycle through 3 values (STOKE p21 reported win) */
uint64_t cycle3(uint64_t x, uint64_t a, uint64_t b, uint64_t c) {
    return x == c ? a : (x == a ? b : c);
}

/* HD: compute parity of a byte, extended (STOKE-reported) */
uint64_t parity8(uint64_t x) {
    x = x & 0xff;
    x ^= x >> 4; x ^= x >> 2; x ^= x >> 1;
    return x & 1;
}

/* popcount of low byte, SWAR (what gcc emits without -mpopcnt) */
uint64_t popcount8(uint64_t x) {
    x = x & 0xff;
    x = x - ((x >> 1) & 0x55);
    x = (x & 0x33) + ((x >> 2) & 0x33);
    x = (x + (x >> 4)) & 0x0f;
    return x;
}

/* sign mask: -1 if negative else 0 */
uint64_t sign_mask(int64_t x) { return (uint64_t)(x >> 63); }

/* is x < y unsigned ? 1 : 0 -- exercises cmp/setcc shapes */
uint64_t ult_flag(uint64_t a, uint64_t b) { return a < b ? 1u : 0u; }

/* select: HD cmov idiom */
uint64_t select_bits(uint64_t x, uint64_t m, uint64_t y) { return (x & m) | (y & ~m); }

/* saturating add (unsigned) */
uint64_t sat_add(uint64_t a, uint64_t b) {
    uint64_t s = a + b;
    return s < a ? UINT64_MAX : s;
}

/* byte swap (likely needs bswap opcode — expected exclusion) */
uint64_t bswap64(uint64_t x) { return __builtin_bswap64(x); }

/* count trailing zeros (likely needs tzcnt/bsf — expected exclusion) */
uint64_t ctz64(uint64_t x) { return (uint64_t)__builtin_ctzll(x); }

/* unsigned divide by 7 (likely needs one-operand mul — magic number) */
uint64_t div7(uint64_t x) { return x / 7; }

/* HD 10-1: average floored differently, avoid overflow — same as avg but via cmp */
uint64_t avg2(uint64_t a, uint64_t b) { return (a >> 1) + (b >> 1) + (a & b & 1); }
