// SPDX-License-Identifier: GPL-2.0-or-later
// A browser-like process tree for the limits test: PROCS processes of THREADS threads, each
// holding MB of touched memory, mostly idle. Usage: tabs PROCS THREADS MB.
#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void *idle_thread(void *a)
{
	(void)a;
	for (;;)
		usleep(200000);
	return NULL;
}

int main(int c, char **v)
{
	if (c != 4)
		return 2;
	int procs = atoi(v[1]), threads = atoi(v[2]), mb = atoi(v[3]);
	for (int i = 1; i < procs; i++)
		if (fork() == 0)
			break;
	char *m = malloc((size_t)mb << 20);
	memset(m, 1, (size_t)mb << 20);
	for (int i = 1; i < threads; i++) {
		pthread_t t;
		pthread_create(&t, NULL, idle_thread, NULL);
	}
	idle_thread(NULL);
}
