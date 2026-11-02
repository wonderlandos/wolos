/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * scx_wolos: WolOS QoS classes on sched_ext (port of the FreeBSD wolos_qos policy).
 * The class is read from what userspace set with sched_setattr(2) (<linux/wolos.h>), see
 * scx_wolos_class.h: SCHED_IDLE -> background; uclamp_min >= 768 -> interactive; >= 384 ->
 * foreground (the frontmost app); uclamp_max <= 512 -> utility; else default. EEVDF reads the
 * same nice/uclamp values, so the classes keep meaning when this scheduler is not loaded.
 * Order: interactive (1 ms slices) > foreground (2 ms) > default/utility (4 ms, per-CPU deadline
 * queues, idle CPUs steal) > background (only when idle, or after bg_starve_ns). A waking task
 * preempts a CPU running a lower class. Within the default class a task that wakes with an
 * earlier deadline preempts: the deadline is its virtual time plus its usual burst, so an
 * untagged render thread beats a compiler without anyone naming it.
 * The deadline and wakeup preemption follow the design of scx_bpfland (Andrea Righi, GPL-2.0,
 * sched-ext/scx); this is a reimplementation of the idea, not its code.
 */
#include <scx/common.bpf.h>
#include "scx_wolos_class.h"

char _license[] SEC("license") = "GPL";

#define MAX_CPUS	512
#define VTIME_STEP_NS	1000000ULL

const volatile u64 bg_starve_ns = 100000000ULL;		/* background runs at least every 100 ms */
/* schedutil target per class (0..1024); HWP/EPP drivers decide on their own. */
static const u32 class_perf[C_NR] = { 1024, 1024, 768, 512, 256 };

static u64 vtime_now;
static u64 bg_last_ns;
static s32 nr_low;	/* CPUs running utility or background: when 0, no scan for one */
UEI_DEFINE(uei);

struct {
	__uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
	__uint(key_size, sizeof(u32));
	__uint(value_size, sizeof(u64));
	__uint(max_entries, C_NR);
} stats SEC(".maps");	/* enqueues per class, read by `scx_wolos --stats` */

/* What each CPU runs now, for preemption. Racy reads are fine: a wrong guess costs one kick. */
struct cpu_ctx {
	u32 cls;	/* C_NR = idle */
	u32 perf;	/* last cpuperf target */
	u64 start;
	u64 deadline;
	u64 req;
} __attribute__((aligned(64)));

struct {
	__uint(type, BPF_MAP_TYPE_ARRAY);
	__uint(key_size, sizeof(u32));
	__uint(value_size, sizeof(struct cpu_ctx));
	__uint(max_entries, MAX_CPUS);
} cpu_ctxs SEC(".maps");

struct task_ctx {
	u64 vtime;
	u64 deadline;
	u64 req;	/* what it asks for, wolos_request() */
	u64 start;	/* when it last started running */
	u64 burst;	/* run time since it last slept */
	u64 avg_burst;
	u64 slice;	/* the slice it was last given */
	u64 left;	/* preempted with this much slice left, its deadline stands */
	bool sync;	/* the waker is about to sleep */
};

struct {
	__uint(type, BPF_MAP_TYPE_TASK_STORAGE);
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, struct task_ctx);
} task_ctxs SEC(".maps");

/*
 * tgid -> highest class it may take (C_INTERACTIVE or C_FOREGROUND). Written only by the root
 * loader through the map pinned at /sys/fs/bpf/wolos/boosted (0600); key WOLOS_GRANTS_KEY
 * present means grants are in use. See wolos_class_guarded() (audit A19).
 */
struct {
	__uint(type, BPF_MAP_TYPE_HASH);
	__uint(key_size, sizeof(u32));
	__uint(value_size, sizeof(u32));
	__uint(max_entries, 4096);
} boosted SEC(".maps");

static struct task_ctx *task_ctx(struct task_struct *p);

static __always_inline int task_class(struct task_struct *p)
{
	int c = wolos_class(p->policy, p->uclamp_req[0].value, p->uclamp_req[1].value);
	u32 key = WOLOS_GRANTS_KEY, tgid = p->tgid;
	struct task_ctx *t;
	u32 *g;
	int grant = -1;

	if (c >= C_DEFAULT)
		return c;
	int on = bpf_map_lookup_elem(&boosted, &key) != NULL;
	if (on && (g = bpf_map_lookup_elem(&boosted, &tgid)))
		grant = *g;
	t = task_ctx(p);
	return wolos_class_guarded(c, grant, on, t ? t->avg_burst : 0);
}

static struct task_ctx *task_ctx(struct task_struct *p)
{
	return bpf_task_storage_get(&task_ctxs, p, 0, 0);
}

static struct cpu_ctx *cpu_ctx(s32 cpu)
{
	u32 k = cpu;

	return bpf_map_lookup_elem(&cpu_ctxs, &k);
}

static void stat_inc(u32 c)
{
	u64 *n = bpf_map_lookup_elem(&stats, &c);

	if (n)
		(*n)++;
}

/* An idle task keeps at most one slice of credit; the deadline adds its usual burst. */
static void update_deadline(struct task_struct *p, struct task_ctx *t, int c)
{
	u64 floor = vtime_now - wolos_class_slice(c);

	if (time_before(t->vtime, floor))
		t->vtime = floor;
	t->req = wolos_request(t->avg_burst, t->burst);
	t->deadline = t->vtime + scale_by_task_weight_inverse(p, t->req);
}

static bool can_take(int c, struct task_ctx *t, struct cpu_ctx *cc, u64 now, bool sync)
{
	return cc && wolos_wake_preempts(c, cc->cls, t->deadline, cc->deadline, t->req,
					 cc->req, now - cc->start, sync);
}

/*
 * The CPU a waking @p should take over, -1 for none. Its own CPU first, it is cache-hot, then
 * the others. A default-class wake that could only win over a lower class skips the walk when
 * no CPU runs one, which keeps it off the path of a pipe or a task that asks for all it can.
 */
static s32 preempt_target(struct task_struct *p, struct task_ctx *t, int c, s32 prev, bool sync)
{
	u32 n = scx_bpf_nr_cpu_ids();
	u64 now = bpf_ktime_get_ns();
	s32 i;

	if (bpf_cpumask_test_cpu(prev, p->cpus_ptr) && can_take(c, t, cpu_ctx(prev), now, sync))
		return prev;
	if (c >= C_UTILITY ||
	    (c == C_DEFAULT && !nr_low && (sync || wolos_req_outranks(t->req, WOLOS_REQ_MAX) == 0)))
		return -1;
	bpf_for(i, 1, n) {
		s32 v = (prev + i) % n;

		if (v >= MAX_CPUS || !bpf_cpumask_test_cpu(v, p->cpus_ptr))
			continue;
		if (can_take(c, t, cpu_ctx(v), now, sync))
			return v;
	}
	return -1;
}

s32 BPF_STRUCT_OPS(wolos_select_cpu, struct task_struct *p, s32 prev_cpu, u64 wake_flags)
{
	struct task_ctx *t = task_ctx(p);
	bool idle = false;
	s32 cpu = scx_bpf_select_cpu_dfl(p, prev_cpu, wake_flags, &idle);

	if (t)
		t->sync = wake_flags & SCX_WAKE_SYNC;
	if (idle) {
		int c = task_class(p);

		if (t) {
			update_deadline(p, t, c);
			t->slice = wolos_class_slice(c);
			t->left = 0;
		}
		stat_inc(c);
		scx_bpf_dsq_insert(p, SCX_DSQ_LOCAL, wolos_class_slice(c), 0);
	}
	return cpu;
}

void BPF_STRUCT_OPS(wolos_enqueue, struct task_struct *p, u64 enq_flags)
{
	struct task_ctx *t = task_ctx(p);
	int c = task_class(p);
	int dsq = wolos_class_dsq(c);
	u64 slice = wolos_class_slice(c);
	s32 cpu = scx_bpf_task_cpu(p), target = -1;
	bool sync;

	stat_inc(c);
	if (!t) {
		scx_bpf_dsq_insert(p, SCX_DSQ_LOCAL, slice, enq_flags);
		return;
	}
	/*
	 * Back from preemption mid-burst: it keeps its place, or it would queue behind a peer
	 * and pay for a whole burst of that peer on top of what preempted it.
	 */
	if (t->left && !(enq_flags & SCX_ENQ_WAKEUP))
		slice = t->left;
	else
		update_deadline(p, t, c);
	t->left = 0;
	t->slice = slice;
	sync = t->sync;
	t->sync = false;
	/*
	 * The shared queues also take a task whose slice ended: its CPU picked its next task
	 * before this enqueue ran, so without a kick the task waits for some other CPU's slice.
	 */
	if ((enq_flags & SCX_ENQ_WAKEUP) || c <= C_FOREGROUND)
		target = preempt_target(p, t, c, cpu, sync);
	/*
	 * Queue and preempt in one step. A separate kick can land while the CPU switches and be
	 * lost, and a task pinned there then waits out a whole slice.
	 */
	if (target >= 0) {
		scx_bpf_dsq_insert(p, SCX_DSQ_LOCAL_ON | target, slice,
				   enq_flags | SCX_ENQ_HEAD | SCX_ENQ_PREEMPT);
		return;
	}
	/* Normal work stays on its CPU, cache-hot, no shared lock. */
	if (dsq == DSQ_PCPU)
		dsq = DSQ_PCPU_BASE + cpu;
	scx_bpf_dsq_insert_vtime(p, dsq, slice, t->deadline, enq_flags);
	if (dsq >= DSQ_PCPU_BASE) {
		/* Every kick takes that CPU's rq lock, and a busy CPU finds the task on its own. */
		struct cpu_ctx *cc = cpu_ctx(cpu);

		if (!cc || cc->cls == C_NR)
			scx_bpf_kick_cpu(cpu, SCX_KICK_IDLE);
	}
}

void BPF_STRUCT_OPS(wolos_dispatch, s32 cpu, struct task_struct *prev)
{
	u64 now = bpf_ktime_get_ns();
	u32 n = scx_bpf_nr_cpu_ids();
	s32 i;

	if (now - bg_last_ns > bg_starve_ns && scx_bpf_dsq_move_to_local(DSQ_BG, 0)) {
		bg_last_ns = now;
		return;
	}
	if (scx_bpf_dsq_move_to_local(DSQ_INTERACTIVE, 0) ||
	    scx_bpf_dsq_move_to_local(DSQ_FG, 0) ||
	    scx_bpf_dsq_move_to_local(DSQ_PCPU_BASE + cpu, 0))
		return;
	/* Own queue empty: steal normal work from the next busy CPU. */
	bpf_for(i, 1, n) {
		s32 v = (cpu + i) % n;

		if (scx_bpf_dsq_nr_queued(DSQ_PCPU_BASE + v) &&
		    scx_bpf_dsq_move_to_local(DSQ_PCPU_BASE + v, 0))
			return;
	}
	if (scx_bpf_dsq_move_to_local(DSQ_BG, 0))
		bg_last_ns = now;
}

void BPF_STRUCT_OPS(wolos_running, struct task_struct *p)
{
	struct task_ctx *t = task_ctx(p);
	int c = task_class(p);
	s32 cpu = scx_bpf_task_cpu(p);
	struct cpu_ctx *cc = cpu_ctx(cpu);
	u64 now = bpf_ktime_get_ns();

	if (t) {
		t->start = now;
		/* Coarse on purpose: a store per context switch bounces this line across CPUs. */
		if (time_before(vtime_now + VTIME_STEP_NS, t->vtime))
			vtime_now = t->vtime;
	}
	if (cc) {
		cc->cls = c;
		cc->start = now;
		cc->deadline = t ? t->deadline : 0;
		cc->req = t ? t->req : 0;
		if (c >= C_UTILITY)
			__sync_fetch_and_add(&nr_low, 1);
	}
	if (cc && c >= 0 && c < C_NR && cc->perf != class_perf[c]) {
		cc->perf = class_perf[c];
		scx_bpf_cpuperf_set(cpu, class_perf[c]);
	}
}

void BPF_STRUCT_OPS(wolos_stopping, struct task_struct *p, bool runnable)
{
	struct task_ctx *t = task_ctx(p);
	struct cpu_ctx *cc = cpu_ctx(scx_bpf_task_cpu(p));

	if (t) {
		/* Real run time: a preempted task's slice reads 0, which would charge it all. */
		u64 used = bpf_ktime_get_ns() - t->start;
		u64 delta = scale_by_task_weight_inverse(p, used);

		if (task_class(p) == C_UTILITY)
			delta *= 2;
		t->vtime += delta;
		t->burst += used;
		t->left = runnable ? wolos_slice_left(t->slice, used) : 0;
		if (!runnable) {
			t->avg_burst = wolos_avg_burst(t->avg_burst, t->burst);
			t->burst = 0;
		}
	}
	if (cc) {
		if (cc->cls >= C_UTILITY && cc->cls < C_NR)
			__sync_fetch_and_sub(&nr_low, 1);
		cc->cls = C_NR;
	}
}

s32 BPF_STRUCT_OPS_SLEEPABLE(wolos_init_task, struct task_struct *p, struct scx_init_task_args *args)
{
	struct task_ctx *t = bpf_task_storage_get(&task_ctxs, p, 0, BPF_LOCAL_STORAGE_GET_F_CREATE);

	if (!t)
		return -ENOMEM;
	/* A new task has not shown it is short: it asks for a slice until it has slept. */
	t->avg_burst = WOLOS_SLICE_NORMAL;
	return 0;
}

void BPF_STRUCT_OPS(wolos_enable, struct task_struct *p)
{
	struct task_ctx *t = task_ctx(p);

	if (t)
		t->vtime = vtime_now;
}

s32 BPF_STRUCT_OPS_SLEEPABLE(wolos_init)
{
	u32 n = scx_bpf_nr_cpu_ids();
	s32 i;
	int ret;

	ret = scx_bpf_create_dsq(DSQ_INTERACTIVE, -1);
	if (!ret)
		ret = scx_bpf_create_dsq(DSQ_FG, -1);
	if (!ret)
		ret = scx_bpf_create_dsq(DSQ_BG, -1);
	bpf_for(i, 0, n) {
		struct cpu_ctx *cc;

		if (ret)
			break;
		ret = scx_bpf_create_dsq(DSQ_PCPU_BASE + i, -1);
		cc = cpu_ctx(i);
		if (cc)
			cc->cls = C_NR;
	}
	if (ret)
		scx_bpf_error("create DSQ failed (%d)", ret);
	return ret;
}

void BPF_STRUCT_OPS(wolos_exit, struct scx_exit_info *ei)
{
	UEI_RECORD(uei, ei);
}

SCX_OPS_DEFINE(wolos_ops,
	       .select_cpu	= (void *)wolos_select_cpu,
	       .enqueue		= (void *)wolos_enqueue,
	       .dispatch	= (void *)wolos_dispatch,
	       .running		= (void *)wolos_running,
	       .stopping	= (void *)wolos_stopping,
	       .init_task	= (void *)wolos_init_task,
	       .enable		= (void *)wolos_enable,
	       .init		= (void *)wolos_init,
	       .exit		= (void *)wolos_exit,
	       .timeout_ms	= 5000,
	       .name		= "wolos");
