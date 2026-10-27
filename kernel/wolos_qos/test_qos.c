/* SPDX-License-Identifier: LicenseRef-Wonderland-1.0 -- Copyright (c) 2026 Julia Klee. WolOS kernel work under the Wonderland License 1.0: see LICENSES/Wonderland-1.0.md. */
/* Usage (root): cc -o test_qos test_qos.c && ./test_qos  -> exit 0 = pass. */
#include <sys/types.h>
#include <sys/sysctl.h>
#include <sys/rtprio.h>
#include <sys/resource.h>
#include <errno.h>
#include <stdio.h>
#include <unistd.h>

int
main(void)
{
	int sc; size_t len = sizeof(sc); struct rtprio r;
	if (sysctlbyname("kern.wolos_qos.syscall", &sc, &len, NULL, 0) != 0) { perror("kldload wolos_qos"); return 1; }
	static const struct { int cls, type, prio, nice; } t[] = {
		{0, RTP_PRIO_REALTIME, 8, -99}, {1, RTP_PRIO_NORMAL, 0, -10}, {2, RTP_PRIO_NORMAL, 0, 0},
		{3, RTP_PRIO_NORMAL, 0, 10}, {4, RTP_PRIO_IDLE, 16, -99},
	};
	int fail = 0;
	for (unsigned i = 0; i < sizeof(t) / sizeof(t[0]); i++) {
		if (syscall(sc, 0, t[i].cls) != 0) { printf("cls %d: errno %d\n", t[i].cls, errno); fail = 1; continue; }
		rtprio_thread(RTP_LOOKUP, 0, &r);
		int n = getpriority(PRIO_PROCESS, 0);
		/* RTP_LOOKUP on timeshare reports the current dynamic user prio: check prio only for RT/idle. */
		int ok = r.type == t[i].type && (t[i].type == RTP_PRIO_NORMAL || r.prio == t[i].prio) && (t[i].nice == -99 || n == t[i].nice);
		printf("cls %d: type %d prio %d nice %d %s\n", t[i].cls, r.type, r.prio, n, ok ? "ok" : "FAIL");
		fail |= !ok;
	}
	errno = 0;
	if (syscall(sc, 0, 5) == 0 || errno != EINVAL) { puts("cls 5: expected EINVAL"); fail = 1; }
	return fail;
}
