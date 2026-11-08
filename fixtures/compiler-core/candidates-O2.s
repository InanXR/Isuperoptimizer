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
	.globl	lowest_set_bit
	.type	lowest_set_bit, @function
lowest_set_bit:
.LFB1:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	negq	%rax
	andq	%rdi, %rax
	ret
	.cfi_endproc
.LFE1:
	.size	lowest_set_bit, .-lowest_set_bit
	.p2align 4
	.globl	next_pow2
	.type	next_pow2, @function
next_pow2:
.LFB2:
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
.LFE2:
	.size	next_pow2, .-next_pow2
	.p2align 4
	.globl	avg
	.type	avg, @function
avg:
.LFB3:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	andq	%rsi, %rdi
	xorq	%rsi, %rax
	shrq	%rax
	addq	%rdi, %rax
	ret
	.cfi_endproc
.LFE3:
	.size	avg, .-avg
	.p2align 4
	.globl	iabs
	.type	iabs, @function
iabs:
.LFB4:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	sarq	$63, %rax
	xorq	%rdi, %rax
	shrq	$63, %rdi
	addq	%rdi, %rax
	ret
	.cfi_endproc
.LFE4:
	.size	iabs, .-iabs
	.p2align 4
	.globl	smin
	.type	smin, @function
smin:
.LFB5:
	.cfi_startproc
	endbr64
	cmpq	%rdi, %rsi
	movq	%rdi, %rax
	cmovle	%rsi, %rax
	ret
	.cfi_endproc
.LFE5:
	.size	smin, .-smin
	.p2align 4
	.globl	umin
	.type	umin, @function
umin:
.LFB6:
	.cfi_startproc
	endbr64
	cmpq	%rdi, %rsi
	movq	%rdi, %rax
	cmovbe	%rsi, %rax
	ret
	.cfi_endproc
.LFE6:
	.size	umin, .-umin
	.p2align 4
	.globl	umax
	.type	umax, @function
umax:
.LFB7:
	.cfi_startproc
	endbr64
	cmpq	%rdi, %rsi
	movq	%rdi, %rax
	cmovnb	%rsi, %rax
	ret
	.cfi_endproc
.LFE7:
	.size	umax, .-umax
	.p2align 4
	.globl	cycle3
	.type	cycle3, @function
cycle3:
.LFB8:
	.cfi_startproc
	endbr64
	movq	%rsi, %rax
	cmpq	%rcx, %rdi
	je	.L10
	cmpq	%rsi, %rdi
	movq	%rdx, %rax
	cmovne	%rcx, %rax
.L10:
	ret
	.cfi_endproc
.LFE8:
	.size	cycle3, .-cycle3
	.p2align 4
	.globl	parity8
	.type	parity8, @function
parity8:
.LFB9:
	.cfi_startproc
	endbr64
	movzbl	%dil, %edi
	movq	%rdi, %rax
	shrq	$4, %rax
	xorq	%rdi, %rax
	movq	%rax, %rdi
	shrq	$2, %rdi
	xorq	%rax, %rdi
	movq	%rdi, %rax
	shrq	%rax
	xorq	%rdi, %rax
	andl	$1, %eax
	ret
	.cfi_endproc
.LFE9:
	.size	parity8, .-parity8
	.p2align 4
	.globl	popcount8
	.type	popcount8, @function
popcount8:
.LFB10:
	.cfi_startproc
	endbr64
	movzbl	%dil, %edi
	movq	%rdi, %rax
	shrq	%rax
	andl	$85, %eax
	subq	%rax, %rdi
	movq	%rdi, %rdx
	shrq	$2, %rdi
	andl	$51, %edx
	andl	$51, %edi
	addq	%rdi, %rdx
	movq	%rdx, %rax
	shrq	$4, %rax
	addq	%rdx, %rax
	andl	$15, %eax
	ret
	.cfi_endproc
.LFE10:
	.size	popcount8, .-popcount8
	.p2align 4
	.globl	sign_mask
	.type	sign_mask, @function
sign_mask:
.LFB11:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	sarq	$63, %rax
	ret
	.cfi_endproc
.LFE11:
	.size	sign_mask, .-sign_mask
	.p2align 4
	.globl	ult_flag
	.type	ult_flag, @function
ult_flag:
.LFB12:
	.cfi_startproc
	endbr64
	xorl	%eax, %eax
	cmpq	%rsi, %rdi
	setb	%al
	ret
	.cfi_endproc
.LFE12:
	.size	ult_flag, .-ult_flag
	.p2align 4
	.globl	select_bits
	.type	select_bits, @function
select_bits:
.LFB13:
	.cfi_startproc
	endbr64
	xorq	%rdx, %rdi
	andq	%rsi, %rdi
	movq	%rdi, %rax
	xorq	%rdx, %rax
	ret
	.cfi_endproc
.LFE13:
	.size	select_bits, .-select_bits
	.p2align 4
	.globl	sat_add
	.type	sat_add, @function
sat_add:
.LFB14:
	.cfi_startproc
	endbr64
	addq	%rsi, %rdi
	movq	$-1, %rax
	cmovnc	%rdi, %rax
	ret
	.cfi_endproc
.LFE14:
	.size	sat_add, .-sat_add
	.p2align 4
	.globl	bswap64
	.type	bswap64, @function
bswap64:
.LFB15:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	bswap	%rax
	ret
	.cfi_endproc
.LFE15:
	.size	bswap64, .-bswap64
	.p2align 4
	.globl	ctz64
	.type	ctz64, @function
ctz64:
.LFB16:
	.cfi_startproc
	endbr64
	xorl	%eax, %eax
	rep bsfq	%rdi, %rax
	cltq
	ret
	.cfi_endproc
.LFE16:
	.size	ctz64, .-ctz64
	.p2align 4
	.globl	div7
	.type	div7, @function
div7:
.LFB17:
	.cfi_startproc
	endbr64
	movabsq	$2635249153387078803, %rax
	mulq	%rdi
	subq	%rdx, %rdi
	shrq	%rdi
	leaq	(%rdx,%rdi), %rax
	shrq	$2, %rax
	ret
	.cfi_endproc
.LFE17:
	.size	div7, .-div7
	.p2align 4
	.globl	avg2
	.type	avg2, @function
avg2:
.LFB18:
	.cfi_startproc
	endbr64
	movq	%rdi, %rax
	movq	%rsi, %rdx
	andq	%rsi, %rdi
	shrq	%rax
	shrq	%rdx
	andl	$1, %edi
	addq	%rdx, %rax
	addq	%rdi, %rax
	ret
	.cfi_endproc
.LFE18:
	.size	avg2, .-avg2
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
