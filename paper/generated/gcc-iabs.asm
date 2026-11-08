movq %rdi, %rax
sarq $63, %rax
xorq %rdi, %rax
shrq $63, %rdi
addq %rdi, %rax
