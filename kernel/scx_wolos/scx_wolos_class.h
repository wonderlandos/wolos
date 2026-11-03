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
#define WOLOS_REQ_MIN		500000ULL	/* deadline request bounds */
#define WOLOS_REQ_MAX		16000000ULL
#define WOLOS_WAKE_GRAN_NS	250000ULL	/* a running task keeps its CPU this long */
#define WOLOS_LONG_RUN_NS	8000000ULL	/* asks for this much: a hog, fair to preempt */

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

/*
 * Audit A19: uclamp_min can be raised by any unprivileged task, so it alone must not buy a high
 * class. @grant is the class the root loader's `boosted` map gives the task's process (-1: none);
 * @grants_on says that map is in use. With grants on, an ungranted task is capped at default and
 * a granted one at its grant. With no map (fallback) uclamp is capped at foreground. Either way a
 * task in a high class that keeps running for WOLOS_HOG_NS per burst drops to default, so a
 * boosted loop cannot starve the rest.
 */
#define WOLOS_HOG_NS		(2 * WOLOS_LONG_RUN_NS)	/* 16 ms average burst */
#define WOLOS_GRANTS_KEY	0			/* map key present = grants in use */

static inline int wolos_class_guarded(int c, int grant, int grants_on,
				      unsigned long long avg_burst)
{
	int cap = grants_on ? (grant >= C_INTERACTIVE && grant < C_NR ? grant : C_DEFAULT) :
			      C_FOREGROUND;

	if (c < cap)
		c = cap;
	if (c < C_DEFAULT && avg_burst >= WOLOS_HOG_NS)
		c = C_DEFAULT;
	return c;
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

/* A waking task of class @waking may take a CPU running class @running. */
static inline int wolos_class_preempts(int waking, int running)
{
	return waking < running;
}

/* What a task asks for in its deadline: its usual burst, so short sleepers come first. */
static inline unsigned long long wolos_request(unsigned long long avg_burst,
					       unsigned long long burst)
{
	unsigned long long r = burst > avg_burst ? burst : avg_burst;

	if (r < WOLOS_REQ_MIN)
		return WOLOS_REQ_MIN;
	return r > WOLOS_REQ_MAX ? WOLOS_REQ_MAX : r;
}

/* Moving average over about four sleeps. */
static inline unsigned long long wolos_avg_burst(unsigned long long avg, unsigned long long burst)
{
	return avg - (avg >> 2) + (burst >> 2);
}

/* What a task stopped after @used of @given keeps; 0 when it is too little to keep. */
static inline unsigned long long wolos_slice_left(unsigned long long given,
						  unsigned long long used)
{
	unsigned long long left = given > used ? given - used : 0;

	return left >= WOLOS_WAKE_GRAN_NS ? left : 0;
}

/*
 * @a asks for clearly less than @b and @b is a hog. A ratio alone is not enough: a short message
 * thread would still cut into a worker that was about to finish and sleep.
 */
static inline int wolos_req_outranks(unsigned long long a, unsigned long long b)
{
	return a + a / 2 <= b && b >= WOLOS_LONG_RUN_NS;
}

/*
 * A wake preempts a lower class always. Within the default class it preempts a task with a
 * later deadline that asks for clearly more, unless that task only just started or the waker
 * is about to sleep and hand over its own CPU anyway (a pipe would bounce on every message).
 * Peers wait: taking the CPU from one stretches its request by the whole of the other's burst.
 */
static inline int wolos_wake_preempts(int waking, int running, unsigned long long wake_dl,
				      unsigned long long run_dl, unsigned long long wake_req,
				      unsigned long long run_req, unsigned long long ran_ns,
				      int sync)
{
	if (wolos_class_preempts(waking, running))
		return 1;
	if (waking != C_DEFAULT || running != C_DEFAULT || sync || ran_ns < WOLOS_WAKE_GRAN_NS)
		return 0;
	return (long long)(wake_dl - run_dl) < 0 && wolos_req_outranks(wake_req, run_req);
}

#endif
