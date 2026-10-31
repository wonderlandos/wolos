// SPDX-License-Identifier: GPL-2.0-or-later
// Resource pressure for the limits test (image/vm/test/limits). Runs only in the throwaway guest
// (it needs /run/kbomb-guest, which only the guest's generator creates) and every worker stops at
// a deadline. Usage: kb KIND SECONDS; waits for a line on stdin before it starts.
#define _GNU_SOURCE
#include <fcntl.h>
#include <ftw.h>
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static double deadline;

static double now(void)
{
	struct timespec t;
	clock_gettime(CLOCK_MONOTONIC, &t);
	return t.tv_sec + t.tv_nsec / 1e9;
}

static void stop_at_deadline(void)
{
	if (now() > deadline)
		_exit(0);
}

static void hold(void)
{
	for (;;) {
		stop_at_deadline();
		sleep(1);
	}
}

// Incompressible, so zram cannot make the test easy.
static void noise(uint64_t *p, size_t bytes)
{
	static __thread uint64_t x = 88172645463325252ULL;
	for (size_t i = 0; i < bytes / 8; i++) {
		x ^= x << 13;
		x ^= x >> 7;
		x ^= x << 17;
		p[i] = x;
	}
}

static void workers(int n, void (*fn)(void))
{
	for (int i = 0; i < n; i++)
		if (fork() == 0) {
			fn();
			_exit(0);
		}
}

static void forks(void)
{
	for (;;) {
		stop_at_deadline();
		fork();
	}
}

static void *parked(void *a)
{
	(void)a;
	hold();
	return NULL;
}

static void threads(void)
{
	pthread_attr_t a;
	pthread_attr_init(&a);
	pthread_attr_setstacksize(&a, 16384);
	for (;;) {
		pthread_t t;
		stop_at_deadline();
		pthread_create(&t, &a, parked, NULL);
	}
}

static void sleepers(void)
{
	for (;;) {
		stop_at_deadline();
		pid_t p = fork();
		if (p == 0)
			hold();
		if (p < 0)
			usleep(100000);
	}
}

static void eat(void)
{
	for (;;) {
		stop_at_deadline();
		size_t n = 16 << 20;
		uint64_t *p = mmap(NULL, n, PROT_READ | PROT_WRITE,
				   MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
		if (p == MAP_FAILED)
			usleep(100000);
		else
			noise(p, n);
	}
}

// Keep eight eaters alive: killing one is not enough.
static void memory(void)
{
	workers(8, eat);
	for (;;) {
		stop_at_deadline();
		if (wait(NULL) > 0 && fork() == 0) {
			eat();
			_exit(0);
		}
	}
}

static void fds(void)
{
	struct rlimit r;
	getrlimit(RLIMIT_NOFILE, &r);
	r.rlim_cur = r.rlim_max;
	setrlimit(RLIMIT_NOFILE, &r);
	while (open("/dev/null", O_RDONLY) >= 0)
		stop_at_deadline();
	hold();
}

static int watch_one(const char *p, const struct stat *s, int t, struct FTW *f)
{
	static int fd = -1;
	(void)s; (void)t; (void)f;
	if (fd < 0)
		fd = inotify_init1(IN_NONBLOCK);
	if (fd >= 0 && inotify_add_watch(fd, p, IN_ALL_EVENTS) < 0)
		fd = -1;
	return 0;
}

static void watches(void)
{
	nftw("/usr", watch_one, 32, FTW_PHYS);
	while (inotify_init1(IN_NONBLOCK) >= 0)
		stop_at_deadline();
	hold();
}

static void pipes(void)
{
	static uint64_t buf[1 << 17];
	struct rlimit r;
	getrlimit(RLIMIT_NOFILE, &r);
	r.rlim_cur = r.rlim_max;
	setrlimit(RLIMIT_NOFILE, &r);
	int p[2];
	noise(buf, sizeof(buf));
	while (pipe2(p, O_NONBLOCK) == 0) {
		stop_at_deadline();
		fcntl(p[1], F_SETPIPE_SZ, 1 << 20);
		while (write(p[1], buf, sizeof(buf)) > 0)
			;
	}
	hold();
}

static void fill(const char *path)
{
	static uint64_t buf[1 << 17];
	int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
	noise(buf, sizeof(buf));
	while (fd >= 0 && write(fd, buf, sizeof(buf)) == sizeof(buf))
		stop_at_deadline();
}

static void tmpfs_fill(void)
{
	fill("/tmp/kb-fill");
	fill("/dev/shm/kb-fill");
	hold();
}

static void disk_fill(void)
{
	char p[256];
	snprintf(p, sizeof(p), "%s/kb-fill", getenv("HOME"));
	fill(p);
	hold();
}

int main(int c, char **v)
{
	char go;
	if (c != 3 || access("/run/kbomb-guest", F_OK))
		return 2;
	if (read(0, &go, 1) != 1)
		return 2;
	deadline = now() + atof(v[2]);
	const char *k = v[1];
	if (!strcmp(k, "fork"))
		forks();
	else if (!strcmp(k, "thread"))
		threads();
	else if (!strcmp(k, "pid"))
		workers(16, sleepers);
	else if (!strcmp(k, "mem"))
		memory();
	else if (!strcmp(k, "fd"))
		workers(16, fds);
	else if (!strcmp(k, "inotify"))
		workers(4, watches);
	else if (!strcmp(k, "pipe"))
		workers(64, pipes);
	else if (!strcmp(k, "tmpfs"))
		tmpfs_fill();
	else if (!strcmp(k, "disk"))
		disk_fill();
	else
		return 2;
	hold();
}
