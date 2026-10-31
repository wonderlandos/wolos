/* SPDX-License-Identifier: GPL-2.0-only */
/* Host test for scx_wolos_class.h: make -C kernel/scx_wolos test */
#include <stdio.h>
#include "../scx_wolos_class.h"

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

int main(void)
{
	/* INTERFACE.md table */
	CHECK(wolos_class(0, 768, 1024) == C_INTERACTIVE);
	CHECK(wolos_class(0, 384, 1024) == C_FOREGROUND);	/* frontmost app */
	CHECK(wolos_class(0, 512, 1024) == C_FOREGROUND);	/* the bench probe */
	CHECK(wolos_class(0, 0, 1024) == C_DEFAULT);
	CHECK(wolos_class(0, 0, 512) == C_UTILITY);
	CHECK(wolos_class(WOLOS_SCHED_IDLE, 0, 256) == C_BACKGROUND);
	/* The frontmost app never shares a queue or a slice with batch work. */
	for (int b = C_DEFAULT; b < C_NR; b++) {
		CHECK(wolos_class_dsq(C_FOREGROUND) != wolos_class_dsq(b));
		CHECK(wolos_class_dsq(C_INTERACTIVE) != wolos_class_dsq(b));
		CHECK(wolos_class_slice(C_FOREGROUND) < wolos_class_slice(b));
		CHECK(wolos_class_preempts(C_FOREGROUND, b));
		CHECK(wolos_class_preempts(C_INTERACTIVE, b));
		CHECK(!wolos_class_preempts(b, C_FOREGROUND));
	}
	CHECK(wolos_class_preempts(C_INTERACTIVE, C_FOREGROUND));
	CHECK(!wolos_class_preempts(C_FOREGROUND, C_INTERACTIVE));
	CHECK(!wolos_class_preempts(C_FOREGROUND, C_FOREGROUND));
	CHECK(!wolos_class_preempts(C_DEFAULT, C_UTILITY));	/* batch never preempts */
	CHECK(wolos_class_dsq(C_DEFAULT) == DSQ_PCPU && wolos_class_dsq(C_UTILITY) == DSQ_PCPU);
	printf(fails ? "class_test: %d failed\n" : "class_test: ok\n", fails);
	return fails != 0;
}
