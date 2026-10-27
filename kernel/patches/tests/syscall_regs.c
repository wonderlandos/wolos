/* SPDX-License-Identifier: LicenseRef-Wonderland-1.0 -- Copyright (c) 2026 Julia Klee. WolOS kernel work under the Wonderland License 1.0: see LICENSES/Wonderland-1.0.md. */
/*
 * Linux syscall ABI: every register except rax (result), rcx and r11 survives `syscall`.
 * Guards kernel/patches/0002 (SYSRET fast return for Linux syscalls).
 * Build: cc -O2 -static -o syscall_regs syscall_regs.c   Exit 0 = pass.
 */
#include <stdint.h>
#include <stdio.h>
#include <sys/syscall.h>

int main(void)
{
	static const char *name[] = { "rdi", "rsi", "rdx", "r8", "r9", "r10", "r12", "r13", "r14", "r15", "rbx", "rbp" };
	uint64_t in[12], out[12];
	int fail = 0;

	for (int round = 0; round < 100000; round++) {
		for (int i = 0; i < 12; i++) in[i] = 0x1111111111111111ull * (i + 1) ^ (uint64_t)round;
		__asm__ volatile(
		    "push %%rbx\n push %%rbp\n push %%r12\n push %%r13\n push %%r14\n push %%r15\n"
		    "push %[out]\n"
		    "mov 0(%[in]),%%rdi\n mov 8(%[in]),%%rsi\n mov 16(%[in]),%%rdx\n"
		    "mov 24(%[in]),%%r8\n mov 32(%[in]),%%r9\n mov 40(%[in]),%%r10\n"
		    "mov 48(%[in]),%%r12\n mov 56(%[in]),%%r13\n mov 64(%[in]),%%r14\n"
		    "mov 72(%[in]),%%r15\n mov 80(%[in]),%%rbx\n mov 88(%[in]),%%rbp\n"
		    "mov %[nr],%%eax\n syscall\n"
		    "pop %%rax\n"
		    "mov %%rdi,0(%%rax)\n mov %%rsi,8(%%rax)\n mov %%rdx,16(%%rax)\n"
		    "mov %%r8,24(%%rax)\n mov %%r9,32(%%rax)\n mov %%r10,40(%%rax)\n"
		    "mov %%r12,48(%%rax)\n mov %%r13,56(%%rax)\n mov %%r14,64(%%rax)\n"
		    "mov %%r15,72(%%rax)\n mov %%rbx,80(%%rax)\n mov %%rbp,88(%%rax)\n"
		    "pop %%r15\n pop %%r14\n pop %%r13\n pop %%r12\n pop %%rbp\n pop %%rbx\n"
		    :
		    : [in] "r"(in), [out] "r"(out), [nr] "i"(SYS_getppid)
		    : "rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11", "memory");
		for (int i = 0; i < 12; i++)
			if (in[i] != out[i] && fail++ < 12)
				printf("FAIL round %d: %s %#llx -> %#llx\n", round, name[i],
				    (unsigned long long)in[i], (unsigned long long)out[i]);
		if (fail) break;
	}
	printf("%s\n", fail ? "FAIL" : "PASS: all registers preserved (100000 syscalls)");
	return fail != 0;
}
