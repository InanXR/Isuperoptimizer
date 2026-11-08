/* Full-provenance candidate batch: the Gulwani/STOKE benchmark set.
 *
 * Primary source: Gulwani, Jha, Tiwari, Venkatesan, "Synthesis of Loop-Free
 * Programs", PLDI 2011, Figure 4 (programs P1-P25, drawn from Hacker's
 * Delight — the same 25-program set STOKE ran; ASPLOS'13 §6.1 and Fig. 11).
 * Every P-program below is a direct port of the Figure-4 bitvector program
 * to uint64_t/int64_t C, with the porting decisions written at each function:
 * the four 32-bit-constant programs are scaled by doubling the word size
 * (0x11111111 -> 0x1111111111111111, shifts 31->63, 16-bit parts -> 32-bit
 * parts), exactly as the core batch's next_pow2 already scaled HD's 32-bit smear.
 *
 * The core-batch survivors that are themselves P-programs (P1 turn_off_rightmost,
 * P3 lowest_set_bit, P9 iabs, P14 avg, P24 next_pow2) and the core-batch extras
 * (smin, umin, umax, sat_add, select_bits, sign_mask) keep their committed
 * fixtures/compiler-core/candidates.c sources byte-identical so both batches'
 * artifacts rest on
 * the same audited C.
 */
#include <stdint.h>

/* ------------------------------------------------------------------ P-programs */

/* P1: turn off rightmost 1-bit (HD 2-1; core-batch source unchanged) */
uint64_t turn_off_rightmost(uint64_t x) { return x & (x - 1); }

/* P2: test whether an unsigned integer is of the form 2^n - 1. Figure 4
 * returns the bitvector x & (x+1) — zero iff x = 2^n - 1. */
uint64_t p02_test_2n_minus_1(uint64_t x) { return x & (x + 1); }

/* P3: isolate the rightmost 1-bit (core-batch source unchanged) */
uint64_t lowest_set_bit(uint64_t x) { return x & (-x); }

/* P4: form a mask that identifies the rightmost 1 bit and trailing 0s */
uint64_t p04_mask_lowest_ones(uint64_t x) { return x ^ (x - 1); }

/* P5: right-propagate the rightmost 1-bit */
uint64_t p05_smear_rightmost(uint64_t x) { return x | (x - 1); }

/* P6: turn on the rightmost 0-bit in a word */
uint64_t p06_turn_on_rightmost0(uint64_t x) { return x | (x + 1); }

/* P7: isolate the rightmost 0-bit — Figure 4 uses bvnot (~x) */
uint64_t p07_isolate_rightmost0(uint64_t x) { return ~x & (x + 1); }

/* P8: form a mask that identifies the trailing 0s — bvnot again */
uint64_t p08_mask_trailing_zeros(uint64_t x) { return (x - 1) & ~x; }

/* P9: absolute value (core-batch source unchanged; Figure 4 shifts by 31 for
 * 32-bit, we shift by 63 for 64-bit) */
int64_t iabs(int64_t x) { return (x ^ (x >> 63)) - (x >> 63); }

/* P10/P11/P12: nlz-comparison tests. Figure 4 is explicit that these are
 * boolean-valued (bvule/bvugt 1-bit results), so the C returns 0/1. */
uint64_t p10_nlz_eq(uint64_t x, uint64_t y) { return (x ^ y) <= (x & y); }
uint64_t p11_nlz_lt(uint64_t x, uint64_t y) { return (x & ~y) > y; }
uint64_t p12_nlz_le(uint64_t x, uint64_t y) { return (x & ~y) <= y; }

/* P13: sign function. HD's original ((x >> 31) | ((unsigned)-x >> 31),
 * arithmetic first shift, logical second) scaled to 63; written without
 * signed-overflow UB (0 - (uint64_t)x wraps definedly). sign(INT64_MIN)
 * = -1, matching HD. */
uint64_t p13_sign(int64_t x) {
    uint64_t s = (uint64_t)x;
    uint64_t n = 0 - s;
    return ((uint64_t)(x >> 63)) | (n >> 63);
}

/* P14: floor of average of two integers without overflowing (core-batch avg) */
uint64_t avg(uint64_t a, uint64_t b) { return (a & b) + ((a ^ b) >> 1); }

/* P15: ceil of average of two integers without overflowing */
uint64_t p15_avg_ceil(uint64_t a, uint64_t b) { return (a | b) - ((a ^ b) >> 1); }

/* P16: max of two integers, Figure 4's branchless form — o2 = bvneg(bvuge(x,y))
 * is an unsigned-compare mask in the bitvector semantics. */
uint64_t p16_umax_bitwise(uint64_t x, uint64_t y) {
    uint64_t m = 0 - (uint64_t)(x >= y);
    return ((x ^ y) & m) ^ y;
}

/* P17: turn off the rightmost contiguous string of 1 bits — Figure 4 verbatim:
 * o1=x-1; o2=x|o1; o3=o2+1; res=o3&x */
uint64_t p17_turn_off_run(uint64_t x) { return (((x | (x - 1)) + 1) & x); }

/* P18: determine if an integer is a power of 2 (boolean 0/1). Figure 4:
 * bvredor(x) && !bvredor(x & (x-1)), written branchless in the SSA order. */
uint64_t p18_is_pow2(uint64_t x) {
    uint64_t o1 = x - 1;
    uint64_t o2 = o1 & x;
    uint64_t o3 = (x != 0);
    uint64_t o4 = (o2 != 0);
    uint64_t o5 = (o4 == 0);
    return o5 & o3;
}

/* P19: exchange fields A and B of register x; m masks field B, k is the
 * distance from the end of A to the start of B (Figure 4 verbatim, runtime
 * k — variable shifts). NB: C leaves x>>k undefined for k >= 64 but gcc
 * emits shrq %cl, which masks to 6 bits — matching both SMTLIB bvshr and
 * our model; the compiled code is total and is the spec of record. */
uint64_t p19_exchange_fields(uint64_t x, uint64_t m, uint64_t k) {
    uint64_t o1 = x >> k;
    uint64_t o2 = x ^ o1;
    uint64_t o3 = o2 & m;
    uint64_t o4 = o3 << k;
    uint64_t o5 = o4 ^ o3;
    return o5 ^ x;
}

/* P20: next higher unsigned number with same number of 1 bits (snoob).
 * At x = 0: o2 = 0 — division by zero, #DE in hardware and a definedness
 * violation in our model (ADR-008); the function is undefined there.
 *
 * Figure 4's line 4 prints bvdiv(bvshr(bvxor(x, o2), 2), o2) — x XORed
 * with the lowest set bit. HD's snoob (the stated source, verbatim) uses
 * ones = x ^ ripple there (ripple = x + lowest); the paper's variant
 * returns a same-popcount higher value that is not the NEXT one (x=22:
 * paper form gives 26, HD gives 25). Implemented per HD; the deviation is
 * documented here and in src/benchmarks/compiler_full.rs (same class as
 * P23's line-6 typo). */
uint64_t p20_snoob(uint64_t x) {
    uint64_t o1 = 0 - x;
    uint64_t o2 = x & o1;
    uint64_t o3 = x + o2;
    uint64_t o4 = x ^ o3;
    uint64_t o5 = o4 >> 2;
    return (o5 / o2) | o3;
}

/* P21: cycle through 3 values a,b,c — the branchless HD form STOKE shows gcc
 * transcribing (ASPLOS'13 Fig. 12). bvneg(bveq(...)) is the 0/-1 mask. */
uint64_t p21_cycle3(uint64_t x, uint64_t a, uint64_t b, uint64_t c) {
    return ((((0 - (uint64_t)(x == c)) & (a ^ c))
             ^ ((0 - (uint64_t)(x == a)) & (b ^ c))) ^ c);
}

/* P22: compute parity via the HD multiply trick, scaled 32->64:
 * 0x11111111 -> 0x1111111111111111, >>28 -> >>60. Computes the parity of
 * popcount over all 64 bits. */
uint64_t p22_parity(uint64_t x) {
    uint64_t o1 = x >> 1;
    uint64_t o2 = o1 ^ x;
    uint64_t o3 = o2 >> 2;
    uint64_t o4 = o2 ^ o3;
    uint64_t o5 = o4 & 0x1111111111111111;
    uint64_t o6 = o5 * 0x1111111111111111;
    uint64_t o7 = o6 >> 60;
    return o7 & 0x1;
}

/* P23: HD's popcount method up to per-byte counts, scaled 32->64
 * (0x55555555/0x33333333/0x0F0F0F0F doubled). The paper's Figure-4 line 6
 * prints bvand(o3, 0x33333333) — identical to its line 4, a transcription
 * typo; HD's method (the stated source) uses (o3 >> 2) & 0x33333333 there.
 * Implemented per HD; the deviation is documented here and in
 * src/benchmarks/compiler_full.rs. */
uint64_t p23_popcnt_bytes(uint64_t x) {
    uint64_t o1 = x >> 1;
    uint64_t o2 = o1 & 0x5555555555555555;
    uint64_t o3 = x - o2;
    uint64_t o4 = o3 & 0x3333333333333333;
    uint64_t o5 = (o3 >> 2) & 0x3333333333333333;
    uint64_t o7 = o4 + o5;
    uint64_t o8 = o7 >> 4;
    uint64_t o9 = o8 + o7;
    return o9 & 0x0F0F0F0F0F0F0F0F;
}

/* P24: round up to the next highest power of 2 (core-batch source unchanged;
 * the HD 32-bit smear scaled to 64) */
uint64_t next_pow2(uint64_t x) {
    x = x - 1;
    x |= x >> 1; x |= x >> 2; x |= x >> 4;
    x |= x >> 8; x |= x >> 16; x |= x >> 32;
    return x + 1;
}

/* P25: compute the higher-order half of the product of x and y — HD's
 * parts decomposition, scaled 16-bit parts -> 32-bit parts (0xFFFF ->
 * 0xFFFFFFFF, >>16 -> >>32) per the 64-bit-product title. The partial
 * products fit in 64 bits (32x32) and the aggregation order avoids
 * intermediate overflow, so the wrapping-64 result is exactly (x*y)>>64. */
uint64_t p25_mulhu(uint64_t x, uint64_t y) {
    uint64_t o1 = x & 0xFFFFFFFF;
    uint64_t o2 = x >> 32;
    uint64_t o3 = y & 0xFFFFFFFF;
    uint64_t o4 = y >> 32;
    uint64_t o5 = o1 * o3;
    uint64_t o6 = o2 * o3;
    uint64_t o7 = o1 * o4;
    uint64_t o8 = o2 * o4;
    uint64_t o9 = o5 >> 32;
    uint64_t o10 = o6 + o9;
    uint64_t o11 = o10 & 0xFFFFFFFF;
    uint64_t o12 = o10 >> 32;
    uint64_t o13 = o7 + o11;
    uint64_t o14 = o13 >> 32;
    uint64_t o15 = o14 + o12;
    return o15 + o8;
}

/* ------------------------------------------------ core-batch extras (unchanged) */

/* signed min */
int64_t smin(int64_t a, int64_t b) { return a < b ? a : b; }

/* unsigned min */
uint64_t umin(uint64_t a, uint64_t b) { return a < b ? a : b; }

/* unsigned max */
uint64_t umax(uint64_t a, uint64_t b) { return a > b ? a : b; }

/* sign mask: -1 if negative else 0 */
uint64_t sign_mask(int64_t x) { return (uint64_t)(x >> 63); }

/* select: HD cmov idiom */
uint64_t select_bits(uint64_t x, uint64_t m, uint64_t y) { return (x & m) | (y & ~m); }

/* saturating add (unsigned) */
uint64_t sat_add(uint64_t a, uint64_t b) {
    uint64_t s = a + b;
    return s < a ? UINT64_MAX : s;
}
