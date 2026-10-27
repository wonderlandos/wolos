/* SPDX-License-Identifier: LicenseRef-Wonderland-1.0 -- Copyright (c) 2026 Julia Klee. WolOS kernel work under the Wonderland License 1.0: see LICENSES/Wonderland-1.0.md. */
/*
 * wolos_qos: per-thread QoS syscall.  ABI (raw values are stable):
 *   0 UserInteractive  rtprio 8      1 UserInitiated nice -10   2 Default nice 0
 *   3 Utility          nice 10       4 Background    idprio 16
 * int wolos_qos_set(lwpid_t tid /0 = self/, int class);
 * Syscall number: sysctl kern.wolos_qos.syscall (dynamic slot).
 * nice is per-process in FreeBSD: classes 1-3 set timeshare for the thread
 * and nice for its process.  Raising priority (0, 1) needs root privileges.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/lock.h>
#include <sys/mutex.h>
#include <sys/module.h>
#include <sys/sysent.h>
#include <sys/sysproto.h>
#include <sys/proc.h>
#include <sys/priv.h>
#include <sys/rtprio.h>
#include <sys/resourcevar.h>
#include <sys/sched.h>
#include <sys/sysctl.h>

/* sysproto.h #undefs its PAD macros (15.x); define local equivalents. */
#define QPAD_(t)	(sizeof(syscallarg_t) <= sizeof(t) ? 0 : sizeof(syscallarg_t) - sizeof(t))
#if BYTE_ORDER == LITTLE_ENDIAN
#define QPADL_(t)	0
#define QPADR_(t)	QPAD_(t)
#else
#define QPADL_(t)	QPAD_(t)
#define QPADR_(t)	0
#endif

struct wolos_qos_set_args {
	char tid_l_[QPADL_(lwpid_t)]; lwpid_t tid; char tid_r_[QPADR_(lwpid_t)];
	char cls_l_[QPADL_(int)]; int cls; char cls_r_[QPADR_(int)];
};

static int qos_nice[] = { 0, -10, 0, 10, 0 };

static int
wolos_qos_set(struct thread *td, void *a)
{
	struct wolos_qos_set_args *uap = a;
	struct rtprio rtp;
	struct thread *ttd;
	struct proc *p;
	int error;

	if (uap->cls < 0 || uap->cls > 4)
		return (EINVAL);
	if (uap->cls == 0 &&
	    (error = priv_check(td, PRIV_SCHED_RTPRIO)) != 0)
		return (error);
	if (uap->cls == 1 &&
	    (error = priv_check(td, PRIV_SCHED_SETPRIORITY)) != 0)
		return (error);
	switch (uap->cls) {
	case 0: rtp.type = RTP_PRIO_REALTIME; rtp.prio = 8; break;
	case 4: rtp.type = RTP_PRIO_IDLE; rtp.prio = 16; break;
	default: rtp.type = RTP_PRIO_NORMAL; rtp.prio = 0; break;
	}
	if (uap->tid == 0 || uap->tid == td->td_tid) {
		ttd = td;
		p = td->td_proc;
		PROC_LOCK(p);
	} else {
		ttd = tdfind(uap->tid, -1);
		if (ttd == NULL)
			return (ESRCH);
		p = ttd->td_proc;
		if ((error = p_cansched(td, p)) != 0) {
			PROC_UNLOCK(p);
			return (error);
		}
	}
	error = rtp_to_pri(&rtp, ttd);
	if (error == 0 && uap->cls >= 1 && uap->cls <= 3)
		sched_nice(p, qos_nice[uap->cls]);
	PROC_UNLOCK(p);
	return (error);
}

static struct sysent wolos_qos_sysent = {
	.sy_narg = 2,
	.sy_call = wolos_qos_set,
};
static int wolos_qos_offset = NO_SYSCALL;

SYSCTL_NODE(_kern, OID_AUTO, wolos_qos, CTLFLAG_RD | CTLFLAG_MPSAFE, 0, "WolOS QoS");
SYSCTL_INT(_kern_wolos_qos, OID_AUTO, syscall, CTLFLAG_RD, &wolos_qos_offset, 0,
    "wolos_qos_set syscall number");

static int
wolos_qos_load(struct module *m __unused, int what, void *arg __unused)
{
	return (what == MOD_LOAD || what == MOD_UNLOAD ? 0 : EOPNOTSUPP);
}

SYSCALL_MODULE(wolos_qos, &wolos_qos_offset, &wolos_qos_sysent, wolos_qos_load, NULL);
