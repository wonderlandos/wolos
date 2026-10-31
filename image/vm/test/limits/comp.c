// SPDX-License-Identifier: GPL-2.0-or-later
// Stand-in for the compositor in the limits test: an "input" thread stamps an event at 120 Hz, a
// "compositor" thread takes it and does 1 ms of work, as a frame would. Once a second it reports
// input latency to stdout and times spawning a program, opening a file and touching fresh memory
// (a new Terminal window, a config read, a buffer). Usage: comp TAG (one report line per second).
#define _GNU_SOURCE
#include <fcntl.h>
#include <pthread.h>
#include <spawn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/eventfd.h>
#include <sys/mman.h>
#include <sys/timerfd.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define PERIOD_NS 8333333ULL
#define MAX_SAMPLES 2048

extern char **environ;
static int efd;
static _Atomic uint64_t stamp;

static uint64_t ns(void)
{
	struct timespec t;
	clock_gettime(CLOCK_MONOTONIC, &t);
	return t.tv_sec * 1000000000ULL + t.tv_nsec;
}

static void *input(void *a)
{
	(void)a;
	int t = timerfd_create(CLOCK_MONOTONIC, 0);
	struct itimerspec s = { { 0, PERIOD_NS }, { 0, PERIOD_NS } };
	timerfd_settime(t, 0, &s, NULL);
	uint64_t n = 1, due = ns() + PERIOD_NS;
	for (;;) {
		uint64_t ticks;
		if (read(t, &ticks, 8) != 8)
			continue;
		stamp = due + (ticks - 1) * PERIOD_NS;
		due += ticks * PERIOD_NS;
		if (write(efd, &n, 8) != 8)
			continue;
	}
	return NULL;
}

static int cmp(const void *a, const void *b)
{
	uint32_t x = *(const uint32_t *)a, y = *(const uint32_t *)b;
	return x < y ? -1 : x > y;
}

static long spawn_us(void)
{
	char *argv[] = { "/usr/bin/true", NULL };
	uint64_t t = ns();
	pid_t p;
	int e = posix_spawn(&p, argv[0], NULL, NULL, argv, environ);
	if (e)
		return -e;
	int st = 1;
	waitpid(p, &st, 0);
	return st ? -1000 : (long)((ns() - t) / 1000);
}

static long open_us(void)
{
	uint64_t t = ns();
	char b[64];
	int fd = open("/etc/os-release", O_RDONLY);
	if (fd < 0)
		return -1;
	(void)!read(fd, b, sizeof(b));
	close(fd);
	return (ns() - t) / 1000;
}

static long touch_us(void)
{
	size_t n = 4 << 20;
	uint64_t t = ns();
	char *p = mmap(NULL, n, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	if (p == MAP_FAILED)
		return -1;
	memset(p, 1, n);
	munmap(p, n);
	return (ns() - t) / 1000;
}

static void *probes(void *tag)
{
	for (;;) {
		long s = spawn_us(), o = open_us(), m = touch_us();
		printf("%s P %.3f spawn=%ld open=%ld touch=%ld\n", (char *)tag, ns() / 1e9, s, o, m);
		sleep(1);
	}
	return NULL;
}

int main(int c, char **v)
{
	static uint32_t lat[MAX_SAMPLES];
	const char *tag = c > 1 ? v[1] : "C";
	pthread_t t;
	int n = 0, late = 0;
	uint64_t next = ns() + 1000000000ULL;
	setvbuf(stdout, NULL, _IOLBF, 0);
	efd = eventfd(0, 0);
	pthread_create(&t, NULL, input, NULL);
	pthread_create(&t, NULL, probes, (void *)tag);
	for (;;) {
		uint64_t e;
		if (read(efd, &e, 8) != 8)
			continue;
		uint64_t d = ns() - stamp;
		uint64_t until = ns() + 1000000;
		while (ns() < until)
			;
		if (n < MAX_SAMPLES)
			lat[n++] = d > 4000000000ULL ? 4000000000U : (uint32_t)d;
		late += d > 16666667;
		if (ns() >= next) {
			qsort(lat, n, sizeof(*lat), cmp);
			printf("%s L %.3f n=%d p50=%u p99=%u max=%u late=%d\n", tag, ns() / 1e9, n,
			       lat[n / 2] / 1000, lat[n * 99 / 100] / 1000, lat[n - 1] / 1000, late);
			n = late = 0;
			next = ns() + 1000000000ULL;
		}
	}
}
