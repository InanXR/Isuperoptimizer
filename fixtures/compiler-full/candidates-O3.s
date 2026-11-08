	.file	"candidates.c"
	.text
	.p2align 4
	.globl	turn_off_rightmost
	.type	turn_off_rightmost, @function
turn_off_rightmost:
.LFB0:
	.cfi_startproc
	endbr64
	leaq	-1(%rdi), %rax
	andq	%rdi, %rax
	ret
	.cfi_endproc
.LFE0:
	.size	turn_off_rightmost, .-turn_off_rightmost
	.p2align 4
	.globl	p02_test_2n_minus_1
	.type	p02_test_2n_minus_1, @function
p02_test_2n_minus_1:
.LFB1:
	.cfi_startproc
	endbr64
	leaq	1(%rdi), %rax
	andq	%rdi, %rax
	ret
	.cfi_endproc
.LFE1:
	.size	p02_test_2n_minus_1, .-p02_test_2n_minus_1
	.p2align 4
	.globl	lowest_set_bit
	.type	lowest_set_bit, @function
lowest_set_bit:
.LFB2:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	negq	%rax
	andq	%rdi, %rax
	ret
	.cfi_endproc
.LFE2:
	.size	lowest_set_bit, .-lowest_set_bit
	.p2align 4
	.globl	p04_mask_lowest_ones
	.type	p04_mask_lowest_ones, @function
p04_mask_lowest_ones:
.LFB3:
	.cfi_startproc
	endbr64
	leaq	-1(%rdi), %rax
	xorq	%rdi, %rax
	ret
	.cfi_endproc
.LFE3:
	.size	p04_mask_lowest_ones, .-p04_mask_lowest_ones
	.p2align 4
	.globl	p05_smear_rightmost
	.type	p05_smear_rightmost, @function
p05_smear_rightmost:
.LFB4:
	.cfi_startproc
	endbr64
	leaq	-1(%rdi), %rax
	orq	%rdi, %rax
	ret
	.cfi_endproc
.LFE4:
	.size	p05_smear_rightmost, .-p05_smear_rightmost
	.p2align 4
	.globl	p06_turn_on_rightmost0
	.type	p06_turn_on_rightmost0, @function
p06_turn_on_rightmost0:
.LFB5:
	.cfi_startproc
	endbr64
	leaq	1(%rdi), %rax
	orq	%rdi, %rax
	ret
	.cfi_endproc
.LFE5:
	.size	p06_turn_on_rightmost0, .-p06_turn_on_rightmost0
	.p2align 4
	.globl	p07_isolate_rightmost0
	.type	p07_isolate_rightmost0, @function
p07_isolate_rightmost0:
.LFB6:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	addq	$1, %rdi
	notq	%rax
	andq	%rdi, %rax
	ret
	.cfi_endproc
.LFE6:
	.size	p07_isolate_rightmost0, .-p07_isolate_rightmost0
	.p2align 4
	.globl	p08_mask_trailing_zeros
	.type	p08_mask_trailing_zeros, @function
p08_mask_trailing_zeros:
.LFB7:
	.cfi_startproc
	endbr64
	leaq	-1(%rdi), %rax
	notq	%rdi
	andq	%rdi, %rax
	ret
	.cfi_endproc
.LFE7:
	.size	p08_mask_trailing_zeros, .-p08_mask_trailing_zeros
	.p2align 4
	.globl	iabs
	.type	iabs, @function
iabs:
.LFB8:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	sarq	$63, %rax
	xorq	%rdi, %rax
	shrq	$63, %rdi
	addq	%rdi, %rax
	ret
	.cfi_endproc
.LFE8:
	.size	iabs, .-iabs
	.p2align 4
	.globl	p10_nlz_eq
	.type	p10_nlz_eq, @function
p10_nlz_eq:
.LFB9:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	andq	%rsi, %rdi
	xorq	%rsi, %rax
	cmpq	%rax, %rdi
	setnb	%al
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE9:
	.size	p10_nlz_eq, .-p10_nlz_eq
	.p2align 4
	.globl	p11_nlz_lt
	.type	p11_nlz_lt, @function
p11_nlz_lt:
.LFB10:
	.cfi_startproc
	endbr64
	movq	%rsi, %rax
	notq	%rax
	andq	%rdi, %rax
	cmpq	%rax, %rsi
	setb	%al
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE10:
	.size	p11_nlz_lt, .-p11_nlz_lt
	.p2align 4
	.globl	p12_nlz_le
	.type	p12_nlz_le, @function
p12_nlz_le:
.LFB11:
	.cfi_startproc
	endbr64
	movq	%rsi, %rax
	notq	%rax
	andq	%rdi, %rax
	cmpq	%rax, %rsi
	setnb	%al
	movzbl	%al, %eax
	ret
	.cfi_endproc
.LFE11:
	.size	p12_nlz_le, .-p12_nlz_le
	.p2align 4
	.globl	p13_sign
	.type	p13_sign, @function
p13_sign:
.LFB12:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	sarq	$63, %rdi
	negq	%rax
	shrq	$63, %rax
	orq	%rdi, %rax
	ret
	.cfi_endproc
.LFE12:
	.size	p13_sign, .-p13_sign
	.p2align 4
	.globl	avg
	.type	avg, @function
avg:
.LFB13:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	andq	%rsi, %rdi
	xorq	%rsi, %rax
	shrq	%rax
	addq	%rdi, %rax
	ret
	.cfi_endproc
.LFE13:
	.size	avg, .-avg
	.p2align 4
	.globl	p15_avg_ceil
	.type	p15_avg_ceil, @function
p15_avg_ceil:
.LFB14:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	xorq	%rsi, %rdi
	orq	%rsi, %rax
	shrq	%rdi
	subq	%rdi, %rax
	ret
	.cfi_endproc
.LFE14:
	.size	p15_avg_ceil, .-p15_avg_ceil
	.p2align 4
	.globl	p16_umax_bitwise
	.type	p16_umax_bitwise, @function
p16_umax_bitwise:
.LFB15:
	.cfi_startproc
	endbr64
	cmpq	%rsi, %rdi
	movq	%rsi, %rax
	cmovnb	%rdi, %rax
	ret
	.cfi_endproc
.LFE15:
	.size	p16_umax_bitwise, .-p16_umax_bitwise
	.p2align 4
	.globl	p17_turn_off_run
	.type	p17_turn_off_run, @function
p17_turn_off_run:
.LFB16:
	.cfi_startproc
	endbr64
	leaq	-1(%rdi), %rax
	orq	%rdi, %rax
	addq	$1, %rax
	andq	%rdi, %rax
	ret
	.cfi_endproc
.LFE16:
	.size	p17_turn_off_run, .-p17_turn_off_run
	.p2align 4
	.globl	p18_is_pow2
	.type	p18_is_pow2, @function
p18_is_pow2:
.LFB17:
	.cfi_startproc
	endbr64
	leaq	-1(%rdi), %rax
	testq	%rdi, %rax
	sete	%dl
	xorl	%eax, %eax
	testq	%rdi, %rdi
	setne	%al
	andq	%rdx, %rax
	ret
	.cfi_endproc
.LFE17:
	.size	p18_is_pow2, .-p18_is_pow2
	.p2align 4
	.globl	p19_exchange_fields
	.type	p19_exchange_fields, @function
p19_exchange_fields:
.LFB18:
	.cfi_startproc
	endbr64
	movq	%rdx, %rcx
	movq	%rdi, %r8
	shrq	%cl, %rdi
	xorq	%r8, %rdi
	andq	%rsi, %rdi
	movq	%rdi, %rax
	xorq	%rdi, %r8
	salq	%cl, %rax
	xorq	%r8, %rax
	ret
	.cfi_endproc
.LFE18:
	.size	p19_exchange_fields, .-p19_exchange_fields
	.p2align 4
	.globl	p20_snoob
	.type	p20_snoob, @function
p20_snoob:
.LFB19:
	.cfi_startproc
	endbr64
	movq	%rdi, %rcx
	movq	%rdi, %rax
	xorl	%edx, %edx
	negq	%rcx
	andq	%rdi, %rcx
	leaq	(%rdi,%rcx), %rsi
	xorq	%rsi, %rax
	shrq	$2, %rax
	divq	%rcx
	orq	%rsi, %rax
	ret
	.cfi_endproc
.LFE19:
	.size	p20_snoob, .-p20_snoob
	.p2align 4
	.globl	p21_cycle3
	.type	p21_cycle3, @function
p21_cycle3:
.LFB20:
	.cfi_startproc
	endbr64
	movq	%rdi, %r8
	movq	%rcx, %rax
	movq	%rsi, %rdi
	xorq	%rsi, %rax
	xorl	%esi, %esi
	cmpq	%rcx, %r8
	cmovne	%rsi, %rax
	xorq	%rcx, %rdx
	cmpq	%rdi, %r8
	cmovne	%rsi, %rdx
	xorq	%rdx, %rax
	xorq	%rcx, %rax
	ret
	.cfi_endproc
.LFE20:
	.size	p21_cycle3, .-p21_cycle3
	.p2align 4
	.globl	p22_parity
	.type	p22_parity, @function
p22_parity:
.LFB21:
	.cfi_startproc
	endbr64
	movq	%rdi, %rdx
	shrq	%rdx
	xorq	%rdi, %rdx
	movq	%rdx, %rax
	shrq	$2, %rax
	xorq	%rdx, %rax
	movabsq	$1229782938247303441, %rdx
	andq	%rdx, %rax
	imulq	%rdx, %rax
	shrq	$60, %rax
	andl	$1, %eax
	ret
	.cfi_endproc
.LFE21:
	.size	p22_parity, .-p22_parity
	.p2align 4
	.globl	p23_popcnt_bytes
	.type	p23_popcnt_bytes, @function
p23_popcnt_bytes:
.LFB22:
	.cfi_startproc
	endbr64
	movabsq	$6148914691236517205, %rdx
	movq	%rdi, %rax
	shrq	%rax
	andq	%rdx, %rax
	subq	%rax, %rdi
	movabsq	$3689348814741910323, %rax
	movq	%rdi, %rdx
	andq	%rax, %rdi
	shrq	$2, %rdx
	andq	%rax, %rdx
	addq	%rdi, %rdx
	movq	%rdx, %rax
	shrq	$4, %rax
	addq	%rdx, %rax
	movabsq	$1085102592571150095, %rdx
	andq	%rdx, %rax
	ret
	.cfi_endproc
.LFE22:
	.size	p23_popcnt_bytes, .-p23_popcnt_bytes
	.p2align 4
	.globl	next_pow2
	.type	next_pow2, @function
next_pow2:
.LFB23:
	.cfi_startproc
	endbr64
	subq	$1, %rdi
	movq	%rdi, %rax
	shrq	%rax
	orq	%rdi, %rax
	movq	%rax, %rdi
	shrq	$2, %rdi
	orq	%rax, %rdi
	movq	%rdi, %rdx
	shrq	$4, %rdx
	orq	%rdi, %rdx
	movq	%rdx, %rax
	shrq	$8, %rax
	orq	%rdx, %rax
	movq	%rax, %rdx
	shrq	$16, %rdx
	orq	%rax, %rdx
	movq	%rdx, %rax
	shrq	$32, %rax
	orq	%rdx, %rax
	addq	$1, %rax
	ret
	.cfi_endproc
.LFE23:
	.size	next_pow2, .-next_pow2
	.p2align 4
	.globl	p25_mulhu
	.type	p25_mulhu, @function
p25_mulhu:
.LFB24:
	.cfi_startproc
	endbr64
	movl	%edi, %eax
	movl	%esi, %ecx
	shrq	$32, %rdi
	movq	%rax, %rdx
	shrq	$32, %rsi
	imulq	%rcx, %rdx
	imulq	%rdi, %rcx
	imulq	%rsi, %rax
	shrq	$32, %rdx
	imulq	%rsi, %rdi
	addq	%rcx, %rdx
	movl	%edx, %ecx
	shrq	$32, %rdx
	addq	%rcx, %rax
	addq	%rdx, %rdi
	shrq	$32, %rax
	addq	%rdi, %rax
	ret
	.cfi_endproc
.LFE24:
	.size	p25_mulhu, .-p25_mulhu
	.p2align 4
	.globl	smin
	.type	smin, @function
smin:
.LFB25:
	.cfi_startproc
	endbr64
	cmpq	%rdi, %rsi
	movq	%rdi, %rax
	cmovle	%rsi, %rax
	ret
	.cfi_endproc
.LFE25:
	.size	smin, .-smin
	.p2align 4
	.globl	umin
	.type	umin, @function
umin:
.LFB26:
	.cfi_startproc
	endbr64
	cmpq	%rdi, %rsi
	movq	%rdi, %rax
	cmovbe	%rsi, %rax
	ret
	.cfi_endproc
.LFE26:
	.size	umin, .-umin
	.p2align 4
	.globl	umax
	.type	umax, @function
umax:
.LFB27:
	.cfi_startproc
	endbr64
	cmpq	%rdi, %rsi
	movq	%rdi, %rax
	cmovnb	%rsi, %rax
	ret
	.cfi_endproc
.LFE27:
	.size	umax, .-umax
	.p2align 4
	.globl	sign_mask
	.type	sign_mask, @function
sign_mask:
.LFB28:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	sarq	$63, %rax
	ret
	.cfi_endproc
.LFE28:
	.size	sign_mask, .-sign_mask
	.p2align 4
	.globl	select_bits
	.type	select_bits, @function
select_bits:
.LFB29:
	.cfi_startproc
	endbr64
	xorq	%rdx, %rdi
	andq	%rsi, %rdi
	movq	%rdi, %rax
	xorq	%rdx, %rax
	ret
	.cfi_endproc
.LFE29:
	.size	select_bits, .-select_bits
	.p2align 4
	.globl	sat_add
	.type	sat_add, @function
sat_add:
.LFB30:
	.cfi_startproc
	endbr64
	addq	%rsi, %rdi
	movq	$-1, %rax
	cmovnc	%rdi, %rax
	ret
	.cfi_endproc
.LFE30:
	.size	sat_add, .-sat_add
	.ident	"GCC: (Ubuntu 15.2.0-16ubuntu1) 15.2.0"
	.section	.note.GNU-stack,"",@progbits
	.section	.note.gnu.property,"a"
	.align 8
	.long	1f - 0f
	.long	4f - 1f
	.long	5
0:
	.string	"GNU"
1:
	.align 8
	.long	0xc0000002
	.long	3f - 2f
2:
	.long	0x3
3:
	.align 8
4:
