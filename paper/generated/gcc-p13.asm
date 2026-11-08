movq %rdi, %rax
sarq $63, %rdi
negq %rax
shrq $63, %rax
orq %rdi, %rax
