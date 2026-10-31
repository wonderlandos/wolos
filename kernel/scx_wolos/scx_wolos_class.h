/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * scx_wolos class policy as pure functions: shared by scx_wolos.bpf.c and the host test
 * (tests/class_test.c). Inputs are what userspace set with sched_setattr(2) (INTERFACE.md).
 */
#ifndef SCX_WOLOS_CLASS_H
#define SCX_WOLOS_CLASS_H

enum { C_INTERACTIVE, C_FOREGROUND, C_DEFAULT, C_UTILITY, C_BACKGROUND, C_NR };

/* Shared queues; normal work goes to per-CPU queues DSQ_PCPU_BASE + cpu. */
#define DSQ_INTERACTIVE	0
#define DSQ_FG		1
#define DSQ_BG		2
#define DSQ_PCPU_BASE	16
#define DSQ_PCPU	(-1)	/* class_dsq(): "the per-CPU normal queue" */

#define WOLOS_SCHED_IDLE	5
#define WOLOS_SLICE_INTERACTIVE	1000000ULL	/* 1 ms */
#define WOLOS_SLICE_FG		2000000ULL	/* 2 ms */
#define WOLOS_SLICE_NORMAL	4000000ULL	/* 4 ms */

/* UserInteractive sets uclamp_min 768, the frontmost app (UserInitiated) 384. */
static inline int wolos_class(unsigned int policy, unsigned int umin, unsigned int umax)
{
	if (policy == WOLOS_SCHED_IDLE)
		return C_BACKGROUND;
	if (umin >= 768)
		return C_INTERACTIVE;
	if (umin >= 384)
		return C_FOREGROUND;
	if (umax <= 512)
		return C_UTILITY;
	return C_DEFAULT;
}

/* Queue per class: interactive and foreground never share a queue with batch work. */
static inline int wolos_class_dsq(int c)
{
	switch (c) {
	case C_INTERACTIVE: return DSQ_INTERACTIVE;
	case C_FOREGROUND: return DSQ_FG;
	case C_BACKGROUND: return DSQ_BG;
	default: return DSQ_PCPU;
	}
}

static inline unsigned long long wolos_class_slice(int c)
{
	return c == C_INTERACTIVE ? WOLOS_SLICE_INTERACTIVE :
	       c == C_FOREGROUND ? WOLOS_SLICE_FG : WOLOS_SLICE_NORMAL;
}

/* A waking task of class @waking preempts a CPU running class @running. */
static inline int wolos_class_preempts(int waking, int running)
{
	return waking <= C_FOREGROUND && running > waking;
}

#endif
