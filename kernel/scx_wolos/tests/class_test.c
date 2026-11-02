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
	/* An untagged game thread takes the CPU from batch work: most games are untagged. */
	CHECK(wolos_class_preempts(C_DEFAULT, C_BACKGROUND));
	CHECK(wolos_class_preempts(C_DEFAULT, C_UTILITY));
	CHECK(wolos_class_preempts(C_UTILITY, C_BACKGROUND));
	CHECK(!wolos_class_preempts(C_DEFAULT, C_DEFAULT));
	CHECK(!wolos_class_preempts(C_BACKGROUND, C_DEFAULT));
	CHECK(wolos_class_dsq(C_DEFAULT) == DSQ_PCPU && wolos_class_dsq(C_UTILITY) == DSQ_PCPU);
	/* A19: uclamp alone does not buy a high class. */
	CHECK(wolos_class_guarded(C_INTERACTIVE, -1, 0, 0) == C_FOREGROUND);	/* no map: cap */
	CHECK(wolos_class_guarded(C_FOREGROUND, -1, 0, 0) == C_FOREGROUND);
	CHECK(wolos_class_guarded(C_INTERACTIVE, -1, 1, 0) == C_DEFAULT);	/* map, no grant */
	CHECK(wolos_class_guarded(C_INTERACTIVE, C_INTERACTIVE, 1, 0) == C_INTERACTIVE);
	CHECK(wolos_class_guarded(C_INTERACTIVE, C_FOREGROUND, 1, 0) == C_FOREGROUND);
	CHECK(wolos_class_guarded(C_FOREGROUND, C_INTERACTIVE, 1, 0) == C_FOREGROUND);
	CHECK(wolos_class_guarded(C_INTERACTIVE, 99, 1, 0) == C_DEFAULT);	/* bad value */
	CHECK(wolos_class_guarded(C_INTERACTIVE, C_INTERACTIVE, 1, WOLOS_HOG_NS) == C_DEFAULT);
	CHECK(wolos_class_guarded(C_FOREGROUND, -1, 0, WOLOS_HOG_NS) == C_DEFAULT);
	CHECK(wolos_class_guarded(C_UTILITY, C_INTERACTIVE, 1, 0) == C_UTILITY);
	/* Deadline request: a sleeper asks for its burst, a hog for the cap. */
	CHECK(wolos_request(0, 0) == WOLOS_REQ_MIN);
	CHECK(wolos_request(4000000, 0) == 4000000);
	CHECK(wolos_request(4000000, 6000000) == 6000000);	/* a sleeper turned hog */
	CHECK(wolos_request(100000000, 0) == WOLOS_REQ_MAX);
	CHECK(wolos_avg_burst(0, 4000000) == 1000000);
	CHECK(wolos_avg_burst(4000000, 4000000) == 4000000);
	/* A preempted task keeps what is left of its slice; a used-up slice starts fresh. */
	CHECK(wolos_slice_left(4000000, 1000000) == 3000000);
	CHECK(wolos_slice_left(4000000, 4000000) == 0);
	CHECK(wolos_slice_left(4000000, 5000000) == 0);
	CHECK(wolos_slice_left(4000000, 3900000) == 0);	/* too little left to be worth keeping */
	/* Wake rules. Same-class preemption needs an earlier deadline and a much smaller ask. */
	CHECK(wolos_req_outranks(4000000, WOLOS_REQ_MAX));	/* frame thread over a hog */
	CHECK(wolos_req_outranks(10000000, WOLOS_REQ_MAX));	/* 60 Hz render thread too */
	CHECK(!wolos_req_outranks(4000000, 4000000));		/* peers never trade CPUs */
	CHECK(!wolos_req_outranks(4000000, 5000000));
	CHECK(!wolos_req_outranks(500000, 4000000));	/* never a task that sleeps soon */
	static const struct {
		int w, r; unsigned long long wd, rd, wreq, rreq, ran; int sync, want;
	} wake[] = {
		{ C_DEFAULT, C_BACKGROUND, 9, 1, 8, 1, 0, 1, 1 },	/* batch gives way, always */
		{ C_DEFAULT, C_UTILITY, 9, 1, 8, 1, 0, 0, 1 },
		{ C_DEFAULT, C_DEFAULT, 1, 9, 4000000, 16000000, 1000000, 0, 1 },
		{ C_DEFAULT, C_DEFAULT, 9, 1, 4000000, 16000000, 1000000, 0, 0 },	/* later deadline */
		{ C_DEFAULT, C_DEFAULT, 1, 9, 4000000, 16000000, 1000, 0, 0 },	/* just started */
		{ C_DEFAULT, C_DEFAULT, 1, 9, 4000000, 16000000, 1000000, 1, 0 },	/* sync hand-over */
		{ C_DEFAULT, C_DEFAULT, 1, 9, 4000000, 4000000, 1000000, 0, 0 },	/* equal peers */
		{ C_DEFAULT, C_FOREGROUND, 1, 9, 4000000, 16000000, 1000000, 0, 0 },
		{ C_UTILITY, C_UTILITY, 1, 9, 4000000, 16000000, 1000000, 0, 0 },
		{ C_BACKGROUND, C_BACKGROUND, 1, 9, 4000000, 16000000, 1000000, 0, 0 },
		{ C_FOREGROUND, C_DEFAULT, 9, 1, 8, 1, 0, 1, 1 },
		{ C_DEFAULT, C_DEFAULT, 1ULL, ~0ULL, 4000000, 16000000, 1000000, 0, 0 }, /* wrap: 1 is later */
	};
	for (unsigned int i = 0; i < sizeof(wake) / sizeof(wake[0]); i++)
		if (wolos_wake_preempts(wake[i].w, wake[i].r, wake[i].wd, wake[i].rd, wake[i].wreq,
					wake[i].rreq, wake[i].ran, wake[i].sync) != wake[i].want) {
			printf("FAIL wake rule %u\n", i);
			fails++;
		}
	printf(fails ? "class_test: %d failed\n" : "class_test: ok\n", fails);
	return fails != 0;
}
