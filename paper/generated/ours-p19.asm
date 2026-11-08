movq %rdi, %rax
movq %rdx, %rcx
shrq %cl, %rdi
xorq %rax, %rdi
andq %rsi, %rdi
xorq %rdi, %rax
shlq %cl, %rdi
xorq %rdi, %rax
