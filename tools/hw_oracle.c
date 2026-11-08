/* hw_oracle.c — execute single instructions on real x86-64 silicon and
 * report results + flags, for differential testing of our concrete
 * semantics. Reads lines:  <op> <a> <b> <count>
 * where a/b/count are 64-bit hex (0x...). Prints:  r zf sf cf of
 * ('-' in a flag field means undefined/uninteresting: do not compare).
 * For divq/idivq prints:  rax rdx - - - -   and the literal line "def"
 * when the inputs would fault (#DE: divisor 0 or quotient overflow) —
 * matching our definedness predicate instead of faulting.
 *
 * Memory ops: a static 256-byte buffer is initialized with the shared byte
 * formula buf[i] = (i*7+3) & 0xff, and its absolute address is printed as
 * the first line: "MEMBASE 0x...". The Rust side passes absolute addresses
 * (MEMBASE + offset) in the `a` operand, mirroring our sandbox model.
 *
 * Ops: add sub cmp and or xor imul shl shr sar
 *      sete..setle cmove..cmovle (as before)
 *      neg not            (one-operand; `not` pre-sets flags via subq to
 *                         check NOT leaves them untouched)
 *      divq idivq        (a -> RAX, b -> divisor, count -> RDX)
 *      load              (a -> address; RAX-style movq (%rax),%rcx)
 *      storeload         (a -> address, b -> value; store then load it back)
 *
 * Build: cc -O2 -o hw_oracle hw_oracle.c
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define F_ZF(f) (((f) >> 6) & 1)
#define F_SF(f) (((f) >> 7) & 1)
#define F_CF(f) (((f) >> 0) & 1)
#define F_OF(f) (((f) >> 11) & 1)

/* Each handler: res starts as `a`; the asm performs the op; flags captured
 * after it. "cc" is listed for clarity (implicit on x86). */
#define BINOP(NAME, ASM)                                                       \
    static uint64_t run_##NAME(uint64_t a, uint64_t b, uint64_t *flags) {      \
        uint64_t res = a, fl;                                                  \
        asm volatile(ASM "\n\tpushfq\n\tpopq %1"                               \
                         : "+r"(res), "=r"(fl)                                 \
                         : "r"(b)                                              \
                         : "cc");                                              \
        *flags = fl;                                                           \
        return res;                                                            \
    }

BINOP(add, "addq %2, %0")
BINOP(sub, "subq %2, %0")
BINOP(cmp, "cmpq %2, %0")
BINOP(and, "andq %2, %0")
BINOP(or, "orq %2, %0")
BINOP(xor, "xorq %2, %0")
BINOP(imul, "imulq %2, %0")

#define SHIFTOP(NAME, ASM)                                                     \
    static uint64_t run_##NAME(uint64_t a, uint64_t cnt, uint64_t *flags) {    \
        uint64_t res = a, fl;                                                  \
        asm volatile(ASM "\n\tpushfq\n\tpopq %1"                               \
                         : "+r"(res), "=r"(fl)                                 \
                         : "c"((uint8_t)cnt)                                   \
                         : "cc");                                              \
        *flags = fl;                                                           \
        return res;                                                            \
    }

SHIFTOP(shl, "shlq %b2, %0")
SHIFTOP(shr, "shrq %b2, %0")
SHIFTOP(sar, "sarq %b2, %0")

/* neg: one operand; flags captured like the binops. */
static uint64_t run_neg(uint64_t a, uint64_t b, uint64_t *flags) {
    uint64_t res = a, fl;
    asm volatile("negq %0\n\tpushfq\n\tpopq %1"
                         : "+r"(res), "=r"(fl)
                         :
                         : "cc");
    *flags = fl;
    (void)b;
    return res;
}

/* not: one operand; per the SDM NOT leaves RFLAGS untouched, so the
 * handler PRE-SETS flags with a subq on the same operand (the Rust side
 * evaluates that sub in the model first and compares all four flags
 * after the notq) — this checks the flags-untouched claim on silicon. */
static uint64_t run_not(uint64_t a, uint64_t b, uint64_t *flags) {
    uint64_t res = a, fl;
    asm volatile("subq %2, %0\n\tnotq %0\n\tpushfq\n\tpopq %1"
                     : "+r"(res), "=r"(fl)
                     : "r"(b)
                     : "cc");
    *flags = fl;
    return res;
}

/* cmp + setcc: returns 0/1. */
#define SETCC(NAME, SUF)                                                       \
    static uint64_t run_##NAME(uint64_t a, uint64_t b, uint64_t *flags) {       \
        uint64_t fl, out = 0;                                                  \
        asm volatile("cmpq %2, %1\n\tset" SUF " %b0"                           \
                         : "+r"(out)                                            \
                         : "r"(a), "r"(b)                                      \
                         : "cc");                                              \
        asm volatile("pushfq\n\tpopq %0" : "=r"(fl) :: "cc");                   \
        *flags = fl;                                                           \
        return out & 1;                                                        \
    }

SETCC(sete, "e")
SETCC(setne, "ne")
SETCC(seta, "a")
SETCC(setae, "ae")
SETCC(setb, "b")
SETCC(setbe, "be")
SETCC(setg, "g")
SETCC(setge, "ge")
SETCC(setl, "l")
SETCC(setle, "le")

/* cmp + cmovcc: res = cond ? b : a. */
#define CMOV(NAME, SUF)                                                        \
    static uint64_t run_##NAME(uint64_t a, uint64_t b, uint64_t *flags) {       \
        uint64_t res = a, fl;                                                  \
        asm volatile("cmpq %1, %0\n\tcmov" SUF "q %1, %0"                      \
                         : "+r"(res)                                            \
                         : "r"(b)                                              \
                         : "cc");                                              \
        asm volatile("pushfq\n\tpopq %0" : "=r"(fl) :: "cc");                   \
        *flags = fl;                                                           \
        return res;                                                            \
    }

CMOV(cmove, "e")
CMOV(cmovne, "ne")
CMOV(cmova, "a")
CMOV(cmovae, "ae")
CMOV(cmovb, "b")
CMOV(cmovbe, "be")
CMOV(cmovg, "g")
CMOV(cmovge, "ge")
CMOV(cmovl, "l")
CMOV(cmovle, "le")

/* ---- division: a -> RAX, b -> divisor, count -> RDX. #DE pre-checked to
 * mirror our definedness predicate (the process must not fault). */
static int div_precheck(uint64_t a, uint64_t b, uint64_t rdx, int signed_div) {
    if (b == 0) return 0;
    if (!signed_div) {
        __uint128_t dividend = ((__uint128_t)rdx << 64) | a;
        return dividend / b <= UINT64_MAX ? 1 : 0;
    } else {
        __int128_t dividend = (__int128_t)(((__uint128_t)rdx << 64) | a);
        __int128_t divisor = (__int128_t)(int64_t)b;
        __int128_t q = dividend / divisor;
        return q >= INT64_MIN && q <= INT64_MAX ? 1 : 0;
    }
}

static void run_divq(uint64_t a, uint64_t b, uint64_t rdx, uint64_t *o_rax, uint64_t *o_rdx) {
    uint64_t res = a, hi = rdx, div = b;
    asm volatile("divq %2" : "+a"(res), "+d"(hi) : "b"(div) : "cc");
    *o_rax = res;
    *o_rdx = hi;
}

static void run_idivq(uint64_t a, uint64_t b, uint64_t rdx, uint64_t *o_rax, uint64_t *o_rdx) {
    uint64_t res = a, hi = rdx, div = b;
    asm volatile("idivq %2" : "+a"(res), "+d"(hi) : "b"(div) : "cc");
    *o_rax = res;
    *o_rdx = hi;
}

/* ---- memory: the shared sandbox buffer. */
static uint8_t membuf[256];

static uint64_t run_load(uint64_t addr) {
    uint64_t out;
    asm volatile("movq (%1), %0" : "=r"(out) : "r"(addr));
    return out;
}

static uint64_t run_storeload(uint64_t addr, uint64_t value) {
    uint64_t out;
    asm volatile("movq %1, (%2)\n\tmovq (%2), %0"
                 : "=&r"(out)
                 : "r"(value), "r"(addr)
                 : "memory");
    return out;
}

int main(void) {
    for (int i = 0; i < 256; i++) {
        membuf[i] = (uint8_t)((i * 7 + 3) & 0xff);
    }
    printf("MEMBASE %llx\n", (unsigned long long)(uintptr_t)membuf);
    fflush(stdout);

    char op[32];
    unsigned long long a, b, cnt;
    while (scanf("%31s %llx %llx %llx", op, &a, &b, &cnt) == 4) {
        uint64_t flags = 0, r;
        if (!strcmp(op, "divq") || !strcmp(op, "idivq")) {
            int sdiv = (op[0] == 'i');
            if (!div_precheck(a, b, cnt, sdiv)) {
                printf("def\n");
                fflush(stdout);
                continue;
            }
            uint64_t o_rax, o_rdx;
            if (sdiv) run_idivq(a, b, cnt, &o_rax, &o_rdx);
            else run_divq(a, b, cnt, &o_rax, &o_rdx);
            printf("%llx %llx - - - -\n",
                   (unsigned long long)o_rax, (unsigned long long)o_rdx);
            fflush(stdout);
            continue;
        }
        if (!strcmp(op, "load")) {
            r = run_load(a);
            printf("%llx - - - - -\n", (unsigned long long)r);
            fflush(stdout);
            continue;
        }
        if (!strcmp(op, "storeload")) {
            r = run_storeload(a, b);
            printf("%llx - - - - -\n", (unsigned long long)r);
            fflush(stdout);
            continue;
        }
        if (!strcmp(op, "add")) r = run_add(a, b, &flags);
        else if (!strcmp(op, "sub")) r = run_sub(a, b, &flags);
        else if (!strcmp(op, "cmp")) r = run_cmp(a, b, &flags);
        else if (!strcmp(op, "and")) r = run_and(a, b, &flags);
        else if (!strcmp(op, "or")) r = run_or(a, b, &flags);
        else if (!strcmp(op, "xor")) r = run_xor(a, b, &flags);
        else if (!strcmp(op, "imul")) r = run_imul(a, b, &flags);
        else if (!strcmp(op, "shl")) r = run_shl(a, cnt, &flags);
        else if (!strcmp(op, "shr")) r = run_shr(a, cnt, &flags);
        else if (!strcmp(op, "sar")) r = run_sar(a, cnt, &flags);
        else if (!strcmp(op, "neg")) r = run_neg(a, b, &flags);
        else if (!strcmp(op, "not")) r = run_not(a, b, &flags);
        else if (!strcmp(op, "sete")) r = run_sete(a, b, &flags);
        else if (!strcmp(op, "setne")) r = run_setne(a, b, &flags);
        else if (!strcmp(op, "seta")) r = run_seta(a, b, &flags);
        else if (!strcmp(op, "setae")) r = run_setae(a, b, &flags);
        else if (!strcmp(op, "setb")) r = run_setb(a, b, &flags);
        else if (!strcmp(op, "setbe")) r = run_setbe(a, b, &flags);
        else if (!strcmp(op, "setg")) r = run_setg(a, b, &flags);
        else if (!strcmp(op, "setge")) r = run_setge(a, b, &flags);
        else if (!strcmp(op, "setl")) r = run_setl(a, b, &flags);
        else if (!strcmp(op, "setle")) r = run_setle(a, b, &flags);
        else if (!strcmp(op, "cmove")) r = run_cmove(a, b, &flags);
        else if (!strcmp(op, "cmovne")) r = run_cmovne(a, b, &flags);
        else if (!strcmp(op, "cmova")) r = run_cmova(a, b, &flags);
        else if (!strcmp(op, "cmovae")) r = run_cmovae(a, b, &flags);
        else if (!strcmp(op, "cmovb")) r = run_cmovb(a, b, &flags);
        else if (!strcmp(op, "cmovbe")) r = run_cmovbe(a, b, &flags);
        else if (!strcmp(op, "cmovg")) r = run_cmovg(a, b, &flags);
        else if (!strcmp(op, "cmovge")) r = run_cmovge(a, b, &flags);
        else if (!strcmp(op, "cmovl")) r = run_cmovl(a, b, &flags);
        else if (!strcmp(op, "cmovle")) r = run_cmovle(a, b, &flags);
        else { fprintf(stderr, "unknown op %s\n", op); return 2; }
        printf("%llx %d %d %d %d\n", (unsigned long long)r,
               (int)F_ZF(flags), (int)F_SF(flags), (int)F_CF(flags), (int)F_OF(flags));
        fflush(stdout);
    }
    return 0;
}
