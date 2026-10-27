/* SPDX-License-Identifier: LicenseRef-Wonderland-1.0 -- Copyright (c) 2026 Julia Klee. WolOS kernel work under the Wonderland License 1.0: see LICENSES/Wonderland-1.0.md. */
/*
 * futex_waitv(2) conformance test + ping-pong latency, for kernel/patches/0001.
 * Build (on a Linux host): cc -O2 -static -pthread -o futex_waitv futex_waitv.c
 * Run the same binary on Linux (reference) and under the FreeBSD Linuxulator.
 * Exit 0 = all cases pass.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <linux/futex.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#ifndef SYS_futex_waitv
#define SYS_futex_waitv 449
#endif
#ifndef FUTEX2_SIZE_U32
#define FUTEX2_SIZE_U32 0x02
#endif
#ifndef FUTEX2_PRIVATE
#define FUTEX2_PRIVATE 128
#endif

struct waitv { uint64_t val, uaddr; uint32_t flags, reserved; };

static long waitv(struct waitv *w, unsigned n, unsigned flags, struct timespec *to, int clk)
{
	return syscall(SYS_futex_waitv, w, n, flags, to, clk);
}

static long wake(uint32_t *a, int n)
{
	return syscall(SYS_futex, a, FUTEX_WAKE_PRIVATE, n, NULL, NULL, 0);
}

static void set(struct waitv *w, uint32_t *a, uint32_t v)
{
	*w = (struct waitv){ .val = v, .uaddr = (uintptr_t)a, .flags = FUTEX2_SIZE_U32 | FUTEX2_PRIVATE };
}

static struct timespec in_ms(long ms)
{
	struct timespec t;
	clock_gettime(CLOCK_MONOTONIC, &t);
	t.tv_nsec += ms * 1000000L;
	t.tv_sec += t.tv_nsec / 1000000000L;
	t.tv_nsec %= 1000000000L;
	return t;
}

static int failures;
#define EXPECT(name, ret, err) do { long r_ = (ret); int e_ = r_ < 0 ? errno : 0; \
	if (r_ != (err ? -1 : r_) || e_ != (err)) { printf("FAIL %s: ret %ld errno %d (want errno %d)\n", name, r_, e_, err); failures++; } \
	else printf("ok   %s\n", name); } while (0)

static uint32_t f[4];

static void *waker(void *arg)
{
	usleep(20000);
	__atomic_store_n(&f[2], 1, __ATOMIC_SEQ_CST);
	wake(&f[2], 1);
	return arg;
}

/* Ping-pong: each side waits on [mine, unused] and wakes the other's. */
static uint32_t ping, pong, idle;
static int rounds = 20000;

static void *ponger(void *arg)
{
	struct waitv w[2];
	for (int i = 0; i < rounds; i++) {
		set(&w[0], &ping, 0); set(&w[1], &idle, 0);
		while (__atomic_load_n(&ping, __ATOMIC_ACQUIRE) == 0) waitv(w, 2, 0, NULL, 0);
		__atomic_store_n(&ping, 0, __ATOMIC_RELEASE);
		__atomic_store_n(&pong, 1, __ATOMIC_RELEASE);
		wake(&pong, 1);
	}
	return arg;
}

int main(void)
{
	struct waitv w[4], bad;
	struct timespec to;

	set(&w[0], &f[0], 1);
	EXPECT("value mismatch -> EAGAIN", waitv(w, 1, 0, NULL, 0), EAGAIN);
	set(&w[0], &f[0], 0);
	EXPECT("nr 0 -> EINVAL", waitv(w, 0, 0, NULL, 0), EINVAL);
	EXPECT("nr 129 -> EINVAL", waitv(w, 129, 0, NULL, 0), EINVAL);
	EXPECT("flags -> EINVAL", waitv(w, 1, 1, NULL, 0), EINVAL);
	bad = w[0]; bad.flags = 0x03;
	EXPECT("size u64 -> EINVAL", waitv(&bad, 1, 0, NULL, 0), EINVAL);
	bad = w[0]; bad.reserved = 1;
	EXPECT("reserved -> EINVAL", waitv(&bad, 1, 0, NULL, 0), EINVAL);
	bad = w[0]; bad.uaddr += 1;
	EXPECT("unaligned -> EINVAL", waitv(&bad, 1, 0, NULL, 0), EINVAL);
	bad = w[0]; bad.uaddr = 4096;
	EXPECT("bad address -> EFAULT", waitv(&bad, 1, 0, NULL, 0), EFAULT);
	to = in_ms(30);
	EXPECT("bad clock -> EINVAL", waitv(w, 1, 0, &to, CLOCK_PROCESS_CPUTIME_ID), EINVAL);
	to = in_ms(30);
	EXPECT("timeout -> ETIMEDOUT", waitv(w, 1, 0, &to, CLOCK_MONOTONIC), ETIMEDOUT);

	for (int i = 0; i < 4; i++) set(&w[i], &f[i], 0);
	pthread_t t;
	pthread_create(&t, NULL, waker, NULL);
	to = in_ms(2000);
	long r = waitv(w, 4, 0, &to, CLOCK_MONOTONIC);
	pthread_join(t, NULL);
	if (r == 2) printf("ok   wake third of four -> index 2\n");
	else { printf("FAIL wake third of four: ret %ld errno %d\n", r, errno); failures++; }

	struct timespec a, b;
	pthread_create(&t, NULL, ponger, NULL);
	clock_gettime(CLOCK_MONOTONIC, &a);
	for (int i = 0; i < rounds; i++) {
		struct waitv pw[2];
		__atomic_store_n(&ping, 1, __ATOMIC_RELEASE);
		wake(&ping, 1);
		set(&pw[0], &pong, 0); set(&pw[1], &idle, 0);
		while (__atomic_load_n(&pong, __ATOMIC_ACQUIRE) == 0) waitv(pw, 2, 0, NULL, 0);
		__atomic_store_n(&pong, 0, __ATOMIC_RELEASE);
	}
	clock_gettime(CLOCK_MONOTONIC, &b);
	pthread_join(t, NULL);
	printf("ping-pong: %.2f us/round-trip (%d rounds)\n",
	    ((b.tv_sec - a.tv_sec) * 1e9 + (b.tv_nsec - a.tv_nsec)) / 1e3 / rounds, rounds);

	printf("%s (%d failures)\n", failures ? "FAIL" : "PASS", failures);
	return failures != 0;
}
